// The decisions behind tools/label_review.html, kept free of the page so they can be tested (app/ui/test/label-review-core.test.mjs).
// It is a plain script in the browser (window.LabelCore) and a CommonJS module in Node.
(function (root) {
  'use strict';

  /** Letters and digits only: spaces and punctuation are not part of what the reader is judged on. */
  function normalize(text) {
    return String(text || '').replace(/[\s\p{P}\p{S}]/gu, '');
  }

  /** Edit distance between two strings, counted in characters (code points). */
  function distance(a, b) {
    const x = Array.from(a);
    const y = Array.from(b);
    let prev = Array.from({ length: y.length + 1 }, (_, j) => j);
    for (let i = 1; i <= x.length; i++) {
      const cur = [i];
      for (let j = 1; j <= y.length; j++) {
        cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x[i - 1] === y[j - 1] ? 0 : 1));
      }
      prev = cur;
    }
    return prev[y.length];
  }

  /** How different two readings are, 0 (same letters) to 1, ignoring spaces and punctuation. */
  function difference(a, b) {
    const x = normalize(a);
    const y = normalize(b);
    const longest = Math.max(Array.from(x).length, Array.from(y).length);
    return longest === 0 ? 0 : distance(x, y) / longest;
  }

  /** The largest difference between any two of the readings; null when there are fewer than two to compare. */
  function disagreement(texts) {
    const read = texts.filter((t) => normalize(t) !== '');
    if (read.length < 2) return null;
    let worst = 0;
    for (let i = 0; i < read.length; i++) for (let j = i + 1; j < read.length; j++) worst = Math.max(worst, difference(read[i], read[j]));
    return worst;
  }

  /** Where a picture with only one reading goes among those that disagree (above agreeing ones, below clear disagreements). */
  const UNKNOWN_WEIGHT = 0.05;

  /** The text has at least one Hangul syllable. */
  function hasHangul(text) {
    return /[\uac00-\ud7a3]/.test(String(text || ''));
  }

  /**
   * Every reading the page has for one picture: the live reader's draft first, then the other readers by name. A reading
   * from outside this PC without any Hangul is a comment, not a reading (run 3: "The strip cuts off the subtitle"), so it
   * is left out.
   */
  function readings(item) {
    const out = [];
    if (item.draft != null && String(item.draft).trim() !== '') out.push({ from: 'live', text: String(item.draft).trim() });
    for (const tag of Object.keys(item.drafts || {}).sort()) {
      const text = String(item.drafts[tag] || '').trim();
      if (text !== '' && !(tag === 'outside' && !hasHangul(text))) out.push({ from: tag, text });
    }
    return out;
  }

  /**
   * What the text box starts with: the label already written, else the reading of the strongest reader that has one.
   * A provisional label (two readers agreed, `tools/provisional_labels.py`) comes first, then a reader from outside this PC.
   * It is only a start; the person looks at the picture and decides.
   */
  function startingText(item, prefer = ['provisional', 'outside', 'paddle-vl', 'hybrid', 'live']) {
    if (item.label != null) return { text: item.label, from: 'label' };
    // A reading with no Hangul is never offered as a start (a stray "0 0" or a comment).
    const all = readings(item).filter((r) => hasHangul(r.text));
    for (const name of prefer) {
      const hit = all.find((r) => r.from === name);
      if (hit) return { text: hit.text, from: hit.from };
    }
    return all.length ? { text: all[0].text, from: all[0].from } : { text: '', from: 'none' };
  }

  /** Not done yet (no label, not marked unreadable or not a subtitle). */
  function pending(item) {
    return item.label == null && !item.unreadable && !item.notSubtitle;
  }

  /** Two readers agreed on it (`name.provisional.txt`), but no person has looked. */
  function provisional(item) {
    return Boolean(item.drafts && item.drafts.provisional);
  }

  /** Nothing at all yet: no label of a person's, no provisional one, no mark. */
  function unlabelled(item) {
    return pending(item) && !provisional(item);
  }

  /**
   * The order to work in: pictures still to do come before finished ones; with `disagreementsFirst`, those whose readers disagree
   * most come first, since that is where a draft is most likely to be wrong. Otherwise by name.
   */
  function order(items, { disagreementsFirst = true } = {}) {
    const weight = (item) => {
      const d = disagreement(readings(item).map((r) => r.text));
      return d == null ? UNKNOWN_WEIGHT : d;
    };
    return items
      .map((item, index) => ({ item, index, todo: pending(item) ? 0 : 1, w: disagreementsFirst ? weight(item) : 0 }))
      .sort((a, b) => a.todo - b.todo || b.w - a.w || (a.item.id < b.item.id ? -1 : a.item.id > b.item.id ? 1 : a.index - b.index))
      .map((e) => e.item);
  }

  /**
   * Sort the file names found in one folder into pictures and what sits beside them:
   * `a.png`, `a.json` (the live reader's draft and facts), `a.txt` (the label), `a.<reader>.txt` (another reader's draft),
   * `a.unreadable` (marked unusable), `a.not-subtitle` (not a subtitle: a map, a menu, a logo) and `a.tags` (tags the
   * person gave it).
   */
  function groupFiles(names) {
    const groups = new Map();
    const get = (id) => {
      if (!groups.has(id)) groups.set(id, { id, png: null, json: null, label: null, drafts: {}, unreadable: null, notSubtitle: null, tags: null });
      return groups.get(id);
    };
    for (const name of names) {
      let m;
      if ((m = /^([^.]+)\.(png|jpe?g|bmp)$/i.exec(name))) get(m[1]).png = name;
      else if ((m = /^([^.]+)\.json$/i.exec(name))) get(m[1]).json = name;
      else if ((m = /^([^.]+)\.txt$/i.exec(name))) get(m[1]).label = name;
      else if ((m = /^([^.]+)\.([a-z0-9-]+)\.txt$/i.exec(name))) get(m[1]).drafts[m[2].toLowerCase()] = name;
      else if ((m = /^([^.]+)\.unreadable$/i.exec(name))) get(m[1]).unreadable = name;
      else if ((m = /^([^.]+)\.not-subtitle$/i.exec(name))) get(m[1]).notSubtitle = name;
      else if ((m = /^([^.]+)\.tags$/i.exec(name))) get(m[1]).tags = name;
    }
    // A sidecar without a picture is of no use.
    return [...groups.values()].filter((g) => g.png).sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  }

  /**
   * A label as written to disk: each line with single spaces, empty lines dropped. Subtitles are scored on their letters
   * only, so their line breaks change nothing; a whole screen keeps one piece of text per line (`tools/scan_lines.py`).
   */
  function cleanLabel(text) {
    return String(text || '').split(/\r?\n/).map((l) => l.replace(/\s+/g, ' ').trim()).filter(Boolean).join('\n');
  }

  /** How the work is going. `edited` counts labels that differ from what the box started with, in letters. */
  function progress(items, edits = {}) {
    const total = items.length;
    const unreadable = items.filter((i) => i.unreadable).length;
    const notSubtitle = items.filter((i) => i.notSubtitle && !i.unreadable).length;
    const labelled = items.filter((i) => i.label != null).length;
    const changed = Object.values(edits).filter(Boolean).length;
    const confirmed = Object.keys(edits).length;
    return { total, labelled, unreadable, notSubtitle, left: total - labelled - unreadable - notSubtitle, confirmed, changed };
  }

  const api = { normalize, distance, difference, disagreement, hasHangul, readings, startingText, pending, provisional, unlabelled, order, groupFiles, cleanLabel, progress };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.LabelCore = api;
})(typeof window !== 'undefined' ? window : this);
