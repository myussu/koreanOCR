"""Measure a local speech server while it speaks, and build the page that puts several voices side by side.

`maru tts-eval` compares the configured voice with other servers, but it starts the configured server itself and
measures no memory. To try another speech *model* in the installed `tts-server` without touching the settings, this
starts the server on a spare port, times it, watches its memory while a command speaks through it, and stops it.
Windows only (it reads the process and GPU counters Task Manager shows).

  python tools/voice_candidates.py measure --server tts-server.exe --model talker.gguf --codec codec.gguf
         --port 18111 --out DIR [--interval 1.0] [--wait-minutes 20] -- COMMAND …
  python tools/voice_candidates.py page spec.json

`measure` first waits (asking once a minute, at most `--wait-minutes`) for other model servers to go away
(`llama-server.exe`, `tts-server.exe`, `maru-app.exe`; it never ends them), starts the server the way Maru does
(`--alias qwen3-tts --host 127.0.0.1 --lang Korean`, the device left to the server), and counts the seconds until
`/health` answers. Then it runs COMMAND (`{url}` in it becomes the server's address), for example
`maru --home EMPTY tts-eval --endpoint {url} --voice sohee --language Korean --out DIR/audio`, and every `--interval`
seconds notes the server's working set, its private memory and the dedicated GPU memory Windows counts for that process
(`\\GPU Process Memory(pid_N_*)\\Dedicated Usage`), and how much GPU memory everything else is using, so that another
program loading a model in the middle shows. It writes `DIR/measurement.json` and `DIR/server.log`, stops the server,
and says whether the port is free and the GPU memory was given back.

`page` reads a description of the voices (which folder holds each one's WAV files, and either its `measurement.json`
files or the numbers somebody reported earlier) and writes one page: every sentence with a player per voice, and the
table of measurements. It checks each WAV on the way and fails if one is missing, empty or silent, and says under
each player how long the audio is and how much of that is quiet (below -45 dBFS, in 20 ms steps).

The numbers say how heavy and how fast a voice is. How it sounds is for a person to judge by listening.
"""
import argparse
import array
import csv
import datetime
import html
import io
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
import urllib.request
import wave

OTHER_SERVERS = ("llama-server.exe", "tts-server.exe", "maru-app.exe")
MB = 1_048_576
# Audio counts as quiet where a 20 ms stretch stays under -45 dB of full scale.
QUIET_STEP, QUIET_LEVEL = 0.02, 32768 * 10 ** (-45 / 20)
# What `maru tts-eval` prints for each sentence: voice, sentence, audio seconds, seconds it took, their ratio.
EVAL_ROW = re.compile(r"^(.*?)\s+(\S+)\s+([\d.]+)s\s+([\d.]+)s\s+([\d.]+)\s*$")
# Maru's warning when a take lasts far longer than its text could need (it then asks for another).
RETAKE = re.compile(r"\btake=(\d+).*?\bseconds=([\d.]+).*?\blimit=([\d.]+)")
ANSI = re.compile(r"\x1b\[[0-9;]*m")


# ---- pure parts ---------------------------------------------------------------------------------------------------

def model_servers(tasklist_csv):
    """The model servers named in `tasklist /FO CSV /NH` output, as (name, process id)."""
    found = []
    for row in csv.reader(io.StringIO(tasklist_csv)):
        if len(row) >= 2 and row[0].lower() in OTHER_SERVERS and row[1].isdigit():
            found.append((row[0], int(row[1])))
    return found


def wait_until_quiet(running, minutes, sleep=time.sleep):
    """Ask `running()` once a minute until it names nothing or `minutes` have passed: (what is still there, minutes waited)."""
    waited, busy = 0, running()
    while busy and waited < minutes:
        sleep(60)
        waited += 1
        busy = running()
    return busy, waited


def process_gpu_bytes(instances, pid):
    """Dedicated GPU memory of one process: Windows keeps one counter per process and adapter (`pid_N_luid_…`)."""
    prefix = f"pid_{pid}_"
    return sum(value for name, value in instances.items() if name.lower().startswith(prefix))


def peaks(samples):
    """The highest value each counter reached over the samples, in MB (1,048,576 bytes), rounded."""
    return {key: round(max((s[key] for s in samples), default=0) / MB) for key in ("working_set", "private", "gpu_process")}


def others_gpu_mb(samples):
    """The least and the most GPU memory every other program together held during the samples, in MB.

    A model server that starts or stops in the middle moves this by gigabytes; a desktop moves it by a few MB.
    """
    values = [round(s["gpu_others"] / MB) for s in samples if "gpu_others" in s]
    return [min(values), max(values)] if values else None


def eval_rows(text):
    """`maru tts-eval`'s table as rows: sentence name, seconds of audio, seconds it took, and their ratio."""
    rows = []
    for line in text.splitlines():
        m = EVAL_ROW.match(line)
        if m:
            rows.append({"voice": m.group(1), "sentence": m.group(2), "audio": float(m.group(3)), "took": float(m.group(4)), "rtf": float(m.group(5))})
    return rows


def retakes(log):
    """The takes Maru found too long for their text and asked for again: take number, its seconds, the limit."""
    found = []
    for line in ANSI.sub("", log).splitlines():
        m = RETAKE.search(line)
        if m and "asking again" in line:
            found.append({"take": int(m.group(1)), "seconds": float(m.group(2)), "limit": float(m.group(3))})
    return found


def speed_range(rows):
    """The fastest and slowest sentence as fractions of real time, written `0.10–0.26`."""
    values = [r["rtf"] for r in rows]
    return f"{min(values):.2f}–{max(values):.2f}" if values else "—"


def plausible_seconds(text):
    """The shortest and longest a reading of `text` may plausibly last.

    Maru's own limit for one sentence is a second plus 0.45 s for every Hangul syllable or other word; here every
    digit counts too (a phone number is read digit by digit), and the pause Maru puts between sentences is added.
    Under a tenth of a second a syllable means the audio stops early.
    """
    syllables = sum(1 for c in text if "가" <= c <= "힣")
    digits = sum(1 for c in text if c.isdigit())
    words = len(re.findall(r"[A-Za-z]+", text))
    sentences = max(1, len(re.findall(r"[.!?…]+(?=\s|$)", text)))
    longest = sentences * 1.0 + 0.45 * (syllables + digits + words) + 0.35 * (sentences - 1)
    return 0.1 * (syllables + digits), longest


def quiet_seconds(samples, rate):
    """How much of 16-bit audio is quiet: in all, before the first sound, and after the last."""
    step = max(1, int(rate * QUIET_STEP))
    quiet = [sum(x * x for x in samples[i:i + step]) / step < QUIET_LEVEL ** 2 for i in range(0, len(samples) - step + 1, step)]
    before = next((i for i, q in enumerate(quiet) if not q), len(quiet))
    after = next((i for i, q in enumerate(reversed(quiet)) if not q), len(quiet))
    return {"quiet": sum(quiet) * step / rate, "quiet_before": before * step / rate, "quiet_after": after * step / rate}


def wav_facts(path):
    """What a WAV file holds: its length, format, its loudest sample as a fraction of full scale, and how much is quiet."""
    with wave.open(path, "rb") as w:
        channels, width, rate, frames = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
        data = w.readframes(frames)
    facts = {"seconds": frames / rate if rate else 0.0, "rate": rate, "channels": channels, "bits": width * 8, "loudest": None, "bytes": os.path.getsize(path)}
    if width == 2 and rate:
        samples = array.array("h")
        samples.frombytes(data[: len(data) // 2 * 2])
        if sys.byteorder == "big":
            samples.byteswap()
        facts["loudest"] = max(max(samples, default=0), -min(samples, default=0)) / 32768
        # Stereo would be measured as one stream; the voices here are mono.
        facts.update(quiet_seconds(samples, rate * channels))
    return facts


def wav_problem(facts, text):
    """Why this audio cannot be a reading of `text`, or `None` when nothing measurable is wrong with it."""
    shortest, longest = plausible_seconds(text)
    if facts["seconds"] <= 0:
        return "no audio in the file"
    if facts["loudest"] is not None and facts["loudest"] < 0.01:
        return "silent"
    if facts["seconds"] < shortest:
        return f"{facts['seconds']:.1f} s is too short for this text (under {shortest:.1f} s)"
    if facts["seconds"] > longest:
        return f"{facts['seconds']:.1f} s is longer than this text could need ({longest:.1f} s)"
    return None


def thousands(n):
    return f"{n:,}"


def read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def measured_row(runs):
    """The table cells for a voice measured in one or more runs: the worst of the runs for memory, every run for time."""
    rows = [r for run in runs for r in eval_rows(run["client"]["stdout"])]
    again = [t for run in runs for t in retakes(run["client"]["stderr"])]
    takes = "none"
    if again:
        kept = sum(1 for t in again if t["take"] >= 3)
        takes = f"{len(again)} asked again" + (f", {kept} kept although too long" if kept else "")

    def both(key):
        # What the samples caught, and the higher instant the system itself recorded between two samples.
        sampled = f"{thousands(max(run['peaks_mb'][key] for run in runs))} MB"
        system = max(run.get("system_peaks_mb", {}).get(key, 0) for run in runs)
        return f"{sampled} sampled, {thousands(system)} MB at its highest instant" if system else sampled

    def whole(run):
        # The speed alone flatters a voice that pads its audio: this is the time all the sentences took to make.
        made = eval_rows(run["client"]["stdout"])
        return f"{sum(r['took'] for r in made):.1f} s for {sum(r['audio'] for r in made):.1f} s of audio"

    return {
        "ready": " and ".join(f"{run['ready_seconds']:.1f} s" for run in runs),
        "whole": " and ".join(whole(run) for run in runs),
        "working_set": both("working_set"),
        "private": both("private"),
        "gpu": f"{thousands(max(run['peaks_mb']['gpu_process'] for run in runs))} MB",
        "speed": f"{speed_range(rows)} × real time",
        "takes": takes,
    }


PAGE_STYLE = """
:root{--bg:#fff;--fg:#1c1c1c;--dim:#6b6b6b;--line:#d9d9d9;--card:#f6f6f4}
@media (prefers-color-scheme:dark){:root{--bg:#161616;--fg:#e8e8e8;--dim:#9a9a9a;--line:#3a3a3a;--card:#202020}}
body{font:16px system-ui,sans-serif;max-width:1100px;margin:2em auto;padding:0 16px;background:var(--bg);color:var(--fg)}
h1{font-size:26px;margin-bottom:.2em}h2{margin-top:2.2em;font-size:19px}
p,li{line-height:1.5}.dim{color:var(--dim);font-size:13px}
.wrap{overflow-x:auto}table{border-collapse:collapse;width:100%;min-width:820px;font-size:14px}
th,td{text-align:left;padding:8px 10px;border-bottom:1px solid var(--line);vertical-align:top}th{font-weight:600}
.s{margin:1.4em 0;padding-bottom:1.2em;border-bottom:1px solid var(--line)}.k{font-size:20px;line-height:1.5}
.g{display:grid;grid-template-columns:repeat(auto-fit,minmax(250px,1fr));gap:12px;margin-top:.7em}
.v{background:var(--card);border-radius:8px;padding:10px 12px}.n{font-weight:600;font-size:14px}
audio{width:100%;margin:.5em 0 .2em}.bad{color:#c0392b}
"""


def render_page(spec, base):
    """The page for `spec`, and every problem found in its audio. `base` is the folder the page will be in."""
    e = html.escape
    problems = []
    cells = ("ready", "working_set", "private", "gpu", "speed", "whole", "takes")
    out = [f"<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>{e(spec['title'])}</title><style>{PAGE_STYLE}</style>"]
    out.append(f"<h1>{e(spec['title'])}</h1>")
    out.extend(f"<p>{e(p)}</p>" for p in spec.get("intro", []))
    out.append("<h2>Measurements</h2><div class=wrap><table><tr><th>Voice</th><th>Ready in</th><th>Peak working set</th><th>Peak private memory</th><th>Peak dedicated GPU memory</th><th>Speed (time to make ÷ length)</th><th>All the sentences</th><th>Takes that ran long</th></tr>")
    made = {}
    for voice in spec["voices"]:
        runs = [read_json(os.path.join(base, p)) for p in voice.get("measurements", [])]
        row = measured_row(runs) if runs else voice["reported"]
        # The audio on the page is one run's (the last unless told otherwise); its own times go under the players.
        made[voice["name"]] = dict(voice.get("made", {}))
        if runs:
            for r in eval_rows(runs[voice.get("audio_from", len(runs)) - 1]["client"]["stdout"]):
                made[voice["name"]].setdefault(r["sentence"], r)
        out.append(f"<tr><td><b>{e(voice['name'])}</b><div class=dim>{e(voice.get('about', ''))}</div></td>" + "".join(f"<td>{e(row.get(c, '—'))}</td>" for c in cells) + "</tr>")
    out.append("</table></div>")
    out.append("<h2>Listen</h2>")
    for sentence in spec["sentences"]:
        out.append(f"<div class=s><div class=k>{e(sentence['text'])}</div><div class=dim>{e(sentence['name'])}</div><div class=g>")
        for voice in spec["voices"]:
            src = f"{voice['folder']}/{sentence['name']}.wav"
            notes, bad = [], None
            try:
                facts = wav_facts(os.path.join(base, src))
                bad = wav_problem(facts, sentence["text"])
                notes.append(f"{facts['seconds']:.1f} s" + (f", {facts['quiet']:.1f} s of it quiet" if "quiet" in facts else ""))
            except FileNotFoundError:
                bad = "the file is missing"
            except (OSError, EOFError, wave.Error) as error:
                bad = f"not a WAV file that can be read ({error})"
            how = made[voice["name"]].get(sentence["name"])
            if how:
                notes.append(f"made in {how['took']:.1f} s" + (f" ({how['rtf']:.2f} × real time)" if "rtf" in how else ""))
            if bad:
                problems.append(f"{src}: {bad}")
                notes.append(f"<span class=bad>{e(bad)}</span>")
            out.append(f"<div class=v><div class=n>{e(voice['name'])}</div><audio controls preload=metadata src=\"{e(src)}\"></audio><div class=dim>{' · '.join(notes)}</div></div>")
        out.append("</div></div>")
    # How it was measured goes last: the page is for listening.
    if spec.get("method"):
        out.append("<h2>How this was measured</h2>")
        out.extend(f"<p class=dim>{e(p)}</p>" for p in spec["method"])
    out.extend(f"<p class=dim>{e(p)}</p>" for p in spec.get("footer", []))
    return "\n".join(out) + "\n", problems


# ---- Windows counters -----------------------------------------------------------------------------------------------

def running_model_servers():
    listing = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, errors="replace").stdout
    return model_servers(listing)


def process_memory(handle):
    """Working set and private bytes of a process now, and the highest the system saw them (`GetProcessMemoryInfo`)."""
    import ctypes
    from ctypes import wintypes

    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
            (name, ctypes.c_size_t)
            for name in ("PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
                         "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage")
        ]

    counters = Counters()
    counters.cb = ctypes.sizeof(Counters)
    get = ctypes.WinDLL("kernel32").K32GetProcessMemoryInfo
    get.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    if not get(int(handle), ctypes.byref(counters), counters.cb):
        return None
    return {"working_set": counters.WorkingSetSize, "private": counters.PrivateUsage, "peak_working_set": counters.PeakWorkingSetSize, "peak_private": counters.PeakPagefileUsage}


def counter_instances(path):
    """Every instance of a Windows performance counter and its value now, e.g. `\\GPU Process Memory(*)\\Dedicated Usage`."""
    import ctypes
    from ctypes import wintypes

    class Value(ctypes.Structure):
        _fields_ = [("status", wintypes.DWORD), ("large", ctypes.c_longlong)]

    class Item(ctypes.Structure):
        _fields_ = [("name", ctypes.c_wchar_p), ("value", Value)]

    pdh = ctypes.WinDLL("pdh")
    pdh.PdhOpenQueryW.argtypes = [wintypes.LPCWSTR, ctypes.c_size_t, ctypes.POINTER(wintypes.HANDLE)]
    pdh.PdhAddEnglishCounterW.argtypes = [wintypes.HANDLE, wintypes.LPCWSTR, ctypes.c_size_t, ctypes.POINTER(wintypes.HANDLE)]
    pdh.PdhCollectQueryData.argtypes = [wintypes.HANDLE]
    pdh.PdhGetFormattedCounterArrayW.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
    pdh.PdhCloseQuery.argtypes = [wintypes.HANDLE]
    for f in (pdh.PdhOpenQueryW, pdh.PdhAddEnglishCounterW, pdh.PdhCollectQueryData, pdh.PdhGetFormattedCounterArrayW, pdh.PdhCloseQuery):
        f.restype = wintypes.DWORD
    more_data, as_large = 0x800007D2, 0x00000400
    query, counter = wintypes.HANDLE(), wintypes.HANDLE()
    if pdh.PdhOpenQueryW(None, 0, ctypes.byref(query)):
        return {}
    try:
        # A new query each time: a process that has just started using the GPU has a new instance.
        if pdh.PdhAddEnglishCounterW(query, path, 0, ctypes.byref(counter)) or pdh.PdhCollectQueryData(query):
            return {}
        size, count = wintypes.DWORD(0), wintypes.DWORD(0)
        if pdh.PdhGetFormattedCounterArrayW(counter, as_large, ctypes.byref(size), ctypes.byref(count), None) != more_data:
            return {}
        buffer = ctypes.create_string_buffer(size.value)
        if pdh.PdhGetFormattedCounterArrayW(counter, as_large, ctypes.byref(size), ctypes.byref(count), buffer):
            return {}
        items = ctypes.cast(buffer, ctypes.POINTER(Item))
        return {items[i].name: items[i].value.large for i in range(count.value)}
    finally:
        pdh.PdhCloseQuery(query)


def adapters_mb():
    """Dedicated memory in use on each GPU, by every program together, in MB."""
    return {name: round(value / MB) for name, value in counter_instances("\\GPU Adapter Memory(*)\\Dedicated Usage").items()}


def port_is_free(port):
    with socket.socket() as s:
        try:
            s.bind(("127.0.0.1", port))
            return True
        except OSError:
            return False


def healthy(port):
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=0.5) as reply:
            return reply.status == 200
    except OSError:
        return False


# ---- commands -------------------------------------------------------------------------------------------------------

def measure(args):
    if not args.command:
        sys.exit("give the command that speaks through the server after `--`")
    os.makedirs(args.out, exist_ok=True)
    busy, waited = wait_until_quiet(running_model_servers, args.wait_minutes)
    if busy:
        print(f"still running after {waited} minutes, measuring anyway: {busy}", flush=True)
    if not port_is_free(args.port):
        sys.exit(f"port {args.port} is in use; choose another")
    before = adapters_mb()
    command = [args.server, "--model", args.model, "--codec", args.codec, "--alias", "qwen3-tts", "--host", "127.0.0.1", "--port", str(args.port), "--lang", "Korean"]
    samples, stop = [], threading.Event()
    with open(os.path.join(args.out, "server.log"), "wb") as log:
        started = time.perf_counter()
        server = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)

        def watch():
            while not stop.is_set():
                tick = time.perf_counter()
                memory = process_memory(server._handle)
                if memory:
                    gpu = process_gpu_bytes(counter_instances("\\GPU Process Memory(*)\\Dedicated Usage"), server.pid)
                    everyone = sum(counter_instances("\\GPU Adapter Memory(*)\\Dedicated Usage").values())
                    samples.append({"t": round(tick - started, 2), "working_set": memory["working_set"], "private": memory["private"], "gpu_process": gpu, "gpu_others": everyone - gpu})
                stop.wait(max(0.0, args.interval - (time.perf_counter() - tick)))

        watcher = threading.Thread(target=watch, daemon=True)
        watcher.start()
        try:
            while not healthy(args.port):
                if server.poll() is not None:
                    sys.exit(f"the server ended while loading (exit code {server.returncode}); see {log.name}")
                if time.perf_counter() - started > args.ready_timeout:
                    sys.exit(f"the server was not ready after {args.ready_timeout} s; see {log.name}")
                time.sleep(0.05)
            ready = time.perf_counter() - started
            print(f"ready in {ready:.1f} s (process {server.pid})", flush=True)
            url = f"http://127.0.0.1:{args.port}"
            spoke = time.perf_counter()
            client = subprocess.run([part.replace("{url}", url) for part in args.command], capture_output=True, stdin=subprocess.DEVNULL)
            spoke = time.perf_counter() - spoke
            # Two more looks after the last sentence, then the highest values the system itself recorded.
            time.sleep(2 * args.interval)
            last = process_memory(server._handle) or {}
        finally:
            stop.set()
            watcher.join()
            server.terminate()
            server.wait(timeout=30)
    released = None
    for second in range(15):
        if port_is_free(args.port) and not process_gpu_bytes(counter_instances("\\GPU Process Memory(*)\\Dedicated Usage"), server.pid):
            released = second
            break
        time.sleep(1)
    text = lambda b: b.decode("utf-8", errors="replace")
    result = {
        "started": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
        "server_command": command,
        "port": args.port,
        "interval_seconds": args.interval,
        "other_servers_still_running": busy,
        "minutes_waited_for_them": waited,
        "other_servers_at_the_end": running_model_servers(),
        "gpu_others_mb": others_gpu_mb(samples),
        "ready_seconds": round(ready, 2),
        "samples": samples,
        "peaks_mb": peaks(samples),
        "system_peaks_mb": {"working_set": round(last.get("peak_working_set", 0) / MB), "private": round(last.get("peak_private", 0) / MB)},
        "gpu_adapters_mb": {"before": before, "after": adapters_mb()},
        "client": {"command": args.command, "seconds": round(spoke, 2), "exit_code": client.returncode, "stdout": text(client.stdout), "stderr": text(client.stderr)},
        "after": {"port_free": port_is_free(args.port), "seconds_until_gpu_memory_was_released": released},
    }
    with open(os.path.join(args.out, "measurement.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(result, f, ensure_ascii=False, indent=1)
    rows = eval_rows(result["client"]["stdout"])
    print(f"peaks {result['peaks_mb']} MB · system peaks {result['system_peaks_mb']} MB · {len(samples)} samples · speed {speed_range(rows)} over {len(rows)} sentences"
          f" · {len(retakes(result['client']['stderr']))} takes asked again · other programs' GPU memory {result['gpu_others_mb']} MB · other servers at the end {result['other_servers_at_the_end']} · command exit {client.returncode} · port free {result['after']['port_free']} · GPU memory released after {released} s")
    if client.returncode:
        sys.exit(result["client"]["stderr"][-2000:])


def page(args):
    spec = read_json(args.spec)
    base = os.path.dirname(os.path.abspath(args.spec))
    text, problems = render_page(spec, base)
    out = os.path.join(base, spec.get("page", "index.html"))
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {out}: {len(spec['sentences'])} sentences × {len(spec['voices'])} voices")
    if problems:
        sys.exit("audio that needs a look:\n  " + "\n  ".join(problems))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="do", required=True)
    m = commands.add_parser("measure")
    m.add_argument("--server", required=True, help="tts-server.exe")
    m.add_argument("--model", required=True, help="the talker GGUF")
    m.add_argument("--codec", required=True, help="the codec GGUF")
    m.add_argument("--port", type=int, required=True, help="a spare port, not the one in your settings")
    m.add_argument("--out", required=True, help="folder for measurement.json and server.log")
    m.add_argument("--interval", type=float, default=1.0, help="seconds between looks at the memory counters")
    m.add_argument("--wait-minutes", type=int, default=20, help="how long to wait for other model servers to go away")
    m.add_argument("--ready-timeout", type=float, default=120.0)
    m.add_argument("command", nargs=argparse.REMAINDER, help="after `--`: the command that speaks; {url} is the server")
    p = commands.add_parser("page")
    p.add_argument("spec", help="the JSON that describes the voices; the page is written beside it")
    args = parser.parse_args()
    if args.do == "measure":
        args.command = args.command[1:] if args.command[:1] == ["--"] else args.command
        measure(args)
    else:
        page(args)


if __name__ == "__main__":
    main()
