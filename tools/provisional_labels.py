#!/usr/bin/env python3
"""Write provisional labels for test pictures from the drafts of independent readers.

    maru eval-ocr <set> --engine paddle hybrid --write-drafts
    maru eval-ocr <set> --engine connection --connection Claude --write-drafts --draft-tag outside
    python tools/provisional_labels.py <set> --outside outside --local hybrid paddle

A picture gets `name.provisional.txt` when the outside reader (`name.<outside>.txt`) and at least one local reader read the
same letters and digits; spaces and punctuation are ignored, as `maru eval-ocr` ignores them when it scores. The text
written is the outside reader's. Everything else is left without a label for a person to do in `tools/label_review.html`,
which starts each box with the provisional text or, failing that, the outside reader's.

A provisional label is not a label. No person has seen it, the readers can agree on the same mistake, and the pictures
they agree on are the easy ones. `maru eval-ocr` only uses it with `--labels provisional` and says so in its output.
A picture that already has `name.txt` (a person's label) is never given one; with `--check` the rule is applied to those
pictures anyway and compared with their labels, which says how often a provisional label would have been wrong.

`--set-aside FOLDER` moves pictures in which no reader found any Korean, and later pictures of a game whose outside
reading repeats an earlier one, out of the set (with their drafts), so they are neither labelled nor scored.
"""
import argparse
import shutil
import sys
from pathlib import Path

PICTURES = {".png", ".jpg", ".jpeg", ".bmp"}


def letters(text):
    """Letters and digits only: what a reading is judged on."""
    return "".join(c for c in (text or "") if c.isalnum())


def has_korean(text):
    return any("가" <= c <= "힣" for c in text or "")


def agreed(outside, local):
    """The outside reader's text if a local reader read the same letters, else None. Nothing read is never an agreement."""
    want = letters(outside)
    if not want:
        return None
    return " ".join(outside.split()) if any(letters(t) == want for t in local) else None


def read(path):
    try:
        return path.read_text(encoding="utf-8").strip()
    except OSError:
        return None


def pictures(root):
    return sorted(p for p in Path(root).rglob("*") if p.suffix.lower() in PICTURES)


def draft(picture, tag):
    return read(picture.with_name(f"{picture.stem}.{tag}.txt"))


def check(root, outside, local):
    """How the rule does on pictures a person labelled: (labelled, got a provisional label, of those wrong, examples)."""
    labelled = covered = wrong = 0
    examples = []
    for p in pictures(root):
        truth = read(p.with_suffix(".txt"))
        if truth is None or p.with_suffix(".unreadable").exists():
            continue
        labelled += 1
        text = agreed(draft(p, outside), [draft(p, t) for t in local])
        if text is None:
            continue
        covered += 1
        if letters(text) != letters(truth):
            wrong += 1
            examples.append((p.name, truth, text))
    return labelled, covered, wrong, examples


def move_aside(picture, root, aside, why):
    dest = Path(aside) / why / picture.parent.relative_to(root)
    dest.mkdir(parents=True, exist_ok=True)
    for f in picture.parent.glob(picture.stem + ".*"):
        shutil.move(str(f), str(dest / f.name))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("set", help="the test set folder (one folder per game below it)")
    ap.add_argument("--outside", required=True, help="the draft name of the outside reader (name.<this>.txt)")
    ap.add_argument("--local", nargs="+", required=True, help="the draft names of the local readers")
    ap.add_argument("--check", action="store_true", help="do not write: apply the rule to pictures a person labelled and compare")
    ap.add_argument("--set-aside", metavar="FOLDER", help="move pictures with no Korean in any reading, and repeats, to this folder")
    args = ap.parse_args()
    # What this prints is Korean; a Windows console or pipe that is not UTF-8 must not make the check fail.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    root = Path(args.set)
    if not root.is_dir():
        sys.exit(f"{root} is not a folder")

    if args.check:
        labelled, covered, wrong, examples = check(root, args.outside, args.local)
        print(f"{labelled} pictures have a person's label; the rule would have given {covered} of them a provisional label")
        if covered:
            print(f"{wrong} of those {covered} provisional labels differ from the person's label in letters ({100 * wrong / covered:.1f}%)")
        for name, truth, text in examples:
            print(f"  {name}\n    label:       {truth}\n    provisional: {text}")
        return

    written = waiting = no_outside = aside = repeats = 0
    seen = {}
    for p in pictures(root):
        if p.with_suffix(".unreadable").exists():
            continue
        label = read(p.with_suffix(".txt"))
        if label is not None:
            seen.setdefault((p.parent, letters(label)), p)  # a later picture of a subtitle a person labelled is a repeat too
            continue
        out = draft(p, args.outside)
        local = [draft(p, t) for t in args.local]
        target = p.with_name(p.stem + ".provisional.txt")
        if out is None:
            no_outside += 1  # the outside reader has not read it yet: nothing can be said
            continue
        if args.set_aside and not has_korean(out) and not any(has_korean(t) for t in local):
            move_aside(p, root, args.set_aside, "no-korean")
            aside += 1
            continue
        key = (p.parent, letters(out))
        if args.set_aside and letters(out) and key in seen:
            move_aside(p, root, args.set_aside, "repeats")
            repeats += 1
            continue
        seen[key] = p
        text = agreed(out, local)
        if text is None:
            target.unlink(missing_ok=True)  # an earlier run's label that the drafts no longer support
            waiting += 1
        else:
            target.write_text(text + "\n", encoding="utf-8")
            written += 1
    print(f"{written} provisional labels written; {waiting} pictures wait for a person (the readers disagree)")
    if no_outside:
        print(f"{no_outside} pictures have no reading from the outside reader yet and were left alone")
    if args.set_aside:
        print(f"set aside in {args.set_aside}: {aside} with no Korean in any reading, {repeats} repeats of an earlier subtitle")


if __name__ == "__main__":
    main()
