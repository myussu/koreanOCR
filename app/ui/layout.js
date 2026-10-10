// Label placement for the screen-scan overlay.
//
// Pure geometry, no DOM: given where each piece of Korean text is and how big its label wants to
// be, decide where every label goes so that nothing covers other text or other labels.
//
// The hard case is a dense menu: rows a few pixels apart, an inventory grid of narrow cells, a
// column of buttons. There the obvious "just put it underneath" lands on the next row's text.
//
//   1. Each label tries many positions (below / above / right / left, aligned several ways, in
//      two rings of distance) and every position is scored by how much it covers: other text,
//      labels already placed, and its own text, plus a mild taste for "below, then above".
//   2. The most constrained labels (fewest clean positions) are placed first.
//   3. A few improvement passes re-place each label given all the others.
//   4. A label that still collides is shrunk; as a last resort it collapses to a dot that the UI
//      expands on hover.
//   5. Labels that are still fine keep their old position when others change, so the screen does
//      not jump around while the model's answers arrive.

const GAP = 3; // px between text and its label
const SCALES = [1, 0.85, 0.72]; // label sizes tried, largest first
const DOT = 12; // a collapsed label
const PASSES = 3;

// "Hit" = how much a position covers. It decides whether a label may stay full size.
const OK_HIT = 0.5; // below this the position is clean: keep the biggest size that gets here
const DOT_HIT = 2.2; // at or above this even the smallest label would hide too much: collapse
const FLOOR = 1.0; // charged once for every object a label covers at all (more than a sliver)
const SLIVER = 0.005; // a label covering less than this fraction of itself is not "covering"
const W_TEXT = 12; // per fraction of the label covering someone else's text
const W_LABEL = 14; // ... covering another label
const W_OWN = 30; // ... covering the label's own text

// "Taste" only chooses between clean positions.
const W_EDGE = 0.02; // per pixel pushed back onto the screen
const W_DIST = 0.004; // per pixel of distance from the text
const STICKY = 0.6; // bonus for staying where it was

// Mild preference for where a label goes: below the text, then above, then beside.
const SIDE_COST = { below: 0, right: 0.4, above: 0.8, left: 1.0 };
// Shrinking a label a little is better than flipping it to the other side of its text, where a
// neighbour could mistake it for the next cell's label.
const SCALE_COST = 2.0; // per unit of size given up (0.85 costs 0.3)
const RING_COST = [0, 0.7]; // the second ring sits farther out

const area = (r) => Math.max(0, r.w) * Math.max(0, r.h);

function overlap(a, b) {
  const w = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
  return w > 0 && h > 0 ? w * h : 0;
}

/** The area of `r` covered by `other`, as a fraction of `r`. */
function covered(r, other) {
  const a = area(r);
  return a > 0 ? overlap(r, other) / a : 0;
}

function clampInto(r, bounds) {
  const x = Math.min(Math.max(r.x, bounds.x), Math.max(bounds.x, bounds.x + bounds.w - r.w));
  const y = Math.min(Math.max(r.y, bounds.y), Math.max(bounds.y, bounds.y + bounds.h - r.h));
  return { x, y, w: r.w, h: r.h, moved: Math.abs(x - r.x) + Math.abs(y - r.y) };
}

/** Every position worth trying for a label of `size` next to `anchor`. */
function candidates(anchor, size, bounds) {
  const out = [];
  const { w, h } = size;
  for (let ring = 0; ring < RING_COST.length; ring++) {
    const gap = GAP + ring * Math.max(6, h * 0.6);
    const xs = [anchor.x, anchor.x + (anchor.w - w) / 2, anchor.x + anchor.w - w];
    const ys = [anchor.y + (anchor.h - h) / 2, anchor.y, anchor.y + anchor.h - h];
    for (const x of xs) {
      out.push({ side: 'below', ring, x, y: anchor.y + anchor.h + gap });
      out.push({ side: 'above', ring, x, y: anchor.y - gap - h });
    }
    for (const y of ys) {
      out.push({ side: 'right', ring, x: anchor.x + anchor.w + gap, y });
      out.push({ side: 'left', ring, x: anchor.x - gap - w, y });
    }
  }
  return out.map((c) => {
    const fit = clampInto({ x: c.x, y: c.y, w, h }, bounds);
    return { side: c.side, ring: c.ring, x: fit.x, y: fit.y, w, h, moved: fit.moved };
  });
}

/**
 * Score one position (lower is better), in two parts: `hit` (how much it covers: texts, labels
 * already placed, its own text) and `taste` (side, distance, being pushed back onto the screen).
 * `texts` are every item's text box; `placed` the labels already down.
 */
function cost(c, self, texts, placed, anchorCentre) {
  let hit = 0;
  const touch = (frac, weight) => {
    if (frac > SLIVER) hit += FLOOR + weight * frac;
  };
  for (const t of texts) touch(covered(c, t.rect), t.id === self.id ? W_OWN : W_TEXT);
  for (const p of placed) touch(covered(c, p), W_LABEL);
  const taste =
    SIDE_COST[c.side] + RING_COST[c.ring] + c.moved * W_EDGE + W_DIST * (Math.abs(c.x + c.w / 2 - anchorCentre.x) + Math.abs(c.y + c.h / 2 - anchorCentre.y));
  return { hit, taste };
}

function pickBest(item, size, bounds, texts, placed, previous) {
  const centre = { x: item.rect.x + item.rect.w / 2, y: item.rect.y + item.rect.h / 2 };
  let best = null;
  const consider = (c, bonus = 0) => {
    const { hit, taste } = cost(c, item, texts, placed, centre);
    const score = hit + taste - bonus;
    if (!best || score < best.score - 1e-9) best = { ...c, score, hit };
  };
  for (const c of candidates(item.rect, size, bounds)) consider(c);
  // Stay where we were if that is still good enough.
  if (previous && Math.abs(previous.w - size.w) < 0.5 && Math.abs(previous.h - size.h) < 0.5) {
    consider({ side: previous.side, ring: previous.ring ?? 0, x: previous.x, y: previous.y, w: size.w, h: size.h, moved: 0 }, STICKY);
  }
  return best;
}

/** How many of the label's positions are clean against the static text alone (fewer = harder). */
function freedom(item, size, bounds, texts) {
  const centre = { x: item.rect.x + item.rect.w / 2, y: item.rect.y + item.rect.h / 2 };
  return candidates(item.rect, size, bounds).filter((c) => cost(c, item, texts, [], centre).hit < OK_HIT).length;
}

/**
 * @param items  [{ id, rect: {x,y,w,h}, size: {w,h} }]  the text and its label's natural size
 * @param opts   { bounds: {x,y,w,h}, maxWidth?: number, previous?: Map<id, placement> }
 * @returns Map<id, { x, y, w, h, scale, mode: 'full' | 'dot', side }>
 */
export function placeLabels(items, opts) {
  const bounds = opts.bounds;
  const previous = opts.previous ?? new Map();
  const labelled = items.filter((i) => i.size && i.size.w > 0 && i.size.h > 0);
  const texts = items.map((i) => ({ id: i.id, rect: i.rect }));

  // The label's size at each scale, with long labels capped to a width that suits their text.
  const sizeAt = (item, scale) => {
    const cap = Math.max(opts.maxWidth ?? 260, 0);
    const widest = Math.min(cap, Math.max(item.rect.w * 1.8, 96));
    return { w: Math.min(item.size.w, widest) * scale, h: item.size.h * scale };
  };

  // Hardest first: the fewest clean positions; ties top-to-bottom, left-to-right.
  const order = labelled
    .map((item) => ({ item, free: freedom(item, sizeAt(item, 1), bounds, texts) }))
    .sort((a, b) => a.free - b.free || a.item.rect.y - b.item.rect.y || a.item.rect.x - b.item.rect.x)
    .map((o) => o.item);

  const result = new Map();
  const placedExcept = (id) => [...result.entries()].filter(([k]) => k !== id).map(([, v]) => v);

  const place = (item) => {
    const others = placedExcept(item.id);
    const sticky = previous.get(item.id)?.mode === 'full' ? previous.get(item.id) : null;
    const options = SCALES.map((scale) => {
      const best = pickBest(item, sizeAt(item, scale), bounds, texts, others, sticky);
      return { ...best, scale, mode: 'full', total: best.score + (1 - scale) * SCALE_COST };
    });
    // Among the clean options take the best overall (size and side together); with none clean,
    // the one that covers the least.
    const clean = options.filter((o) => o.hit < OK_HIT);
    const byTotal = (a, b) => a.total - b.total;
    const byHit = (a, b) => a.hit - b.hit || a.total - b.total;
    let chosen = clean.length ? [...clean].sort(byTotal)[0] : [...options].sort(byHit)[0];
    if (chosen.hit >= DOT_HIT) {
      // Nothing fits: a small marker that opens when pointed at.
      const dot = pickBest(item, { w: DOT, h: DOT }, bounds, texts, others, null);
      chosen = { ...dot, scale: 1, mode: 'dot' };
    }
    result.set(item.id, chosen);
  };

  for (const item of order) place(item);

  // Re-place each label given all the others; stop when nothing changes.
  for (let pass = 0; pass < PASSES; pass++) {
    let changed = false;
    for (const item of order) {
      const before = result.get(item.id);
      place(item);
      const after = result.get(item.id);
      if (after.x !== before.x || after.y !== before.y || after.mode !== before.mode || after.scale !== before.scale) changed = true;
    }
    if (!changed) break;
  }

  const out = new Map();
  for (const [id, p] of result) out.set(id, { x: p.x, y: p.y, w: p.w, h: p.h, scale: p.scale, mode: p.mode, side: p.side, ring: p.ring });
  return out;
}

/**
 * For tests and diagnostics: how many labels visibly cover something. `tolerance` is the fraction
 * of a label's own area that may be covered before it counts.
 */
export function collisions(items, placements, tolerance = 0.02) {
  const found = { labelLabel: 0, labelText: 0 };
  const boxes = [...placements.entries()].filter(([, p]) => p.mode === 'full');
  for (let i = 0; i < boxes.length; i++) {
    for (let j = i + 1; j < boxes.length; j++) {
      if (covered(boxes[i][1], boxes[j][1]) > tolerance || covered(boxes[j][1], boxes[i][1]) > tolerance) found.labelLabel++;
    }
    for (const t of items) if (covered(boxes[i][1], t.rect) > tolerance) found.labelText++;
  }
  return found;
}
