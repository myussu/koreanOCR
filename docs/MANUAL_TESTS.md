# What only a real PC can verify

Everything in `cargo test` and `node --test` runs without Windows, a GPU, a game or Anki. The items below cannot, so they are
checked by hand. Each has an id (`M-n`) that [ARCHITECTURE.md](ARCHITECTURE.md) and the README point to.

**How to use this file**
1. Do the setup, then the tests you care about. Tests are independent unless noted.
2. For anything that fails, bring the **exact text** of the message, or a screenshot, plus the items under *Bring*.
3. Record the outcome in the [results log](#results-log) at the bottom, so the next person (or agent) knows what is still unproven.
   Nothing here counts as verified until it is in that log.

**Where the files are:** `maru config path` prints the config file. Maru's data folder (it holds `labels.db`, `vocab.db`,
`maru-app.log`) is printed by `maru doctor` as `data:`.

## Setup, on each PC

```powershell
git pull
cargo install --path crates/maru-cli      # the `maru` command
cd app\src-tauri; cargo run --release     # the overlay (quit the running one first)
```
If the build ends with `unresolved external symbol __std_…`, update Visual Studio's C++ build tools (see the README).

---

## M-1 Setup wizard: hardware detection and recommendation
**Needs:** any Windows PC. Best on each different PC you own (NVIDIA, AMD/Intel, laptop with integrated graphics).
**Steps:** `maru setup wizard --dry-run` (changes nothing).
**Expect:** your graphics card(s) with the right name and (NVIDIA) memory, the NVIDIA driver and its CUDA version, memory and free disk
sensible, a recommendation that matches the hardware (CUDA + full model for a 10 GB+ NVIDIA card, Vulkan for AMD/Intel, CPU for
integrated or none), no card listed that is not there.
**Bring:** the whole output; the real specs of the PC if anything looks wrong (card, VRAM, RAM, drive).
**Why unverified:** the PowerShell/CIM probe and Windows' adapter names were only tested against sample text.

## M-2 Install and build
**Steps:** run the setup above; then `maru setup` (answer the questions) or `maru setup wizard --yes`; then `maru doctor --deep`.
Also check: running `maru setup` a second time skips what is installed; ending the input (Ctrl+D) at the last question installs nothing;
`maru setup wizard --backend vulkan --force` replaces a CUDA build (quit Maru first).
**Expect:** every step finishes or fails with a clear message and a way to re-run; `doctor --deep` translates a sentence.
**Bring:** the output of any failing step; `maru doctor` output.
**Why unverified:** downloads (llama.cpp, Kiwi, OCR models, dictionary) could not be reached from the build sandbox; the Windows link
step depends on the C++ toolset.

## M-3 Live subtitles
**Needs:** a game or video with Korean subtitles, borderless/windowed (an exclusive-fullscreen game cannot be drawn over).
**Steps:** `Ctrl+Alt+Shift+C`, drag a tight box around the subtitle strip; play a cutscene; then open History (`Ctrl+Alt+L`) → **Live subtitles**.
Press `Ctrl+Alt+C` to stop and start. Try a subtitle that fades, one that appears over a moving scene, and one that is two lines.
**Expect:** the English appears just above or below the original (the side with more room) soon after it appears and never covers it; it stays while the original is on screen
and about `linger_ms` after; every line is in History → Live subtitles, in order, with timings; the band never gets read back as input.
**Bring:** the "N s to the first word · N s in all" numbers for 5–10 lines; any `live line:` lines from `maru-app.log`; which side the band sits on and whether it ever touches the original, whether it
sits in the right place, wraps badly, lingers too long or too briefly; the game and its resolution.
Optional: a 15–30 s clip (under ~50 MB) pushed to a `live-clip` branch under `clips/`, or 4–5 screenshots of one subtitle line (appearing, mid-fade, gone).
**Replay tool:** a recorded clip can be replayed through the real watcher without the game: `MARU_CLIP=clip.mp4 MARU_CLIP_TRUTH=truth.txt
cargo test -p maru-engine --release --test replay -- --ignored --nocapture` (needs `ffmpeg`; `truth.txt` has one `onset off` pair, in seconds,
per subtitle; see the file header). It checks the watcher's timing, not recognition quality or the model's speed.
**Why unverified:** needs real OCR, a real model and a real screen. Latency numbers can only come from your hardware.

## M-11 Live subtitles: the model on this PC against the model on another PC
**Why:** the model, the reading model, the speech server and the game all want the same GPU; a few lines took 1–1.8 s to the first word when most
took 0.4–0.5 s. This test says how much of that is the GPU being shared, and what a second PC buys.
**Needs:** a second PC on the same network with a GPU, serving the same model (README → "A model on another PC"), and the recorded cutscene (M-3).
**Part 1: the model alone, no screen, no OCR (about a minute each).** Quit the Maru overlay first (it holds the local model's port and GPU memory).
| Run | Setup | Command |
|---|---|---|
| 1 | game closed, local model | `maru bench --live -n 3` |
| 2 | game running (a scene on screen), local model | `maru bench --live -n 3` |
| 3 | game running, model on the other PC | `maru bench --live --endpoint http://<that-pc>:8080 -n 3` |
Run 1 against 2 is what sharing the GPU with the game costs; 2 against 3 is what the second PC buys. Run 3 also with the game closed if you can.
**With Ollama on the other PC** (for example TrueNAS SCALE) instead of llama-server: pull the same model (`ollama pull hf.co/unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL`, if
that Ollama is new enough for it), check `ollama list` for the name and `ollama ps` for `100% GPU` while it is loaded, set `OLLAMA_KEEP_ALIVE=-1` (and
`OLLAMA_NUM_PARALLEL=2`) on the server, and use `maru bench --live --endpoint http://<that-pc>:<port> --model "<name from ollama list>" -n 3`. For the cutscene runs set
the same endpoint and `[llm] model_name` (Settings → Translation model). Maru asks such servers not to think (`[llm] reasoning_effort = "none"`; a person testing live subtitles reported that without it a thinking model used the whole
token limit on reasoning and returned nothing: 4.95 s and no translation, against 1.15 s and a correct one with thinking off). If you still see reasoning text, or the first-word
times are far higher than llama-server's, say so, with the model name.
**Part 2: the whole thing on the cutscene (as in M-3).** Run it twice and keep every other setting equal:
* **A, everything local** as it is now (model, reading model and speech server on your GPU).
* **C, model on the other PC** with `[ocr] vision_reader = false` and `[tts] enabled = false` (so nothing else of ours is on the GPU; the reading model is a separate model
  that every live read uses once it has loaded).
Optionally **B**: local model with the same two switched off, which separates "our other servers" from "the model itself".
In another terminal during each cutscene run: `nvidia-smi --query-gpu=timestamp,memory.used,memory.total,utilization.gpu --format=csv -l 1 > gpu-A.csv` (stop with Ctrl+C afterwards).
**Expect:** local model idle ≈ the first-word times of the typical lines; with the game running slower and more variable; the second PC's numbers close to idle plus a
few milliseconds of network, as long as its GPU is no slower.
**Bring, for each run/condition:** the `maru bench` output; for Part 2 the History → Live subtitles text and the `live line:` log lines, and the `gpu-*.csv`; the GPU of each PC.
**Why unverified:** needs your two PCs; the benchmark and the external-endpoint path are only tested against fake servers.

## M-4 Screen scan and manual join
**Steps:** `Ctrl+Alt+S` on a game menu, an inventory grid and a HUD. Click a label (its breakdown opens), hover for the ×, right-click to remove one.
Then find a phrase that got two labels: **Shift+click** both, press **Join**; scan again (`Ctrl+Alt+S`) and check it comes back joined without
you; pick the joined label (Shift+click) and press **Split**.
**Expect:** labels do not cover text or each other; dictionary meanings appear first and model labels fill in; a second scan of the same
screen is instant; the joined label is read in order with sensible spacing, translated as a whole, remembered for that game only, and Split
undoes and forgets it; the overlay never blocks the game except on a label or the Join chip.
**Bring:** screenshots of anything wrong (before joining for join problems); the game's `.exe` name; which join came back and which did not.
**Why unverified:** needs a real game, Korean OCR and the real window system (click-through hit-testing).

## M-5 Selection and capture across programs
**Steps:** select Korean text in a browser, a PDF reader, a chat app and a game; press `Ctrl+Alt+T`; also drag a region with `Ctrl+Alt+R`
over text that cannot be selected.
**Expect:** exact text when the program supports UI Automation; the Ctrl+C fallback otherwise (your clipboard is restored); OCR for regions.
**Bring:** which programs worked and which did not; whether the text came out exact (selection/copy) or looked misread (OCR); screenshots of misreads.
**Why unverified:** UI Automation, the clipboard route, GDI capture and Windows OCR only type-check on Linux.

## M-6 Overlay behaviour on Windows
**Steps:** use the panel with a game in front: does it steal focus? Is its background transparent? Drag it by the ⠿ grip, pin it. Is the
panel/labels/band absent from a screenshot you take (they are excluded from capture)? Does closing the hub keep Maru in the tray?
**Expect:** the game keeps focus and keyboard input; nothing opaque around the panel; screenshots show no overlay.
**Bring:** what differed, the Windows version, and whether the game is windowed, borderless or exclusive fullscreen.

## M-7 Korean OCR accuracy, model quality and speed, Kiwi
**Steps:** `maru doctor --deep`; `maru bench`; `maru eval-ocr <folder>` on a few screenshots (`name.png` + `name.txt` with the true text);
translate a few sentences you know well.
**Expect:** Kiwi loads (accurate breakdowns); OCR character error rate you can live with (the vision reader, `maru setup ocr-vl`, helps);
translations you judge good; latency in `maru bench` that feels fast.
**Bring:** the `eval-ocr` table, the `bench` percentiles, and translations you think are wrong with the context.

## M-8 Speech
**Needs:** `maru setup tts` done (or a speech server on another PC and `[tts] provider = "endpoint"`).
**Steps:** Settings → Speech → **Test voice**; in the panel press 🔊 on a paragraph (sentence highlight, previous/pause/next/repeat),
🔊 on a single word in a deep dive, and `Ctrl+Alt+P` (translate and read; press again to stop). Try a long paragraph.
**Expect:** natural Korean, no gap between sentences, correct highlighting, the voice stops when the panel is dismissed, a second play of
the same sentence is instant (cache). In lite mode the local voice stays off unless `run_in_lite = true`.
**Bring:** a note of how it sounds on tricky cases (`maru tts-eval` writes a page that plays the same sentences in each voice); GPU/VRAM
use while a game runs; any error text.
**Why unverified:** the `tts-server` Windows build and the Qwen3-TTS models have not been run on a real machine from here.

## M-9 Anki
**Needs:** Anki open with the AnkiConnect add-on (code `2055492159`); Settings → Anki → `enabled`; for audio, M-8 working.
| Step | Expect |
|---|---|
| Settings → Anki → **Check Anki** | connection, decks found, the note type exists (created if missing) |
| Deep dive → **Add to Anki**, pick or type a deck | a note appears with Korean, meaning, example, and tags `maru`, `topik-N`, `pos::…`, `app::<program>` |
| Look at the note's cards | an English→Korean card in `<deck>::Reverse` (unless `reverse = "off"`) |
| Open the card | spoken word and sentence play |
| Add the same word again | no duplicate, no silent failure |
| **Sync now**, or add with `sync_after_add` on | Anki starts its own AnkiWeb sync |
| Review the card several times, wait `refresh_minutes`, look the word up again | its gloss fades (intervals are read back from Anki) |
| Close Anki, press **Check Anki**, then try **Add to Anki** | a clear message, no hang or crash |
**Bring:** for each row "worked" or the exact message; a screenshot of the note in Anki's Browse view (fields, tags, deck); Anki and
AnkiConnect versions; the log lines around a failure.
**Why unverified:** the client was only run against a fake AnkiConnect; the real one rejects calls in ways the fake does not.

## M-10 Settings page and lite mode
**Steps:** change a shortcut (hotkeys apply at once), a panel option, and a model setting (needs a restart: use the page's restart button);
set `lite = true` (or run with `MARU_LITE=1`) and check the footprint (smaller model, no OCR reading model) beside a game.
**Expect:** invalid values are refused with a reason; comments in `config.toml` survive saves; the restart notice appears only when needed.
**Bring:** anything that was saved wrongly (the before/after of `config.toml`).

## M-14 Does enlarging the subtitle picture help PP-OCR on your game?
**Needs:** a recording (a minute or two is plenty) of a cutscene with Korean subtitles, `ffmpeg`, and the PP-OCR models (`maru setup ocr`).
**Steps:** write `truth.txt` (`start_seconds end_seconds the text`, one subtitle per line); find the subtitle area's `x:y:width:height` in the
video; `python3 tools/sample_subtitle_frames.py clip.mp4 truth.txt --crop x:y:w:h --out evalset`; then `maru eval-ocr evalset --engine paddle`
and `maru eval-ocr evalset --engine paddle --scale 2` (also `--scale 1.5`). Run each twice: a tiny line can flip between runs.
**Expect:** a lower mean CER with `--scale 2` on your game, by more than the run-to-run spread, before you set `[continuous] ocr_scale = 2`.
**Bring:** both tables and what the game and its subtitle font look like (outlined, shadowed, plain).
**Why unverified:** Claude measured it on one recorded cutscene (13 subtitles, Linux CPU, ONNX Runtime 1.31, the pictures enlarged with an
outside tool): 4.5–5.1% error without, 3.9% with 2× cubic, 4.0% Lanczos, 4.4% linear, 4.8% nearest, and 4.5% when Maru's own enlargement did it.
The gain depends on the resize method, so it is not established. Not run on Windows, a real game or the live path end to end.

---

## Results log

Add a line per test run. An empty log means **nothing below has been checked on real hardware**.

| Date | Test | PC / setup | Result | Notes |
|---|---|---|---|---|
| 2026-10-11 | M-3 live subtitles | Ryzen 9 7900X3D, RTX 5070 Ti, a game in an emulator, 1280×720, PP-OCR on CPU | partly | Reported by the user: 11 lines translated in order and listed in History → Live subtitles, with "to the first word" times of 1.1–5.4 s. Found by replaying the recorded clip: those times were inflated (the clock started at an earlier scene change; real reads took 0.5–1.4 s), and two short subtitles (about 0.9 s and 1.1 s on screen) were never translated. The watcher was fixed (text signal) and passes the replay; **needs a retest on the real PC**. The user also reported that the band covered the original subtitles, which they do not want; it now goes above or below the original (browser demo checked in Chromium, geometry unit-tested), **not yet seen on the real PC**. |
