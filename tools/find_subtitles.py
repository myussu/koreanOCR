#!/usr/bin/env python3
"""Find when subtitles are on screen in a recording, without reading them.

    python tools/find_subtitles.py clip.mp4 --band 200:780:1520:280 --want 110 --out found.txt
    python tools/sample_subtitle_frames.py clip.mp4 found.txt --crop 300:885:1320:135 --out evalset/raw/game --frames 1 --prefix a

Twice a second it looks in `--band` (x:y:width:height, video pixels: the lower part of the picture, wider than the subtitles)
for light lettering that sits in the middle and holds still from one look to the next while the picture behind it may move.
Each stretch it finds becomes a row `start end ?` in the output; the `?` says nobody knows the text yet, so
`sample_subtitle_frames.py` cuts the picture and writes no label. `--want` keeps that many stretches, spread over the whole
recording rather than its first minutes.

It deliberately does not use OCR: a test set chosen by the reader that is being tested would leave out the subtitles that
reader cannot see. The price is some stretches that are not subtitles (a boss's name, a still picture with a bright line);
they show up as pictures no reader finds Korean in. Dark lettering on a white screen is not found.

Needs `ffmpeg` on the PATH and numpy.
"""
import argparse
import subprocess
import sys

CELL = 4  # the lettering is compared on a grid of 4x4 squares of the half-size picture
SAME = 0.5  # two looks show the same subtitle when their grids overlap at least this much


def stretches(times, grids, overlap):
    """`[(start, end, grid)]`: runs of looks whose lettering stayed. `grids[i]` is the subtitle-like lettering look `i` shares
    with look `i - 1` (None when there is none); a run goes on while that keeps overlapping the lettering the run began with."""
    out, run = [], None
    for i, grid in enumerate(grids):
        if grid is not None and run is not None and overlap(run[2], grid) >= SAME:
            run = (run[0], times[i], run[2])
            continue
        if run is not None:
            out.append(run)
        run = (times[i - 1], times[i], grid) if grid is not None else None
    if run is not None:
        out.append(run)
    return out


def spread(found, want, duration):
    """At most `want` of the stretches, taken evenly over the recording: one from each of `want` equal slices of time
    (the longest in the slice), then the longest of the rest until there are enough."""
    if want <= 0 or len(found) <= want:
        return sorted(found)
    slices = {}
    for s in found:
        slices.setdefault(min(want - 1, int((s[0] + s[1]) / 2 / duration * want)), []).append(s)
    length = lambda s: s[1] - s[0]
    picked = [max(group, key=length) for group in slices.values()]
    rest = sorted((s for s in found if s not in picked), key=length, reverse=True)
    return sorted(picked + rest[: want - len(picked)])


def lettering(gray):
    """Where thin light strokes are: brighter than what is left after everything narrower than a stroke is rubbed out."""
    import numpy as np
    from numpy.lib.stride_tricks import sliding_window_view

    def slide(a, k, fn):
        p = np.pad(a, k // 2, mode="edge")
        return fn(sliding_window_view(fn(sliding_window_view(p, k, axis=1), axis=-1), k, axis=0), axis=-1)

    k = 5
    opened = slide(slide(gray, k, np.min), k, np.max)
    return ((gray.astype(np.int16) - opened) > 50) & (gray > 140)


def to_grid(mask):
    import numpy as np

    h, w = (mask.shape[0] // CELL) * CELL, (mask.shape[1] // CELL) * CELL
    return mask[:h, :w].reshape(h // CELL, CELL, w // CELL, CELL).sum(axis=(1, 3)) >= 3


def box_of(grid):
    """`(top, bottom, left, right)`, in grid squares, of the densest line of lettering and the line next to it, or None.
    Rows with far less lettering than the fullest row (a HUD's edge, a pattern in the scenery) are not part of it."""
    import numpy as np

    per_row = grid.sum(axis=1)
    peak = int(per_row.argmax())
    dense = per_row >= max(4, 0.4 * per_row[peak])
    if not dense[peak]:
        return None
    top = bottom = peak
    for step, limit in ((-1, -1), (1, len(per_row))):
        edge, gap, i = peak, 0, peak + step
        while i != limit and gap <= 4:  # a second line sits at most a few squares away
            edge, gap = (i, 0) if dense[i] else (edge, gap + 1)
            i += step
        top, bottom = (edge, bottom) if step < 0 else (top, edge)
    xs = np.nonzero(grid[top : bottom + 1])[1]
    if xs.size < 12:
        return None
    left, right = np.percentile(xs, [3, 97])
    return top, bottom + 1, int(left), int(right) + 1


def subtitle_part(grid):
    """The lettering of `grid` if it looks like a subtitle, else None: a line or two, in the middle of the band, and not as
    wide as the band (that is a horizon, not a sentence). Everything outside those lines is dropped."""
    box = box_of(grid)
    if box is None:
        return None
    top, bottom, left, right = box
    width = grid.shape[1]
    centred = abs((left + right) / 2 - width / 2) <= 0.12 * width
    if bottom - top > 22 or not centred or right - left > 0.9 * width:
        return None
    part = grid.copy()
    part[:top] = False
    part[bottom:] = False
    return part


def overlap(a, b):
    both = (a & b).sum()
    return 0.0 if both == 0 else both / min(a.sum(), b.sum())


def scan(clip, band, fps):
    """Yield `(seconds, grid)` for every look at the clip."""
    import numpy as np

    x, y, w, h = band
    w2, h2 = w // 2, h // 2
    cmd = ["ffmpeg", "-v", "error", "-hwaccel", "auto", "-i", clip, "-vf", f"fps={fps},crop={w}:{h}:{x}:{y},scale={w2}:{h2}:flags=area,format=gray", "-f", "rawvideo", "-"]
    try:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE)
    except FileNotFoundError:
        sys.exit("ffmpeg was not found; install it and try again")
    n = 0
    while True:
        raw = proc.stdout.read(w2 * h2)
        if len(raw) < w2 * h2:
            break
        yield n / fps, to_grid(lettering(np.frombuffer(raw, dtype=np.uint8).reshape(h2, w2)))
        n += 1
    if proc.wait() != 0:
        sys.exit("ffmpeg could not read the clip (is the band inside the picture?)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("clip")
    ap.add_argument("--band", required=True, help="x:y:width:height of the part of the picture to look in, in video pixels")
    ap.add_argument("--out", default="found.txt", help="where to write the rows (default: found.txt)")
    ap.add_argument("--want", type=int, default=0, help="keep this many stretches, spread over the recording (default: all)")
    ap.add_argument("--fps", type=float, default=2.0, help="looks per second (default 2)")
    ap.add_argument("--min-seconds", type=float, default=1.0, help="ignore lettering that stayed for less than this (default 1)")
    ap.add_argument("--skip", help="a truth file whose stretches are left out (they are already in the test set)")
    args = ap.parse_args()
    try:
        import numpy  # noqa: F401
    except ImportError:
        sys.exit("this tool needs numpy (pip install numpy)")
    try:
        band = tuple(int(v) for v in args.band.split(":"))
        assert len(band) == 4
    except (ValueError, AssertionError):
        sys.exit("--band must be x:y:width:height, for example 200:780:1520:280")

    times, grids, last = [], [], None
    for t, grid in scan(args.clip, band, args.fps):
        times.append(t)
        grids.append(None if last is None else subtitle_part(last & grid))
        last = grid
        if len(times) % 1200 == 0:
            print(f"  {t / 60:.0f} min …", file=sys.stderr)
    if not times:
        sys.exit("no frames were read")
    found = [s for s in stretches(times, grids, overlap) if s[1] - s[0] >= args.min_seconds - 1e-6]
    if args.skip:
        taken = []
        for line in open(args.skip, encoding="utf-8"):
            parts = line.split(None, 2)
            if len(parts) == 3 and not line.lstrip().startswith("#"):
                taken.append((float(parts[0]), float(parts[1])))
        found = [s for s in found if not any(s[0] < b + 1.0 and a - 1.0 < s[1] for a, b in taken)]
    kept = spread(found, args.want, times[-1] or 1.0)

    x, y = band[0], band[1]
    px = 2 * CELL  # one grid square in video pixels
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(f"# {len(kept)} of {len(found)} stretches of lettering found in {args.clip} (band {args.band}); `?` = text not known\n")
        for start, end, grid in kept:
            top, bottom, left, right = box_of(grid)
            f.write(f"# lettering at x {x + left * px}-{x + right * px}, y {y + top * px}-{y + bottom * px}\n{start:.2f} {end:.2f} ?\n")
    print(f"{len(kept)} stretches (of {len(found)} found) written to {args.out}")


if __name__ == "__main__":
    main()
