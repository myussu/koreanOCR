#!/usr/bin/env python3
"""Cut test pictures for `maru eval-ocr` out of a recording of subtitles.

You give it the video, the area where the subtitles are, and a text file that says when each subtitle is on screen and what it
says. It saves a few frames from inside each subtitle as `name.png` with the right text in `name.txt`:

    python3 tools/sample_subtitle_frames.py clip.mp4 truth.txt --crop 160:545:960:130 --out evalset
    maru eval-ocr evalset --engine paddle
    maru eval-ocr evalset --engine paddle --scale 2      # the same, with PP-OCR enlarging the picture first

`truth.txt` has one subtitle per line: `start_seconds end_seconds the text` (a two-line subtitle is written on one line, the
rows separated by a space). `--crop` is `x:y:width:height` in the video's pixels. Needs `ffmpeg` on the PATH; nothing else.

Several frames of the same subtitle share one text, so they are not independent tests: judge a change by how many different
subtitles it fixes or breaks (the per-subtitle summary at the end), not by the picture count.

A row whose text is `?` is a subtitle nobody has read yet (`tools/find_subtitles.py` writes such rows): its pictures are cut
and no `name.txt` is written, so they wait for a label in `tools/label_review.html`. With `--prefix` the pictures are named
`<prefix><nn>_s001_0.png`, where `nn` counts five-minute stretches of the video: `maru eval-ocr` keeps each stretch whole on
one side of its dev / held-out split, and pictures from several videos can share a folder. `--tag` writes `name.tags`.
"""
import argparse
import subprocess
import sys
from pathlib import Path


def read_truth(path):
    rows = []
    for number, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split(None, 2)
        if len(parts) < 3:
            sys.exit(f"{path}:{number}: expected `start end text`, got {line!r}")
        try:
            start, end = float(parts[0]), float(parts[1])
        except ValueError:
            sys.exit(f"{path}:{number}: start and end must be seconds, got {parts[0]!r} {parts[1]!r}")
        if end <= start:
            sys.exit(f"{path}:{number}: the subtitle ends before it starts")
        rows.append((start, end, parts[2].strip()))
    if not rows:
        sys.exit(f"{path}: no subtitles found")
    return rows


UNREAD = "?"  # the text of a row nobody has read yet
SCENE_SECONDS = 300  # as `[continuous] save_reads` groups its pictures


def picture_name(prefix, start, index, k):
    """`s012_3` as before; with a prefix, `<prefix>07_s012_3`: the part before the first `_` is the scene eval-ocr splits on."""
    scene = f"{prefix}{int(start // SCENE_SECONDS):02d}_" if prefix else ""
    return f"{scene}s{index:03d}_{k}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("clip")
    ap.add_argument("truth")
    ap.add_argument("--crop", required=True, help="x:y:width:height of the subtitle area, in video pixels")
    ap.add_argument("--out", default="evalset", help="folder for the pictures (default: evalset)")
    ap.add_argument("--frames", type=int, default=4, help="frames per subtitle, spread over its middle (default 4; short subtitles get fewer)")
    ap.add_argument("--prefix", help="name pictures <prefix><five-minute stretch>_s001_0.png instead of s001_0.png")
    ap.add_argument("--tag", help="write this in name.tags beside every labelled picture (for example: gold)")
    args = ap.parse_args()

    try:
        x, y, w, h = (int(v) for v in args.crop.split(":"))
    except ValueError:
        sys.exit("--crop must be x:y:width:height, for example 160:545:960:130")
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    saved = 0
    for index, (start, end, text) in enumerate(read_truth(args.truth), 1):
        length = end - start
        count = max(1, min(args.frames, int(length / 0.3)))  # about one frame per 0.3 s at most
        for k in range(count):
            # the middle 60% of the subtitle: its edges are fading in or out
            t = start + length * (0.5 if count == 1 else 0.2 + 0.6 * k / (count - 1))
            name = picture_name(args.prefix, start, index, k)
            # ffmpeg's crop filter takes width:height:x:y
            cmd = ["ffmpeg", "-v", "error", "-y", "-ss", f"{t:.3f}", "-i", args.clip, "-frames:v", "1", "-vf", f"crop={w}:{h}:{x}:{y}", str(out / f"{name}.png")]
            try:
                subprocess.run(cmd, check=True)
            except FileNotFoundError:
                sys.exit("ffmpeg was not found; install it and try again")
            except subprocess.CalledProcessError:
                sys.exit(f"ffmpeg could not read a frame at {t:.2f}s (is the time inside the video, and the crop inside the picture?)")
            if text != UNREAD:
                (out / f"{name}.txt").write_text(text, encoding="utf-8")
                if args.tag:
                    (out / f"{name}.tags").write_text(args.tag, encoding="utf-8")
            saved += 1
    print(f"{saved} pictures in {out}/")


if __name__ == "__main__":
    main()
