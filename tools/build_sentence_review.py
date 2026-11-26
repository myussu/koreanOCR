"""Build the blind review page for whole-sentence translations, and score it once it has been marked.

`maru bench --live --alone --connection X --out DIR` writes, per connection, how it translated every line sent by
itself (`DIR/live-bench.jsonl`), which is how the sentence of a card is translated. This puts those translations
under each line for a person to judge without knowing which candidate wrote what: in each line the translations are
called A, B, C… in a different random order, and the key that says who is who is written to a separate file. The
page chooses no winner.

  python tools/build_sentence_review.py build lines.txt --candidate NAME=DIR [NAME=DIR …] --out review.html --key key.json
         [--seed 1]
  python tools/build_sentence_review.py score marks.json key.json

`lines.txt` is the file the bench was given (one line each); NAME is what the key and the score call that candidate,
DIR its `--out` folder (or the `.jsonl` itself). The lines are yours: keep the page and the key out of the repository.

On the page, click a translation to mark it right (✓), click again for wrong (✗), again to clear. Two candidates
that wrote the same translation share one letter. A line a candidate failed on, or answered with nothing, is shown
as "(no answer)" and counted apart, not as wrong. Marks stay in the browser; "Save marks" downloads them as
`marks.json` for `score`.
"""
import argparse
import hashlib
import json
import os
import random
import string
import sys


def load(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def read_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def read_lines(path):
    with open(path, encoding="utf-8-sig") as f:
        return [line.strip() for line in f if line.strip()]


def write(path, text):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def answers(records, lines):
    """One candidate's translation of every line, `None` where it failed or said nothing."""
    out = [None] * len(lines)
    seen = set()
    for r in records:
        n = r["line"]
        # With several passes the first one is shown: the page judges one translation per candidate.
        if n in seen or not 1 <= n <= len(lines):
            continue
        seen.add(n)
        # The bench keeps a byte-order mark on the file's first line; the lines here are read without it.
        if r["source"].lstrip("﻿").strip() != lines[n - 1]:
            raise ValueError(f"line {n} of these results is not line {n} of the lines file: they were made from another file")
        text = " ".join((r.get("translation") or "").split())
        out[n - 1] = None if r.get("error") or not text else text
    return out


def assign(translations, seed):
    """For every line, the different translations under letters in a random order of that line's own.

    `translations` is `{candidate: [translation or None per line]}`. Returns `(key lines, what each letter shows)`.
    Candidates that wrote the same thing share a letter; those with no answer share one too, named in `no_answer`.
    """
    rng = random.Random(seed)
    count = len(next(iter(translations.values()), []))
    key, shown = [], []
    for n in range(count):
        order = list(translations)
        rng.shuffle(order)
        letters, texts, no_answer = {}, {}, None
        for name in order:
            text = translations[name][n]
            letter = next((l for l, t in texts.items() if t == text), None)
            if letter is None:
                letter = string.ascii_uppercase[len(texts)]
                texts[letter] = text
                letters[letter] = []
                if text is None:
                    no_answer = letter
            letters[letter].append(name)
        key.append({"line": n + 1, "letters": letters, "no_answer": no_answer})
        shown.append(texts)
    return key, shown


PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sentence translations: blind review</title>
<style>
:root { color-scheme: light dark; --bg:#f6f7f9; --card:#fff; --fg:#1d2330; --muted:#667085; --line:#d9dde6; --right:#d7f2e2; --wrong:#fbdcdc; --accent:#2f5fd0; --on-accent:#fff; }
@media (prefers-color-scheme: dark) { :root { --bg:#14171d; --card:#1c2029; --fg:#e7e9ee; --muted:#98a1b3; --line:#303746; --right:#1f4a36; --wrong:#5a2626; --accent:#86a8ff; --on-accent:#10131a; } }
* { box-sizing: border-box; }
body { margin: 0; font: 15px/1.5 system-ui, "Segoe UI", "Malgun Gothic", sans-serif; background: var(--bg); color: var(--fg); }
header { position: sticky; top: 0; z-index: 2; background: var(--card); border-bottom: 1px solid var(--line); padding: 10px 20px; display: flex; gap: 16px; align-items: center; flex-wrap: wrap; }
header h1 { font-size: 17px; margin: 0; }
header .count { color: var(--muted); font-variant-numeric: tabular-nums; }
button { font: inherit; padding: 6px 14px; border-radius: 8px; border: 1px solid var(--line); background: var(--card); color: var(--fg); cursor: pointer; }
button.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
main { max-width: 1000px; margin: 0 auto; padding: 16px 20px 80px; }
.how { color: var(--muted); max-width: 90ch; }
section { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 14px 16px; margin: 16px 0; }
.ko { font-family: "Malgun Gothic", "Noto Sans KR", sans-serif; }
h2 { font-size: 19px; margin: 0 0 10px; font-weight: 600; overflow-wrap: anywhere; }
h2 small { color: var(--muted); font-weight: 400; font-size: 13px; margin-right: 8px; }
button.choice { display: flex; gap: 10px; align-items: baseline; width: 100%; margin: 6px 0 0; padding: 8px 10px; text-align: left; overflow-wrap: anywhere; }
button.choice b { flex: none; width: 1.4em; color: var(--muted); }
button.choice i { flex: none; width: 1.2em; font-style: normal; font-weight: 700; }
button.choice.right { background: var(--right); }
button.choice.wrong { background: var(--wrong); }
button.choice.right i::before { content: "✓"; }
button.choice.wrong i::before { content: "✗"; }
button.choice.none { color: var(--muted); font-style: italic; cursor: default; }
</style></head><body>
<header><h1>Sentence translations: blind review</h1><span class="count" id="count"></span>
<button class="primary" id="save">Save marks</button><button id="clear">Clear all marks</button></header>
<main>
<p class="how">Each line was translated by every candidate <strong>by itself</strong>, with no earlier lines to go on, the way the sentence of a card is. The translations are called A, B, C… and <strong>the letters are shuffled in every line</strong>, so a letter is not the same candidate from one line to the next. Candidates that wrote exactly the same translation share one letter, so some lines show fewer. Click a translation once if it is <strong>right for this line</strong> (✓), again if it is <strong>wrong</strong> (✗), again to clear it. "(no answer)" means a candidate failed or gave nothing for the line; it is counted apart and cannot be marked. Your marks are kept in this browser; <em>Save marks</em> downloads them as <code>marks.json</code>.</p>
<div id="lines"></div>
</main>
<script>
const DATA = __DATA__;
const STORE = 'maru-sentence-review-' + __STAMP__;
let marks = {};
// A browser that refuses storage still shows the page; the marks then live until it is closed.
try { marks = JSON.parse(localStorage.getItem(STORE) || '{}'); } catch (e) {}
const keep = () => { try { localStorage.setItem(STORE, JSON.stringify(marks)); } catch (e) {} };
const keyOf = (line, letter) => line + '|' + letter;
const el = (tag, attrs = {}, ...kids) => { const e = document.createElement(tag); for (const [k, v] of Object.entries(attrs)) { if (k === 'text') e.textContent = v; else e.setAttribute(k, v); } e.append(...kids.filter(Boolean)); return e; };
function paint(button, state) { button.classList.toggle('right', state === 'right'); button.classList.toggle('wrong', state === 'wrong'); }
function count() {
  let total = 0, done = 0;
  for (const line of DATA) for (const c of line.choices) if (c.text != null) { total++; if (marks[keyOf(line.line, c.letter)]) done++; }
  document.getElementById('count').textContent = done + ' of ' + total + ' translations marked';
}
function draw() {
  const root = document.getElementById('lines');
  root.replaceChildren();
  for (const line of DATA) {
    const sec = el('section', {}, el('h2', { class: 'ko' }, el('small', { text: 'Line ' + line.line }), document.createTextNode(line.text)));
    for (const c of line.choices) {
      const button = el('button', { class: 'choice' + (c.text == null ? ' none' : '') }, el('b', { text: c.letter }), el('i'), el('span', { text: c.text == null ? '(no answer)' : c.text }));
      if (c.text == null) button.disabled = true;
      else {
        const k = keyOf(line.line, c.letter);
        paint(button, marks[k]);
        button.addEventListener('click', () => {
          const next = marks[k] === 'right' ? 'wrong' : marks[k] === 'wrong' ? undefined : 'right';
          if (next) marks[k] = next; else delete marks[k];
          keep();
          paint(button, next);
          count();
        });
      }
      sec.append(button);
    }
    root.append(sec);
  }
  count();
}
document.getElementById('save').addEventListener('click', () => {
  const blob = new Blob([JSON.stringify({ stamp: __STAMP__, marks }, null, 1)], { type: 'application/json' });
  const a = el('a', { href: URL.createObjectURL(blob), download: 'marks.json' });
  document.body.append(a); a.click(); a.remove();
});
document.getElementById('clear').addEventListener('click', () => { if (confirm('Clear every mark on this page?')) { marks = {}; try { localStorage.removeItem(STORE); } catch (e) {} draw(); } });
draw();
</script></body></html>
"""


def build(lines, translations, seed):
    """`(page html, key)`. The page names no candidate; the key maps each line's letters to them."""
    key, shown = assign(translations, seed)
    data = [
        {"line": n, "text": text, "choices": [{"letter": letter, "text": t} for letter, t in shown[n - 1].items()]}
        for n, text in enumerate(lines, 1)
    ]
    body = json.dumps(data, ensure_ascii=False)
    # The stamp ties the marks to this very page: marks saved from another build are refused by `score`.
    stamp = f"seed-{seed}-lines-{len(lines)}-{hashlib.sha256(body.encode('utf-8')).hexdigest()[:12]}"
    page = PAGE.replace("__DATA__", body.replace("</", "<\\/")).replace("__STAMP__", json.dumps(stamp))
    return page, {"stamp": stamp, "seed": seed, "candidates": sorted(translations), "lines": key}


def score(saved, key):
    """Per candidate: translations marked right, wrong, left unmarked, and lines it gave no answer for. No ranking."""
    if saved.get("stamp") != key.get("stamp"):
        raise ValueError("these marks were saved from another page than the one this key belongs to")
    marks = saved["marks"]
    tally = {name: {"right": 0, "wrong": 0, "not_marked": 0, "no_answer": 0} for name in key["candidates"]}
    for line in key["lines"]:
        for letter, names in line["letters"].items():
            if letter == line["no_answer"]:
                what = "no_answer"
            else:
                mark = marks.get(f"{line['line']}|{letter}")
                what = mark if mark in ("right", "wrong") else "not_marked"
            for name in names:
                tally[name][what] += 1
    return tally


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    b = sub.add_parser("build")
    b.add_argument("lines")
    b.add_argument("--candidate", nargs="+", required=True, metavar="NAME=DIR")
    b.add_argument("--out", required=True)
    b.add_argument("--key", required=True)
    b.add_argument("--seed", type=int, default=1)
    s = sub.add_parser("score")
    s.add_argument("marks")
    s.add_argument("key")
    args = parser.parse_args(argv)
    if args.command == "score":
        try:
            tally = score(read_json(args.marks), read_json(args.key))
        except ValueError as e:
            sys.exit(str(e))
        for name, t in sorted(tally.items()):
            print(f"{name}: {t['right']} right, {t['wrong']} wrong, {t['not_marked']} not marked, {t['no_answer']} no answer")
        return
    lines = read_lines(args.lines)
    translations = {}
    for given in args.candidate:
        name, _, path = given.partition("=")
        if not name or not path or name in translations:
            sys.exit(f"--candidate takes NAME=DIR with a different NAME each time, not {given!r}")
        if os.path.isdir(path):
            path = os.path.join(path, "live-bench.jsonl")
        try:
            translations[name] = answers(load(path), lines)
        except (OSError, ValueError) as e:
            sys.exit(f"{name}: {e}")
    if len(translations) > len(string.ascii_uppercase):
        sys.exit("more candidates than letters")
    page, key = build(lines, translations, args.seed)
    write(args.out, page)
    write(args.key, json.dumps(key, ensure_ascii=False, indent=1))
    for name in translations:
        print(f"{name}: {sum(t is not None for t in translations[name])} of {len(lines)} lines answered")
    print(f"{len(lines)} lines, {len(translations)} candidates -> {args.out}; key (keep it apart until the review is done) -> {args.key}")


if __name__ == "__main__":
    main(sys.argv[1:])
