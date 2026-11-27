"""Build the review page for the analyser's dictionary forms, and score it once it has been marked.

Kiwi turns every word of a line into a dictionary form (its lemma); the cards and the dictionary look-ups depend
on it. This puts its analysis of a set of lines in front of a person, one row per word: the word as written, the
lemma, the part of speech and the morphemes it was split into. The lines are shown in three groups (standard
text, dialect, broken OCR) because the analyser is expected to do very differently on each. The page judges nothing.

  python tools/build_kiwi_review.py build tokens.jsonl --groups groups.json --out index.html [--known known.json]
  python tools/build_kiwi_review.py score marks.json
  python tools/build_kiwi_review.py changes standard.jsonl "Gyeongsang dialect=gyeongsang.jsonl" --groups groups.json --out changes.html

`changes` lists only the words whose dictionary form differs between Kiwi as Maru runs it and Kiwi with an option on
(the files `cargo run -p maru-cli --example kiwi_options` writes), in the same three groups, and judges nothing. An
option given as `"name=!reason"` is shown as not measured, with the reason.

`tokens.jsonl`: one object per line with `line_no`, `line` and `eojeols` (`surface`, `lemma`, `pos`, `content`,
`morphs`), as `maru translate --no-llm --json` gives them in its `breakdown` event.
`groups.json`: `{"lines": {"1": {"group": "dialect", "why": "…"}, …}}`, a group for every line. Whoever made it
is said in the page; to move a line, change its group there and build again (marks are kept: they go by line and word).
`known.json` (optional): `[{"surface": "…", "lemma": "…", "note": "reported wrong in run 1"}]`; a row whose word
and lemma are both the same is shown with that note.

On the page, ✓ marks a row right and ✗ wrong (again to clear); a wrong row takes the correct dictionary form in a
small box. Marks stay in the browser; "Save marks" downloads them as `marks.json` for `score`.
"""
import argparse
import json
import re
import sys
import zlib
from collections import Counter

GROUPS = ("standard text", "dialect", "broken OCR")


def load(path):
    with open(path, encoding="utf-8") as lines:
        return [json.loads(line) for line in lines if line.strip()]


def load_json(path):
    with open(path, encoding="utf-8") as whole:
        return json.load(whole)


def bare(surface):
    """A word without the punctuation around it: `주다,` and `주다` are the same word."""
    return re.sub(r"^\W+|\W+$", "", surface or "")


def rows(tokens, groups, known=()):
    """The page's data: the three groups in order, each with its lines and one row per word."""
    notes = {(k["surface"], k["lemma"]): k.get("note", "known failure") for k in known}
    placed = groups.get("lines", {})
    missing = [str(t["line_no"]) for t in tokens if placed.get(str(t["line_no"]), {}).get("group") not in GROUPS]
    if missing:
        sys.exit(f"no group (one of {', '.join(GROUPS)}) for line {', '.join(missing)} in the groups file")
    out = [{"name": name, "lines": []} for name in GROUPS]
    for t in tokens:
        place = placed[str(t["line_no"])]
        words = [
            {
                "index": e["index"],
                "surface": e["surface"],
                "lemma": e.get("lemma") or "",
                "pos": e.get("pos") or "other",
                # Words the app makes no card for (a bare particle, a symbol) still get a row: leaving one out is a decision too.
                "content": bool(e.get("content")),
                "morphs": [{"form": m["form"], "tag": m["tag"], "label": m.get("label", "")} for m in e.get("morphs", [])],
                "known": notes.get((bare(e["surface"]), e.get("lemma") or ""), ""),
            }
            for e in t["eojeols"]
        ]
        out[GROUPS.index(place["group"])]["lines"].append({"line": t["line_no"], "text": t["line"], "why": place.get("why", ""), "rows": words})
    return out


def totals(data):
    """How many rows each group and each part of speech has, so that `score` can say how many were left unmarked."""
    group, pos = Counter(), Counter()
    for g in data:
        for line in g["lines"]:
            for row in line["rows"]:
                group[g["name"]] += 1
                pos[row["pos"]] += 1
    return {"group": dict(group), "pos": dict(pos)}


PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Kiwi lemmas: review</title>
<style>
:root { color-scheme: light dark; --bg:#f6f7f9; --card:#fff; --fg:#1d2330; --muted:#667085; --line:#d9dde6; --right:#d7f2e2; --wrong:#fbdcdc; --accent:#2f5fd0; --known:#b45f06; --known-bg:#fff1d6; }
@media (prefers-color-scheme: dark) { :root { --bg:#14171d; --card:#1c2029; --fg:#e7e9ee; --muted:#98a1b3; --line:#303746; --right:#1f4a36; --wrong:#5a2626; --accent:#86a8ff; --known:#f0b45a; --known-bg:#4a3a1c; } }
* { box-sizing: border-box; }
body { margin: 0; font: 15px/1.5 system-ui, "Segoe UI", "Malgun Gothic", sans-serif; background: var(--bg); color: var(--fg); }
header { position: sticky; top: 0; z-index: 2; background: var(--card); border-bottom: 1px solid var(--line); padding: 10px 20px; display: flex; gap: 14px; align-items: center; flex-wrap: wrap; }
header h1 { font-size: 17px; margin: 0; }
header .count { color: var(--muted); font-variant-numeric: tabular-nums; }
header nav { display: flex; gap: 10px; flex-wrap: wrap; }
a { color: var(--accent); }
button { font: inherit; padding: 6px 14px; border-radius: 8px; border: 1px solid var(--line); background: var(--card); color: var(--fg); cursor: pointer; }
button.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
main { max-width: 1100px; margin: 0 auto; padding: 16px 20px 80px; }
.how { color: var(--muted); max-width: 95ch; }
.how code { font-size: 13px; overflow-wrap: anywhere; }
.auto { border-left: 4px solid var(--known); background: var(--known-bg); color: var(--fg); padding: 8px 12px; border-radius: 6px; max-width: 95ch; }
h2 { font-size: 20px; margin: 34px 0 4px; scroll-margin-top: 70px; }
h2 small { color: var(--muted); font-weight: 400; font-size: 14px; margin-left: 8px; font-variant-numeric: tabular-nums; }
section { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 12px 16px; margin: 12px 0; }
.ko { font-family: "Malgun Gothic", "Noto Sans KR", sans-serif; }
h3 { font-size: 19px; margin: 0 0 2px; font-weight: 600; overflow-wrap: anywhere; }
h3 small { color: var(--muted); font-weight: 400; font-size: 13px; margin-right: 8px; }
.why { color: var(--muted); font-size: 13px; margin: 0 0 8px; }
.scroll { overflow-x: auto; }
table { border-collapse: collapse; width: 100%; }
th, td { border: 1px solid var(--line); padding: 6px 8px; text-align: left; vertical-align: middle; }
th { font-weight: 600; background: color-mix(in srgb, var(--card) 85%, var(--fg)); white-space: nowrap; font-size: 13px; }
tr { scroll-margin-top: 80px; }
td.surface, td.lemma { font-size: 17px; white-space: nowrap; }
td.lemma { font-weight: 600; }
td.lemma.none { color: var(--muted); font-weight: 400; font-size: 13px; font-style: italic; }
td.pos { color: var(--muted); white-space: nowrap; }
td.morphs span { display: inline-block; margin: 1px 6px 1px 0; white-space: nowrap; }
td.morphs small { color: var(--muted); margin-left: 2px; }
td.mark { white-space: nowrap; width: 1%; }
td.mark button { padding: 2px 10px; font-weight: 700; min-width: 38px; }
td.mark input { font: inherit; width: 9em; margin-left: 6px; padding: 2px 6px; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); color: var(--fg); display: none; }
tr.right td { background: var(--right); }
tr.wrong td { background: var(--wrong); }
tr.right button.yes, tr.wrong button.no { background: var(--fg); color: var(--card); border-color: var(--fg); }
tr.wrong td.mark input { display: inline-block; }
tr.known td { border-top: 2px solid var(--known); border-bottom: 2px solid var(--known); }
tr.known td.surface { box-shadow: inset 5px 0 0 var(--known); padding-left: 13px; }
.badge { display: inline-block; margin-left: 8px; padding: 0 8px; border-radius: 10px; background: var(--known-bg); color: var(--known); border: 1px solid var(--known); font-size: 12px; font-weight: 600; white-space: nowrap; vertical-align: middle; }
</style></head><body>
<header><h1>Kiwi lemmas: review</h1><span class="count" id="count"></span>
<nav id="nav"></nav>
<button class="primary" id="save">Save marks</button><button id="clear">Clear all marks</button></header>
<main>
<p class="how">Every word of every line is a row: the word as it is written, the dictionary form (lemma) Kiwi gave it, its part of speech and the morphemes Kiwi split it into. Press <strong>✓</strong> if the lemma is the dictionary form you would look the word up under, <strong>✗</strong> if it is not (press again to clear); a row marked ✗ shows a small box for the correct dictionary form. A row that says "no lemma" is a word the app makes no card for: mark it ✗ if it should have had one. Your marks are kept in this browser; <em>Save marks</em> downloads them as <code>marks.json</code>, and <code>python tools/build_kiwi_review.py score marks.json</code> counts them per group and per part of speech.</p>
<p class="auto"><strong>The grouping was made automatically.</strong> __BY__ To move a line, change its <code>"group"</code> in <code>__GROUPS_FILE__</code> (next to this page) to one of <code>standard text</code>, <code>dialect</code>, <code>broken OCR</code> and build the page again; marks already made are kept.</p>
<p class="how" id="known"></p>
<div id="groups"></div>
</main>
<script>
const DATA = __DATA__;
const TOTALS = __TOTALS__;
const STAMP = __STAMP__;
const STORE = 'maru-kiwi-review-' + STAMP;
// ?demo=marks shows a few rows already marked, to look at the page without touching the saved marks.
const DEMO = new URLSearchParams(location.search).get('demo') === 'marks';
let marks = {};
try { if (!DEMO) marks = JSON.parse(localStorage.getItem(STORE) || '{}'); } catch (e) { marks = {}; }
const keyOf = (line, index) => line + '|' + index;
const el = (tag, attrs = {}, ...kids) => { const e = document.createElement(tag); for (const [k, v] of Object.entries(attrs)) { if (k === 'text') e.textContent = v; else e.setAttribute(k, v); } e.append(...kids.filter(Boolean)); return e; };
const all = () => DATA.flatMap((g) => g.lines.flatMap((line) => line.rows.map((row) => ({ g, line, row }))));
if (DEMO) { const some = all(); if (some[0]) marks[keyOf(some[0].line.line, some[0].row.index)] = { mark: 'right', correct: '' }; if (some[1]) marks[keyOf(some[1].line.line, some[1].row.index)] = { mark: 'wrong', correct: '(demo)' }; }
function keep() { if (DEMO) return; try { localStorage.setItem(STORE, JSON.stringify(marks)); } catch (e) { /* a private window: the marks live until the page closes */ } }
function count() {
  const rows = all();
  const done = rows.filter(({ line, row }) => marks[keyOf(line.line, row.index)]).length;
  document.getElementById('count').textContent = done + ' of ' + rows.length + ' rows marked';
  for (const g of DATA) {
    const mine = g.lines.flatMap((line) => line.rows.map((row) => marks[keyOf(line.line, row.index)]));
    const right = mine.filter((m) => m && m.mark === 'right').length, wrong = mine.filter((m) => m && m.mark === 'wrong').length;
    document.getElementById('tally-' + DATA.indexOf(g)).textContent = g.lines.length + ' lines, ' + mine.length + ' rows: ' + right + ' ✓, ' + wrong + ' ✗, ' + (mine.length - right - wrong) + ' to do';
  }
}
function paint(tr, input, state) {
  tr.classList.toggle('right', !!state && state.mark === 'right');
  tr.classList.toggle('wrong', !!state && state.mark === 'wrong');
  if (document.activeElement !== input) input.value = state ? state.correct || '' : '';
}
function draw() {
  const root = document.getElementById('groups'), nav = document.getElementById('nav'), known = [];
  root.replaceChildren(); nav.replaceChildren();
  DATA.forEach((g, n) => {
    nav.append(el('a', { href: '#group-' + n, text: g.name }));
    root.append(el('h2', { id: 'group-' + n }, document.createTextNode(g.name), el('small', { id: 'tally-' + n })));
    if (!g.lines.length) root.append(el('p', { class: 'how', text: 'No line is in this group.' }));
    for (const line of g.lines) {
      const table = el('table', {}, el('tr', {}, ...['Written', "Kiwi's lemma", 'Part of speech', 'Morphemes', 'Right?'].map((h) => el('th', { text: h }))));
      for (const row of line.rows) {
        const key = keyOf(line.line, row.index);
        const input = el('input', { type: 'text', placeholder: 'correct form', 'aria-label': 'Correct dictionary form', class: 'ko' });
        const yes = el('button', { class: 'yes', title: 'The lemma is right', text: '✓' }), no = el('button', { class: 'no', title: 'The lemma is wrong', text: '✗' });
        const tr = el('tr', { id: 'row-' + line.line + '-' + row.index, class: row.known ? 'known' : '' },
          el('td', { class: 'surface ko' }, document.createTextNode(row.surface), row.known && el('span', { class: 'badge', text: row.known })),
          el('td', { class: 'lemma ko' + (row.lemma ? '' : ' none'), text: row.lemma || 'no lemma' }),
          el('td', { class: 'pos', text: row.pos + (row.content ? '' : ' · no card') }),
          el('td', { class: 'morphs ko' }, ...row.morphs.map((m) => el('span', { title: m.label }, document.createTextNode(m.form), el('small', { text: m.tag })))),
          el('td', { class: 'mark' }, yes, no, input));
        const set = (mark) => { if (marks[key] && marks[key].mark === mark) delete marks[key]; else marks[key] = { mark, correct: mark === 'wrong' ? input.value : '' }; keep(); paint(tr, input, marks[key]); count(); };
        yes.addEventListener('click', () => set('right'));
        no.addEventListener('click', () => { set('wrong'); if (marks[key]) input.focus(); });
        input.addEventListener('input', () => { if (marks[key]) { marks[key].correct = input.value; keep(); } });
        paint(tr, input, marks[key]);
        table.append(tr);
        if (row.known) known.push(el('a', { href: '#row-' + line.line + '-' + row.index, class: 'ko', text: 'line ' + line.line + ': ' + row.surface + ' → ' + (row.lemma || 'no lemma') }));
      }
      root.append(el('section', {}, el('h3', { class: 'ko' }, el('small', { text: 'Line ' + line.line }), document.createTextNode(line.text)), line.why && el('p', { class: 'why', text: 'In this group because: ' + line.why }), el('div', { class: 'scroll' }, table)));
    }
  });
  const list = document.getElementById('known');
  list.replaceChildren();
  if (known.length) { list.append(document.createTextNode('Rows with an orange badge were already reported wrong: ')); known.forEach((a, n) => list.append(n ? document.createTextNode(' · ') : '', a)); }
  count();
}
document.getElementById('save').addEventListener('click', () => {
  const saved = {};
  for (const { g, line, row } of all()) { const m = marks[keyOf(line.line, row.index)]; if (m) saved[keyOf(line.line, row.index)] = { mark: m.mark, correct: (m.correct || '').trim(), group: g.name, pos: row.pos, surface: row.surface, lemma: row.lemma }; }
  const blob = new Blob([JSON.stringify({ stamp: STAMP, totals: TOTALS, marks: saved }, null, 1)], { type: 'application/json' });
  const a = el('a', { href: URL.createObjectURL(blob), download: 'marks.json' });
  document.body.append(a); a.click(); a.remove();
});
document.getElementById('clear').addEventListener('click', () => { if (confirm('Clear every mark on this page?')) { marks = {}; try { localStorage.removeItem(STORE); } catch (e) { /* nothing was stored */ } draw(); } });
draw();
</script></body></html>
"""


def build(tokens, groups, known=(), groups_file="groups.json"):
    """The page's html. Its marks are stored under a stamp made from the lines and words alone, so that moving a
    line to another group keeps them."""
    data = rows(tokens, groups, known)
    said = json.dumps([[t["line_no"], t["line"], [(e["surface"], e.get("lemma")) for e in t["eojeols"]]] for t in tokens], ensure_ascii=False)
    stamp = f"lines-{len(tokens)}-{zlib.crc32(said.encode('utf-8')):08x}"
    inline = lambda value: json.dumps(value, ensure_ascii=False).replace("</", "<\\/")
    by = groups.get("made_by", "Nobody has said who made it.")
    page = PAGE.replace("__DATA__", inline(data)).replace("__TOTALS__", inline(totals(data))).replace("__STAMP__", inline(stamp))
    return page.replace("__BY__", escape(by)).replace("__GROUPS_FILE__", escape(groups_file))


def escape(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def score(saved):
    """Per group and per part of speech: rows marked right, wrong and left unmarked. No verdict."""
    out = {"group": {}, "pos": {}, "corrections": 0}
    for kind in ("group", "pos"):
        names = list(saved.get("totals", {}).get(kind, {})) + [m[kind] for m in saved["marks"].values()]
        for name in dict.fromkeys(names):
            mine = [m["mark"] for m in saved["marks"].values() if m[kind] == name]
            tally = {"right": mine.count("right"), "wrong": mine.count("wrong")}
            total = saved.get("totals", {}).get(kind, {}).get(name)
            if total is not None:
                tally["not marked"] = total - tally["right"] - tally["wrong"]
            out[kind][name] = tally
    out["corrections"] = sum(1 for m in saved["marks"].values() if m["mark"] == "wrong" and m.get("correct"))
    return out


def changed_rows(base, other, groups, known=()):
    """`[(group, line_no, line, surface, lemma before, lemma after, note)]` for every word whose lemma differs."""
    notes = {(k["surface"], k["lemma"]): k.get("note", "known failure") for k in known}
    placed = groups.get("lines", {})
    after = {t["line_no"]: t for t in other}
    out = []
    for t in base:
        o = after.get(t["line_no"])
        if o is None:
            continue
        for e, f in zip(t["eojeols"], o["eojeols"]):
            before, now = e.get("lemma") or "", f.get("lemma") or ""
            if e["surface"] == f["surface"] and before != now:
                group = placed.get(str(t["line_no"]), {}).get("group", "standard text")
                out.append((group, t["line_no"], t["line"], e["surface"], before, now, notes.get((bare(e["surface"]), before), "")))
    return sorted(out, key=lambda r: (GROUPS.index(r[0]) if r[0] in GROUPS else 9, r[1]))


CHANGES_PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Kiwi options: what changes</title>
<style>
 body{{margin:0;font:15px/1.5 system-ui,"Segoe UI","Malgun Gothic",sans-serif;background:#f6f6f4;color:#1d1d1b}}
 main{{max-width:900px;margin:0 auto;padding:16px}} h1{{font-size:19px;margin:0 0 6px}} h2{{font-size:16px;margin:22px 0 6px}}
 h3{{font-size:14px;color:#6b6b66;margin:14px 0 4px;text-transform:uppercase;letter-spacing:.04em}}
 p{{color:#4a4a45;margin:4px 0 10px}} table{{width:100%;border-collapse:collapse;background:#fff;border:1px solid #dcdcd6;border-radius:8px}}
 td,th{{padding:6px 10px;border-bottom:1px solid #ecece6;text-align:left;vertical-align:top}} th{{font-size:12px;color:#6b6b66;font-weight:600}}
 .ko{{font-family:"Malgun Gothic","Noto Sans CJK KR",sans-serif;font-size:16px}} .line{{color:#6b6b66;font-size:13px}}
 .note{{color:#b4540a;font-size:12px}} .none{{color:#6b6b66;font-style:italic}}
 button{{font:inherit;border:1px solid #dcdcd6;background:#fff;border-radius:6px;padding:2px 8px;cursor:pointer}} button.on{{background:#2f5fd0;color:#fff;border-color:#2f5fd0}}
 @media (prefers-color-scheme: dark){{body{{background:#151514;color:#ecece8}} table{{background:#1f1f1d;border-color:#34342f}} td,th{{border-color:#2a2a26}} p,.line,h3,th{{color:#9a9a93}} button{{background:#1f1f1d;color:#ecece8;border-color:#34342f}}}}
</style></head><body><main>
<h1>Kiwi options: which dictionary forms change</h1>
<p>{intro}</p>
{sections}
<p><button id="save">Save marks</button> downloads what you marked as <code>kiwi-option-marks.json</code>. Marks stay in this browser until then.</p>
</main>
<script>
const KEY = 'maru-kiwi-option-marks';
const marks = JSON.parse(localStorage.getItem(KEY) || '{{}}');
for (const b of document.querySelectorAll('button[data-row]')) {{
  const paint = () => b.classList.toggle('on', marks[b.dataset.row] === b.dataset.say);
  paint();
  b.addEventListener('click', () => {{
    marks[b.dataset.row] = marks[b.dataset.row] === b.dataset.say ? undefined : b.dataset.say;
    localStorage.setItem(KEY, JSON.stringify(marks));
    for (const o of document.querySelectorAll(`button[data-row="${{b.dataset.row}}"]`)) o.classList.toggle('on', marks[o.dataset.row] === o.dataset.say);
  }});
}}
document.getElementById('save').addEventListener('click', () => {{
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([JSON.stringify(marks, null, 1)], {{type: 'application/json'}}));
  a.download = 'kiwi-option-marks.json'; a.click();
}});
</script></body></html>
"""


def changes_page(base, options, groups, known=()):
    """The page: per option, the changed words by group; or why the option could not be measured."""
    parts = []
    for name, other in options:
        parts.append(f"<h2>{escape(name)}</h2>")
        if isinstance(other, str):
            parts.append(f'<p class="none">Not measured: {escape(other)}</p>')
            continue
        rows_ = changed_rows(base, other, groups, known)
        counts = {g: sum(1 for r in rows_ if r[0] == g) for g in GROUPS}
        parts.append("<p>" + ", ".join(f"{g}: {counts[g]}" for g in GROUPS) + f" words changed, of {sum(len(t['eojeols']) for t in base)}.</p>")
        for group in GROUPS:
            mine = [r for r in rows_ if r[0] == group]
            if not mine:
                continue
            parts.append(f"<h3>{group}</h3><table><tr><th>Line</th><th>Word</th><th>Kiwi today</th><th>With the option</th><th>Which is right?</th></tr>")
            for _, no, line, surface, before, now, note in mine:
                row = f"{name}|{no}|{surface}"
                buttons = "".join(f'<button data-row="{escape(row)}" data-say="{say}">{label}</button> ' for say, label in (("today", "today"), ("option", "with option"), ("neither", "neither")))
                extra = f'<div class="note">{escape(note)}</div>' if note else ""
                parts.append(f'<tr><td class="line">{no}<br><span class="ko">{escape(line)}</span></td><td class="ko">{escape(surface)}</td><td class="ko">{escape(before) or "—"}</td><td class="ko">{escape(now) or "—"}</td><td>{buttons}{extra}</td></tr>')
            parts.append("</table>")
    intro = ("Kiwi analysed the same lines twice: as Maru runs it, and with an option on. Only the words whose dictionary form "
             "changed are listed, in the groups of the run 2 page (sorted by the assistant, not checked by a person). Nothing here is judged.")
    return CHANGES_PAGE.format(intro=escape(intro), sections="\n".join(parts))


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    b = sub.add_parser("build")
    b.add_argument("tokens")
    b.add_argument("--groups", required=True)
    b.add_argument("--out", required=True)
    b.add_argument("--known")
    s = sub.add_parser("score")
    s.add_argument("marks")
    c = sub.add_parser("changes")
    c.add_argument("base")
    c.add_argument("options", nargs="+", help='"name=file.jsonl", or "name=!why it was not measured"')
    c.add_argument("--groups", required=True)
    c.add_argument("--out", required=True)
    c.add_argument("--known")
    args = parser.parse_args(argv)
    if args.command == "changes":
        options = []
        for spec in args.options:
            name, _, value = spec.partition("=")
            options.append((name, value[1:] if value.startswith("!") else load(value)))
        page = changes_page(load(args.base), options, load_json(args.groups), load_json(args.known) if args.known else [])
        with open(args.out, "w", encoding="utf-8", newline="\n") as out:
            out.write(page)
        print(f"-> {args.out}")
        return
    if args.command == "score":
        result = score(load_json(args.marks))
        for kind, title in (("group", "By group"), ("pos", "By part of speech")):
            print(title)
            for name, tally in result[kind].items():
                print(f"  {name}: " + ", ".join(f"{count} {what}" for what, count in tally.items()))
        print(f"Wrong rows with a correct form written in: {result['corrections']}")
        return
    tokens = load(args.tokens)
    groups = load_json(args.groups)
    known = load_json(args.known) if args.known else []
    page = build(tokens, groups, known, groups_file=re.split(r"[\\/]", args.groups)[-1])
    with open(args.out, "w", encoding="utf-8", newline="\n") as out:
        out.write(page)
    data = rows(tokens, groups, known)
    counts = totals(data)["group"]
    flagged = sum(1 for g in data for line in g["lines"] for row in line["rows"] if row["known"])
    print(f"{len(tokens)} lines -> {args.out}; rows: " + ", ".join(f"{name} {counts.get(name, 0)}" for name in GROUPS) + f"; flagged as known failures: {flagged}")


if __name__ == "__main__":
    main(sys.argv[1:])
