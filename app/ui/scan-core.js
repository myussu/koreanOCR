// The parts of joining screen labels that need no DOM, so they can be tested: which action the chip
// offers, where it goes, and the number of a new item.

export const CHIP_H = 26;
const GAP = 6;
const EDGE = 4;

/** An item number that no label on screen uses. */
export function nextItemId(ids) {
  let max = -1;
  for (const id of ids) if (Number.isFinite(id) && id > max) max = id;
  return max + 1;
}

/**
 * What the chip offers for the labels picked so far (entries as the overlay keeps them: `parts` is set
 * on a label that was made by joining others).
 */
export function chipFor(picked) {
  if (picked.length >= 2) return { kind: 'join', n: picked.length };
  if (picked.length === 1 && picked[0].parts?.length) return { kind: 'split' };
  return null;
}

/** The smallest rectangle around all of `rects` (`{x, y, w, h}`). */
export function unionRect(rects) {
  if (!rects.length) return null;
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const r of rects) {
    x0 = Math.min(x0, r.x);
    y0 = Math.min(y0, r.y);
    x1 = Math.max(x1, r.x + r.w);
    y1 = Math.max(y1, r.y + r.h);
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** Where the chip goes: just above the picked labels, or below them when there is no room above; always inside `bounds`. */
export function chipPlacement(union, size, bounds) {
  const x = Math.max(EDGE, Math.min(union.x, bounds.w - size.w - EDGE));
  const above = union.y - size.h - GAP;
  const y = above >= EDGE ? above : Math.min(union.y + union.h + GAP, bounds.h - size.h - EDGE);
  return { x, y, w: size.w, h: size.h };
}

/** What the backend needs for each label that is joined: its text and where it is (physical pixels). */
export function piecesOf(entries) {
  return entries.map((it) => ({ text: it.data.text, rect: it.data.rect }));
}
