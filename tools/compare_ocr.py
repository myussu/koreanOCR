#!/usr/bin/env python3
"""Score today's OCR and every candidate on a labelled test set, and print the comparison tables.

    python tools/compare_ocr.py [test set]      (default: Documents\\maru-evalset\\raw)

It runs `maru eval-ocr` once per engine in CANDIDATES (the first is today's setup, the one the others are judged
against), saves each run's per-picture results under `--out` (default: `reports` beside the test set), and then prints,
with `maru eval-report`: every engine overall and game by game, and each engine against today's, subtitle by subtitle,
with a sign test. A candidate whose model files are not on this PC is skipped and named.

`--labels person` (the default) scores only pictures a person labelled: these are the numbers a decision may rest on.
`--labels all` also uses provisional labels (`tools/provisional_labels.py`) where no person has labelled (a person's
`name.txt` always wins over `name.provisional.txt`). It prints the whole set first, then the pictures a person labelled
(whole, and each half), then the provisional ones apart; everything that includes provisional labels is marked.

`--tables` prints the tables again from the saved runs without reading a picture. `--only NAME ...` runs a few engines.
One engine at a time, and nothing else heavy running, or the times and memory figures mean little.
Needs `maru` built (`cargo build --release -p maru-cli`), `maru setup ocr` and `maru setup ocr-vl`.
"""
import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

VL16 = "vlm/paddleocr-vl-1.6"
OCR = "ocr-candidates"
V6_TUNING = "0.2,0.45,1.4"  # the values PP-OCRv6 detectors are published with (score, box, unclip)

# (name, eval-ocr arguments, model files it needs below the models folder). `{m}` is the models folder.
SMALL = "ocr/pp-ocrv6-small/det.onnx"  # where `maru setup ocr` installs it
# A file `maru setup` installs, and where run 3 left the same file: either will do.
ALSO = {SMALL: OCR + "/pp-ocrv6-small-det/det.onnx"}

CANDIDATES = [
    ("today, live subtitles: PP-OCR alone, v6 small detector", ["--engine", "paddle", "--det", "{m}/" + SMALL], [SMALL]),
    ("today, captures: hybrid, reader PaddleOCR-VL-1.6, keep sure lines 0.90", ["--engine", "hybrid", "--reader-dir", "{m}/" + VL16, "--keep-sure", "0.90"], [VL16]),
    ("before run 4: hybrid (PP-OCR + PaddleOCR-VL-1.5)", ["--engine", "hybrid"], []),
    ("PP-OCR alone (also live CPU PP-OCR mode)", ["--engine", "paddle"], []),
    ("PP-OCR alone on the graphics card", ["--engine", "paddle", "--device", "directml:0"], []),
    ("PaddleOCR-VL-1.5 alone", ["--engine", "paddle-vl"], []),
    ("hybrid, enlarged 2x", ["--engine", "hybrid", "--scale", "2"], []),
    ("PP-OCR alone, enlarged 2x", ["--engine", "paddle", "--scale", "2"], []),
    ("hybrid, reader PaddleOCR-VL-1.6", ["--engine", "hybrid", "--reader-dir", "{m}/" + VL16], [VL16]),
    ("PP-OCR alone, v5 server detector (official file)", ["--engine", "paddle", "--det", "{m}/" + OCR + "/pp-ocrv5-server-det/det.onnx"], [OCR + "/pp-ocrv5-server-det/det.onnx"]),
    ("PP-OCR alone, v5 mobile detector", ["--engine", "paddle", "--det", "{m}/" + OCR + "/pp-ocrv5-mobile-det/det.onnx"], [OCR + "/pp-ocrv5-mobile-det/det.onnx"]),
    ("PP-OCR alone, v6 small detector, stray lines dropped", ["--engine", "paddle", "--det", "{m}/" + SMALL, "--drop-stray-lines"], [SMALL]),
    ("PP-OCR alone, v6 small detector + Qota-OCR recogniser", ["--engine", "paddle", "--det", "{m}/" + SMALL, "--rec", "{m}/" + OCR + "/qota-ocr/rec.onnx", "--dict", "{m}/" + OCR + "/qota-ocr/dict.txt"], [SMALL, OCR + "/qota-ocr/rec.onnx", OCR + "/qota-ocr/dict.txt"]),
    ("PP-OCR alone, v6 small detector, its own tuning", ["--engine", "paddle", "--det", "{m}/" + OCR + "/pp-ocrv6-small-det/det.onnx", "--det-tuning", V6_TUNING], [OCR + "/pp-ocrv6-small-det/det.onnx"]),
    ("PP-OCR alone, v6 medium detector", ["--engine", "paddle", "--det", "{m}/" + OCR + "/pp-ocrv6-medium-det/det.onnx"], [OCR + "/pp-ocrv6-medium-det/det.onnx"]),
    ("PP-OCR alone, v6 medium detector, its own tuning", ["--engine", "paddle", "--det", "{m}/" + OCR + "/pp-ocrv6-medium-det/det.onnx", "--det-tuning", V6_TUNING], [OCR + "/pp-ocrv6-medium-det/det.onnx"]),
    ("hybrid, v5 mobile detector", ["--engine", "hybrid", "--det", "{m}/" + OCR + "/pp-ocrv5-mobile-det/det.onnx"], [OCR + "/pp-ocrv5-mobile-det/det.onnx"]),
    ("hybrid, v6 medium detector, its own tuning", ["--engine", "hybrid", "--det", "{m}/" + OCR + "/pp-ocrv6-medium-det/det.onnx", "--det-tuning", V6_TUNING], [OCR + "/pp-ocrv6-medium-det/det.onnx"]),
    ("PP-OCR alone, Korean v5 recogniser (official file)", ["--engine", "paddle", "--rec", "{m}/" + OCR + "/korean-pp-ocrv5-mobile-rec/rec.onnx"], [OCR + "/korean-pp-ocrv5-mobile-rec/rec.onnx"]),
    ("PP-OCR alone, Qota-OCR recogniser", ["--engine", "paddle", "--rec", "{m}/" + OCR + "/qota-ocr/rec.onnx", "--dict", "{m}/" + OCR + "/qota-ocr/dict.txt"], [OCR + "/qota-ocr/rec.onnx", OCR + "/qota-ocr/dict.txt"]),
    ("hybrid, Qota-OCR recogniser", ["--engine", "hybrid", "--rec", "{m}/" + OCR + "/qota-ocr/rec.onnx", "--dict", "{m}/" + OCR + "/qota-ocr/dict.txt"], [OCR + "/qota-ocr/rec.onnx", OCR + "/qota-ocr/dict.txt"]),
    ("hybrid, reader GLM-OCR", ["--engine", "hybrid", "--reader-dir", "{m}/vlm/glm-ocr", "--vlm-prompt", "Text Recognition:"], ["vlm/glm-ocr"]),
    ("hybrid, reader Surya 2.5", ["--engine", "hybrid", "--reader-dir", "{m}/vlm/surya-2.5", "--vlm-prompt", "OCR this block image to HTML.", "--reader-html"], ["vlm/surya-2.5"]),
    ("hybrid, reader Nanonets-OCR2 1.5B", ["--engine", "hybrid", "--reader-dir", "{m}/vlm/nanonets-ocr2-1.5b", "--vlm-prompt", "Extract the text from the above image as if you were reading it naturally."], ["vlm/nanonets-ocr2-1.5b"]),
]


def slug(name):
    """A file name for an engine: `hybrid, reader GLM-OCR` -> `hybrid-reader-glm-ocr`."""
    return re.sub(r"[^a-z0-9.]+", "-", name.lower()).strip("-")


def available(candidates, models, only=None):
    """`(usable, skipped)`: candidates whose files are all there (and named in `only`, when given), and the rest with what is missing."""
    usable, skipped = [], []
    for name, args, needs in candidates:
        if only and not any(o.lower() in name.lower() for o in only):
            continue
        swap = {n: ALSO[n] for n in needs if not (Path(models) / n).exists() and n in ALSO and (Path(models) / ALSO[n]).exists()}
        missing = [n for n in needs if n not in swap and not (Path(models) / n).exists()]
        if missing:
            skipped.append((name, missing))
        else:
            def fill(a):
                rel = a[len("{m}/"):] if a.startswith("{m}/") else None
                return "{m}/" + swap[rel] if rel in swap else a
            usable.append((name, [fill(a).replace("{m}", str(models)) for a in args]))
    return usable, skipped


def eval_command(maru, home, test_set, name, args, labels, report):
    return [str(maru), *(["--home", str(home)] if home else []), "eval-ocr", str(test_set), *args, "--labels", labels, "--name", name, "--save-report", str(report)]


def table_views(labels):
    """`(heading, eval-report arguments)` for each table to print."""
    if labels == "person":
        return [("Every labelled picture", []), ("The half to tune on (dev)", ["--split", "dev"]), ("The half to judge on (held-out)", ["--split", "held-out"])]
    if labels == "provisional":
        return [("PROVISIONAL labels, the half to tune on (dev)", ["--split", "dev"]), ("PROVISIONAL labels, the half to judge on (held-out)", ["--split", "held-out"])]
    return [
        ("The whole set: a person's label wherever there is one, else the provisional one (PROVISIONAL)", []),
        ("The whole set, the half to judge on (held-out) (PROVISIONAL)", ["--split", "held-out"]),
        ("Pictures a person labelled", ["--tag", "person"]),
        ("Pictures a person labelled, the half to tune on (dev)", ["--tag", "person", "--split", "dev"]),
        ("Pictures a person labelled, the half to judge on (held-out)", ["--tag", "person", "--split", "held-out"]),
        ("PROVISIONAL labels only, the half to tune on (dev)", ["--tag", "provisional", "--split", "dev"]),
        ("PROVISIONAL labels only, the half to judge on (held-out)", ["--tag", "provisional", "--split", "held-out"]),
    ]


def default_set():
    """`Documents\\maru-evalset\\raw`, where Maru saves the pictures it reads and the label page can open them."""
    return Path.home() / "Documents" / "maru-evalset" / "raw"


def default_models():
    appdata = os.environ.get("APPDATA")
    return Path(appdata) / "maru" / "Maru" / "data" / "models" if appdata else None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("set", nargs="?", default=str(default_set()), help="the test set folder, one folder per game below it (default: Documents\\maru-evalset\\raw)")
    ap.add_argument("--maru", default=str(Path("target") / "release" / ("maru.exe" if os.name == "nt" else "maru")), help="the maru program (default: target/release/maru)")
    ap.add_argument("--home", help="run maru with this --home (default: your own)")
    ap.add_argument("--models", help="Maru's models folder, where the candidates' files are (default: the one in your data folder)")
    ap.add_argument("--out", help="where the runs are saved (default: `reports` beside the test set)")
    ap.add_argument("--labels", choices=["person", "provisional", "all"], default="person")
    ap.add_argument("--only", nargs="+", metavar="NAME", help="only engines whose name contains one of these (today's is always included)")
    ap.add_argument("--tables", action="store_true", help="do not read any picture: print the tables from the runs already saved")
    ap.add_argument("--keep", action="store_true", help="do not run an engine again if its run is already saved")
    args = ap.parse_args()

    test_set = Path(args.set)
    out = Path(args.out) if args.out else test_set.parent / "reports"
    out.mkdir(parents=True, exist_ok=True)
    models = Path(args.models) if args.models else (Path(args.home) / "models" if args.home else default_models())
    if models is None or not models.is_dir():
        sys.exit("give Maru's models folder with --models (`maru doctor` prints the data folder it is in)")
    only = [CANDIDATES[0][0], *args.only] if args.only else None
    usable, skipped = available(CANDIDATES, models, only)
    for name, missing in skipped:
        print(f"skipped: {name} (not on this PC: {', '.join(missing)})")

    reports = []
    for name, extra in usable:
        report = out / f"{slug(name)}.json"
        if not args.tables and not (args.keep and report.exists()):
            print(f"\n=== {name}", flush=True)
            cmd = eval_command(args.maru, args.home, test_set, name, extra, args.labels, report)
            with open(out / f"{slug(name)}.log", "w", encoding="utf-8") as log:
                done = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT)
            if done.returncode != 0 or not report.exists():
                print(f"  did not finish (see {out / (slug(name) + '.log')})")
                continue
        if report.exists():
            reports.append(report)
    if not reports or reports[0].name != f"{slug(CANDIDATES[0][0])}.json":
        sys.exit("today's setup has no saved run, so there is nothing to compare with")

    for heading, view in table_views(args.labels):
        print(f"\n\n## {heading}\n", flush=True)
        subprocess.run([str(args.maru), "eval-report", *map(str, reports), *view])


if __name__ == "__main__":
    main()
