"""Build the blind review page for card meanings, and score it once it has been marked.

`maru bench --cards` (and `tools/rerank_senses.py`) write, per candidate, the English and Spanish meaning each one
decided for every word of every line. This puts them side by side for a person to judge without knowing which
candidate wrote what: in each line the candidates are called A, B, C… in a different random order, and the key that
says who is who is written to a separate file. The page chooses no winner.

  python tools/build_card_review.py build tasks.jsonl --cards a.jsonl b.jsonl … --out review.html --key key.json
         [--translations lines.jsonl] [--seed 1]
  python tools/build_card_review.py score marks.json key.json

On the page, click a meaning to mark it right (✓), click again for wrong (✗), again to clear; equal meanings in a
row are marked together. Marks stay in the browser; "Save marks" downloads them as `marks.json` for `score`.
"""
import argparse
import html
import json
import random
import string
import sys


def load(path):
    return [json.loads(line) for line in open(path, encoding="utf-8") if line.strip()]


def assign(candidates, lines, seed):
    """For every line, the candidates under letters in a random order of that line's own."""
    rng = random.Random(seed)
    key = []
    for n in range(1, lines + 1):
        order = list(candidates)
        rng.shuffle(order)
        key.append({"line": n, "letters": {string.ascii_uppercase[i]: name for i, name in enumerate(order)}})
    return key


def meaning(record, word, asked):
    """What a candidate shows for a word: its two meanings, or that it gave no answer."""
    if record is None:
        return None
    found = next((w for w in record["words"] if w["index"] == word["index"]), None)
    # A request that failed leaves the dictionary's first sense: that is not this candidate's choice.
    if record.get("error") and asked:
        return None
    if found is None:
        return None
    return " · ".join(part for part in (found["en"], found["es"]) if part)


def rows(tasks, records, key):
    """The page's data: per line, the words a decision was needed for, with each letter's meaning."""
    by = {(r["candidate"], r["line"]): r for r in records}
    out = []
    for n, task in enumerate(tasks, 1):
        letters = key[n - 1]["letters"]
        words, settled = [], []
        for w in task["words"]:
            if len(w["senses"]) == 1:
                settled.append({"lemma": w["lemma"], "meaning": " · ".join(p for p in (w["senses"][0]["en"], w["senses"][0]["es"]) if p)})
                continue
            cells = {letter: meaning(by.get((name, n)), w, True) for letter, name in letters.items()}
            words.append({
                "index": w["index"], "surface": w["surface"], "lemma": w["lemma"], "pos": w["pos"],
                "senses": [" · ".join(p for p in (s["en"], s["es"]) if p) for s in w["senses"]],
                "cells": cells,
            })
        out.append({"line": n, "text": task["line"], "words": words, "settled": settled, "letters": sorted(letters)})
    return out


PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Card meanings: blind review</title>
<style>
:root { color-scheme: light dark; --bg:#f6f7f9; --card:#fff; --fg:#1d2330; --muted:#667085; --line:#d9dde6; --right:#d7f2e2; --wrong:#fbdcdc; --accent:#2f5fd0; }
@media (prefers-color-scheme: dark) { :root { --bg:#14171d; --card:#1c2029; --fg:#e7e9ee; --muted:#98a1b3; --line:#303746; --right:#1f4a36; --wrong:#5a2626; --accent:#86a8ff; } }
* { box-sizing: border-box; }
body { margin: 0; font: 15px/1.5 system-ui, "Segoe UI", "Malgun Gothic", sans-serif; background: var(--bg); color: var(--fg); }
header { position: sticky; top: 0; z-index: 2; background: var(--card); border-bottom: 1px solid var(--line); padding: 10px 20px; display: flex; gap: 16px; align-items: center; flex-wrap: wrap; }
header h1 { font-size: 17px; margin: 0; }
header .count { color: var(--muted); font-variant-numeric: tabular-nums; }
button { font: inherit; padding: 6px 14px; border-radius: 8px; border: 1px solid var(--line); background: var(--card); color: var(--fg); cursor: pointer; }
button.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
main { max-width: 1500px; margin: 0 auto; padding: 16px 20px 80px; }
.how { color: var(--muted); max-width: 90ch; }
section { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 14px 16px; margin: 16px 0; }
.ko { font-family: "Malgun Gothic", "Noto Sans KR", sans-serif; }
h2 { font-size: 19px; margin: 0 0 2px; font-weight: 600; }
h2 small { color: var(--muted); font-weight: 400; font-size: 13px; margin-right: 8px; }
.tr { color: var(--muted); margin: 0 0 10px; }
.scroll { overflow-x: auto; }
table { border-collapse: collapse; width: 100%; }
th, td { border: 1px solid var(--line); padding: 6px 8px; text-align: left; vertical-align: top; }
th { font-weight: 600; background: color-mix(in srgb, var(--card) 85%, var(--fg)); white-space: nowrap; }
td.word { white-space: nowrap; }
td.word small { display: block; color: var(--muted); }
td.cell { cursor: pointer; min-width: 110px; }
td.cell.none { color: var(--muted); font-style: italic; cursor: default; }
td.cell.right { background: var(--right); }
td.cell.wrong { background: var(--wrong); }
td.cell.right::before { content: "✓ "; font-weight: 700; }
td.cell.wrong::before { content: "✗ "; font-weight: 700; }
td.cell.odd { outline: 2px solid color-mix(in srgb, var(--accent) 55%, transparent); outline-offset: -2px; }
details { color: var(--muted); font-size: 13px; }
.settled { color: var(--muted); font-size: 13px; margin: 8px 0 0; }
</style></head><body>
<header><h1>Card meanings: blind review</h1><span class="count" id="count"></span>
<button class="primary" id="save">Save marks</button><button id="clear">Clear all marks</button></header>
<main>
<p class="how">Each line shows the words whose meaning had to be chosen, and what each candidate chose, in English and Spanish. The candidates are called A, B, C… and <strong>the letters are shuffled in every line</strong>, so a letter is not the same candidate from one line to the next. Click a meaning once if it is <strong>right for this line</strong> (✓), again if it is <strong>wrong</strong> (✗), again to clear it; equal meanings in a row are marked together. An outline marks a meaning that differs from what most candidates chose. "no answer" means that candidate gave nothing for the word. Your marks are kept in this browser; <em>Save marks</em> downloads them as <code>marks.json</code>.</p>
<div id="lines"></div>
</main>
<script>
const DATA = __DATA__;
const STORE = 'maru-card-review-' + __STAMP__;
let marks = JSON.parse(localStorage.getItem(STORE) || '{}');
const keyOf = (line, index, letter) => line + '|' + index + '|' + letter;
const el = (tag, attrs = {}, ...kids) => { const e = document.createElement(tag); for (const [k, v] of Object.entries(attrs)) { if (k === 'text') e.textContent = v; else e.setAttribute(k, v); } e.append(...kids.filter(Boolean)); return e; };
function paint(td, state) { td.classList.toggle('right', state === 'right'); td.classList.toggle('wrong', state === 'wrong'); }
function count() {
  let total = 0, done = 0;
  for (const line of DATA) for (const w of line.words) for (const letter of line.letters) if (w.cells[letter] != null) { total++; if (marks[keyOf(line.line, w.index, letter)]) done++; }
  document.getElementById('count').textContent = done + ' of ' + total + ' meanings marked';
}
function draw() {
  const root = document.getElementById('lines');
  root.replaceChildren();
  for (const line of DATA) {
    const sec = el('section', {}, el('h2', { class: 'ko' }, el('small', { text: 'Line ' + line.line }), document.createTextNode(line.text)), line.translation && el('p', { class: 'tr', text: line.translation }));
    if (line.words.length) {
      const table = el('table', {}, el('tr', {}, el('th', { text: 'Word' }), ...line.letters.map((l) => el('th', { text: l }))));
      for (const w of line.words) {
        const tally = {};
        for (const letter of line.letters) { const m = w.cells[letter]; if (m != null) tally[m] = (tally[m] || 0) + 1; }
        const most = Object.entries(tally).sort((a, b) => b[1] - a[1])[0]?.[0];
        const tr = el('tr', {}, el('td', { class: 'word ko' }, document.createTextNode(w.lemma), el('small', { text: w.surface + ' · ' + w.pos }),
          w.senses.length ? el('details', {}, el('summary', { text: w.senses.length + ' dictionary senses' }), ...w.senses.map((s, n) => el('div', { text: (n + 1) + '. ' + s }))) : el('details', {}, el('summary', { text: 'not in the dictionary' }))));
        for (const letter of line.letters) {
          const m = w.cells[letter];
          const td = el('td', { class: 'cell' + (m == null ? ' none' : '') + (m != null && m !== most ? ' odd' : ''), text: m == null ? 'no answer' : m });
          if (m != null) {
            paint(td, marks[keyOf(line.line, w.index, letter)]);
            td.addEventListener('click', () => {
              const now = marks[keyOf(line.line, w.index, letter)];
              const next = now === 'right' ? 'wrong' : now === 'wrong' ? undefined : 'right';
              for (const other of line.letters) if (w.cells[other] === m) { const k = keyOf(line.line, w.index, other); if (next) marks[k] = next; else delete marks[k]; }
              localStorage.setItem(STORE, JSON.stringify(marks));
              draw();
            });
          }
          tr.append(td);
        }
        table.append(tr);
      }
      sec.append(el('div', { class: 'scroll' }, table));
    }
    if (line.settled.length) sec.append(el('p', { class: 'settled ko', text: 'One dictionary sense, the same for every candidate: ' + line.settled.map((s) => s.lemma + ' — ' + s.meaning).join('; ') }));
    root.append(sec);
  }
  count();
}
document.getElementById('save').addEventListener('click', () => {
  const blob = new Blob([JSON.stringify({ stamp: __STAMP__, marks }, null, 1)], { type: 'application/json' });
  const a = el('a', { href: URL.createObjectURL(blob), download: 'marks.json' });
  document.body.append(a); a.click(); a.remove();
});
document.getElementById('clear').addEventListener('click', () => { if (confirm('Clear every mark on this page?')) { marks = {}; localStorage.removeItem(STORE); draw(); } });
draw();
</script></body></html>
"""


def build(tasks, records, candidates, seed, translations=None):
    """`(page html, key)`. The page names no candidate; the key maps each line's letters to them."""
    key = assign(candidates, len(tasks), seed)
    data = rows(tasks, records, key)
    for line in data:
        line["translation"] = (translations or {}).get(line["text"], "")
    stamp = json.dumps(f"seed-{seed}-lines-{len(tasks)}-candidates-{len(candidates)}")
    page = PAGE.replace("__DATA__", json.dumps(data, ensure_ascii=False).replace("</", "<\\/")).replace("__STAMP__", stamp)
    return page, {"seed": seed, "candidates": sorted(candidates), "lines": key}


def score(marks, key):
    """Per candidate: meanings marked right, wrong and left unmarked. No ranking."""
    letters = {line["line"]: line["letters"] for line in key["lines"]}
    tally = {name: {"right": 0, "wrong": 0} for name in key["candidates"]}
    for where, mark in marks.items():
        line, _, letter = where.split("|")
        name = letters[int(line)][letter]
        if mark in tally[name]:
            tally[name][mark] += 1
    return tally


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    b = sub.add_parser("build")
    b.add_argument("tasks")
    b.add_argument("--cards", nargs="+", required=True)
    b.add_argument("--out", required=True)
    b.add_argument("--key", required=True)
    b.add_argument("--translations")
    b.add_argument("--seed", type=int, default=1)
    s = sub.add_parser("score")
    s.add_argument("marks")
    s.add_argument("key")
    args = parser.parse_args(argv)
    if args.command == "score":
        tally = score(json.load(open(args.marks, encoding="utf-8"))["marks"], json.load(open(args.key, encoding="utf-8")))
        for name, t in sorted(tally.items()):
            print(f"{name}: {t['right']} right, {t['wrong']} wrong")
        return
    tasks = load(args.tasks)
    records = [r for path in args.cards for r in load(path)]
    candidates = list(dict.fromkeys(r["candidate"] for r in records))
    if len(candidates) > len(string.ascii_uppercase):
        sys.exit("more candidates than letters")
    translations = {t["line"]: t.get("translation", "") for t in load(args.translations)} if args.translations else None
    page, key = build(tasks, records, candidates, args.seed, translations)
    open(args.out, "w", encoding="utf-8", newline="\n").write(page)
    json.dump(key, open(args.key, "w", encoding="utf-8", newline="\n"), ensure_ascii=False, indent=1)
    print(f"{len(tasks)} lines, {len(candidates)} candidates -> {args.out}; key (keep it apart until the review is done) -> {args.key}")


if __name__ == "__main__":
    main(sys.argv[1:])
