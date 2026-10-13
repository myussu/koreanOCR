// The parts of joining screen labels that need no DOM, so they can be tested: which action the chip
// offers, where it goes, and the number of a new item.

export const CHIP_H = 26;
const GAP = 6;
const EDGE = 4;

/** Selecting words is explicit; an ordinary click opens the word's explanation. */
export function labelClickAction(joining, shiftKey = false) {
  return joining || shiftKey ? 'select' : 'open';
}

export const startsNewScan = (id, latest) => id > latest;
export const clearsCurrentScan = (current, through) => current == null || through == null || through >= current;
export const acceptsJoinReply = (currentId, requestId, currentRevision, requestRevision) => currentId === requestId && currentRevision === requestRevision;

/** An item number that no label on screen uses. */
export function nextItemId(ids) {
  let max = -1;
  for (const id of ids) if (Number.isFinite(id) && id > max) max = id;
  return max + 1;
}

export const canSeparate = (it) => (it?.data?.join?.parts?.length ?? 0) >= 2 || (it?.parts?.length ?? 0) >= 2;

/** Include unfinished joins and reserved correction IDs; their labels may not have arrived yet. */
export const nextJoinId = (items, pending, reserved) => nextItemId([...items, ...pending, ...reserved]);

export const acceptsJoinFailure = (currentScan, requestScan, currentParts, requestParts) => currentScan === requestScan && currentParts === requestParts;

/** Remove absorbed ancestors too, including joins that were subsequently corrected. */
export function joinedTreeIds(items, root) {
  const seen = new Set();
  const remaining = [root];
  while (remaining.length) {
    const id = remaining.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    remaining.push(...(items.get(id)?.parts ?? []));
  }
  return [...seen];
}

/**
 * What the chip offers for the labels picked so far (entries as the overlay keeps them: `parts` is set
 * on a label that was made by joining others).
 */
export function chipFor(picked) {
  if (picked.length >= 2) return { kind: 'join', n: picked.length };
  if (picked.length === 1 && canSeparate(picked[0])) return { kind: 'split' };
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
