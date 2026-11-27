#!/usr/bin/env python3
"""Score OCR on whole screens (menus, HUDs, dialogue boxes), line by line.

A subtitle test set has one text per picture; a screen has many, in no fixed order, and its readers find them in different
orders. So a screen's label is its Korean lines (`name.txt`, one per line, any order), and each labelled line is looked
for in what an engine read: the closest stretch of the engine's letters, wherever it is.

    python tools/scan_lines.py label <set>                 # provisional labels, line by line (see below)
    python tools/scan_lines.py score <set> a.json b.json   # the first saved run is the one the others are judged against

`label`: a line gets into `name.provisional.txt` when the outside reader (`name.outside.txt`) wrote it and one of
today's readers (`name.scan.txt`: the screen scan; `name.hybrid.txt`: a capture) read the same letters somewhere in the
picture. Lines with fewer than two Hangul syllables are left out (they match by chance). So a provisional label lists
only the lines two readers agreed on, and is not all the text on the screen: a person's `name.txt` should be.

`score`: the runs are `maru eval-ocr <set> ... --labels all --save-report run.json` (eval-ocr's own letter error means
little on a whole screen; this one is per line). For each labelled line: its letter error against the closest stretch
of the engine's letters (spaces and punctuation ignored), "found" when that is at most a third, "exact" when it is
nothing. Then each run against the first, line by line, with a sign test. `--split dev|held-out` uses eval-ocr's halves.
Everything scored on provisional lines is marked as such: those lines are, by construction, ones today's readers read.
"""
import argparse
import json
import math
import sys
from pathlib import Path

LOCAL = ("scan", "hybrid")


def letters(text):
    """Letters and digits only, as eval-ocr compares them."""
    return "".join(c for c in text if c.isalnum())


def syllables(text):
    return sum(1 for c in text if "가" <= c <= "힣")


def read(path):
    return path.read_text(encoding="utf-8").strip() if path.exists() else None


def agreed_lines(outside, locals_):
    """The outside reader's lines (two Hangul syllables or more) whose letters a local reader also read."""
    found = [letters(t) for t in locals_ if t]
    out = []
    for line in (outside or "").splitlines():
        line = " ".join(line.split())
        want = letters(line)
        if syllables(line) >= 2 and want and any(want in f for f in found) and line not in out:
            out.append(line)
    return out


def label(root, outside="outside", local=LOCAL):
    """Write `name.provisional.txt` where lines agree; returns (pictures labelled, lines, pictures left)."""
    done = lines = left = 0
    for png in sorted(Path(root).rglob("*.png")):
        if png.with_suffix(".txt").exists() or png.with_suffix(".unreadable").exists():
            continue
        out = read(png.with_suffix(f".{outside}.txt"))
        agreed = agreed_lines(out, [read(png.with_suffix(f".{t}.txt")) for t in local])
        target = png.with_suffix(".provisional.txt")
        if agreed:
            target.write_text("\n".join(agreed) + "\n", encoding="utf-8")
            done += 1
            lines += len(agreed)
        else:
            if target.exists():
                target.unlink()
            left += 1
    return done, lines, left


def line_error(line, got):
    """Edit distance from the line's letters to the closest stretch of `got`'s letters, over the line's length (at most 1)."""
    a, b = letters(line), letters(got)
    if not a:
        return 0.0
    # Free start and end in `b`: the line may be anywhere in what was read.
    prev = [0] * (len(b) + 1)
    for i, ca in enumerate(a, 1):
        cur = [i] + [0] * len(b)
        for j, cb in enumerate(b, 1):
            cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb))
        prev = cur
    return min(1.0, min(prev) / len(a))


def sign_test(better, worse):
    """Two-sided exact sign test."""
    n = better + worse
    if n == 0:
        return 1.0
    k = min(better, worse)
    p = sum(math.comb(n, i) for i in range(k + 1)) / 2 ** n
    return min(1.0, 2 * p)


def scored(report, split=None):
    """`(name, {(picture, line): error}, provisional?, times)` for one saved run."""
    data = json.loads(Path(report).read_text(encoding="utf-8"))
    engine = data["engines"][0]
    errors, times, provisional = {}, [], False
    for p in engine["pictures"]:
        if split and p.get("split") != split:
            continue
        provisional |= "provisional" in p.get("tags", [])
        times.append(p["ms"])
        for i, line in enumerate(l for l in p["truth"].splitlines() if l.strip()):
            errors[(p["rel"], i)] = line_error(line, p["got"])
    return engine["engine"], errors, provisional, sorted(times)


def summary(errors, times):
    n = len(errors) or 1
    found = sum(e <= 1 / 3 for e in errors.values()) / n
    exact = sum(e == 0 for e in errors.values()) / n
    mean = sum(errors.values()) / n
    p50 = times[len(times) // 2] if times else 0
    return found, exact, mean, p50


def score(reports, split=None, out=sys.stdout):
    runs = [scored(r, split) for r in reports]
    if any(prov for _, _, prov, _ in runs):
        print("PROVISIONAL: some labelled lines are ones two readers agreed on and no person checked; today's readers read them, so a candidate can only tie or lose there.\n", file=out)
    print("| engine | lines | text found | exact lines | letter error | p50 ms per screen |", file=out)
    print("|---|---:|---:|---:|---:|---:|", file=out)
    for name, errors, _, times in runs:
        found, exact, mean, p50 = summary(errors, times)
        print(f"| {name} | {len(errors)} | {found:.1%} | {exact:.1%} | {mean:.1%} | {p50} |", file=out)
    base_name, base, _, _ = runs[0]
    print(f"\nAgainst **{base_name}**, line by line:\n", file=out)
    print("| engine | better | worse | the same | sign test p |", file=out)
    print("|---|---:|---:|---:|---:|", file=out)
    for name, errors, _, _ in runs[1:]:
        common = base.keys() & errors.keys()
        better = sum(errors[k] < base[k] for k in common)
        worse = sum(errors[k] > base[k] for k in common)
        print(f"| {name} | {better} | {worse} | {len(common) - better - worse} | {sign_test(better, worse):.3f} |", file=out)


def main():
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    lab = sub.add_parser("label", help="write provisional labels, line by line")
    lab.add_argument("set")
    sc = sub.add_parser("score", help="score saved eval-ocr runs line by line")
    sc.add_argument("set")
    sc.add_argument("reports", nargs="+")
    sc.add_argument("--split", choices=["dev", "held-out"])
    args = ap.parse_args()
    if args.cmd == "label":
        done, lines, left = label(args.set)
        print(f"{done} screens got a provisional label ({lines} lines two readers agreed on); {left} were left for a person")
    else:
        score(args.reports, args.split)


if __name__ == "__main__":
    main()
