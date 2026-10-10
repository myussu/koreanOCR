import test from 'node:test';
import assert from 'node:assert/strict';
import { placeLabels, collisions } from '../layout.js';

const SCREEN = { x: 0, y: 0, w: 1920, h: 1080 };
const size = (text) => ({ w: 7 * text.length + 12, h: 17 }); // like a 12px label with padding

let nextId = 0;
const item = (x, y, w, h, label) => ({ id: nextId++, rect: { x, y, w, h }, size: size(label) });

/** A column of menu rows: text `h` tall, rows `pitch` apart. */
function menu(count, { x = 200, y = 100, w = 150, h = 20, pitch = 26, label = 'Inventory' } = {}) {
  return Array.from({ length: count }, (_, i) => item(x, y + i * pitch, w, h, label));
}

/** An inventory grid: narrow cells with a short name each. */
function grid(cols, rows, { x = 300, y = 300, cell = 70, textH = 14, gapY = 36, label = 'Iron sword' } = {}) {
  const out = [];
  for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) out.push(item(x + c * (cell + 10), y + r * (textH + gapY), cell, textH, label));
  return out;
}

const run = (items, extra = {}) => placeLabels(items, { bounds: SCREEN, ...extra });

test('a lone label goes just below its text', () => {
  const t = item(500, 400, 100, 20, 'Start');
  const p = run([t]).get(t.id);
  assert.equal(p.mode, 'full');
  assert.equal(p.side, 'below');
  assert.ok(p.y >= 420 && p.y <= 428, `directly under the text, got y=${p.y}`);
  assert.ok(p.x >= 500 - 1 && p.x + p.w <= 600 + 80);
});

test('at the bottom of the screen the label moves beside or above its text, never off-screen', () => {
  const t = item(500, 1070, 100, 10, 'Quit');
  const items = [t];
  const placed = run(items);
  const p = placed.get(t.id);
  assert.notEqual(p.side, 'below');
  assert.ok(p.y + p.h <= 1080 + 0.01 && p.x >= 0);
  assert.deepEqual(collisions(items, placed), { labelLabel: 0, labelText: 0 });
});

test('a label would rather shrink a little than flip to the other side of its text', () => {
  // two neighbours whose natural labels (~90px) do not fit in the 80px pitch below them
  const a = item(300, 300, 70, 14, 'Iron sword');
  const b = item(380, 300, 70, 14, 'Wooden shield');
  const placed = run([a, b]);
  assert.equal(placed.get(a.id).side, 'below');
  assert.equal(placed.get(b.id).side, 'below', 'both stay on the same side so each is clearly its own');
  assert.deepEqual(collisions([a, b], placed), { labelLabel: 0, labelText: 0 });
});

test('labels stay inside the screen', () => {
  const items = [item(1900, 10, 20, 12, 'Close'), item(0, 0, 30, 12, 'Back'), item(1850, 1060, 70, 18, 'Next page')];
  for (const [, p] of run(items)) {
    assert.ok(p.x >= 0 && p.y >= 0 && p.x + p.w <= 1920 + 0.01 && p.y + p.h <= 1080 + 0.01, JSON.stringify(p));
  }
});

test('a tight menu column: nothing lands on the next row, nothing overlaps', () => {
  // rows are 26px apart with 20px text: only 6px between rows, less than a label is tall
  const items = menu(14);
  const placed = run(items);
  assert.equal(placed.size, 14);
  const c = collisions(items, placed);
  assert.deepEqual(c, { labelLabel: 0, labelText: 0 }, 'every label sits in free space, e.g. beside the row');
  for (const [, p] of placed) assert.equal(p.mode, 'full', 'no label needs to collapse here');
});

test('a very tight column (rows touching) still places everything without collisions', () => {
  const items = menu(10, { pitch: 21, h: 20 });
  const placed = run(items);
  assert.deepEqual(collisions(items, placed), { labelLabel: 0, labelText: 0 });
});

test('an inventory grid: narrow cells, labels do not collide', () => {
  const items = grid(8, 5);
  const placed = run(items);
  assert.equal(placed.size, 40);
  assert.deepEqual(collisions(items, placed), { labelLabel: 0, labelText: 0 });
});

test('a grid with almost no room between cells shrinks or collapses labels instead of colliding', () => {
  const items = grid(10, 6, { cell: 60, gapY: 6, label: 'Healing potion' });
  const placed = run(items);
  assert.equal(placed.size, 60);
  const c = collisions(items, placed, 0.1);
  assert.equal(c.labelText + c.labelLabel, 0, `unplaceable labels collapse to dots rather than overlap: ${JSON.stringify(c)}`);
  const modes = new Set([...placed.values()].map((p) => p.mode));
  assert.ok(modes.has('dot') || [...placed.values()].some((p) => p.scale < 1), 'something had to give');
});

test('hopeless overlap collapses to dots, and every item still gets a spot', () => {
  const items = Array.from({ length: 80 }, (_, i) => item(500 + (i % 8) * 4, 400 + Math.floor(i / 8) * 3, 120, 20, 'Overlapping'));
  const placed = run(items);
  assert.equal(placed.size, 80);
  for (const [, p] of placed) assert.ok(p.w > 0 && p.h > 0 && Number.isFinite(p.x) && Number.isFinite(p.y));
  assert.ok([...placed.values()].some((p) => p.mode === 'dot'));
});

test('the same input always gives the same layout', () => {
  const items = grid(6, 4);
  const a = [...run(items)].map(([id, p]) => [id, p.x, p.y, p.mode]);
  const b = [...run(items)].map(([id, p]) => [id, p.x, p.y, p.mode]);
  assert.deepEqual(a, b);
});

test('labels already placed stay put when more labels arrive', () => {
  const items = menu(8, { pitch: 40 });
  const first = run(items.slice(0, 5));
  const second = run(items, { previous: first });
  for (const it of items.slice(0, 5)) {
    const a = first.get(it.id);
    const b = second.get(it.id);
    assert.ok(Math.abs(a.x - b.x) < 0.01 && Math.abs(a.y - b.y) < 0.01, `item ${it.id} moved from (${a.x},${a.y}) to (${b.x},${b.y})`);
  }
});

test('a label that would now collide does move, even if it was sticky', () => {
  const t = item(500, 400, 100, 20, 'Shop');
  const first = run([t]);
  // a new piece of text appears exactly where the old label is
  const where = first.get(t.id);
  const blocker = item(where.x - 5, where.y - 2, where.w + 10, where.h + 4, 'x');
  blocker.size = null; // text only, no label of its own
  const second = run([t, blocker], { previous: first });
  const moved = second.get(t.id);
  assert.ok(moved.x !== where.x || moved.y !== where.y);
  assert.deepEqual(collisions([t, blocker], second), { labelLabel: 0, labelText: 0 });
});

test('long labels are capped relative to their text', () => {
  const t = item(500, 400, 60, 18, 'x');
  t.size = { w: 600, h: 17 };
  const p = run([t], { maxWidth: 260 }).get(t.id);
  assert.ok(p.w <= 260 + 0.01 && p.w >= 96, `capped, not collapsed: ${p.w}`);
});

test('items without a label size are text-only obstacles, never placed', () => {
  const a = item(100, 100, 100, 20, 'A');
  const b = { id: 99, rect: { x: 100, y: 124, w: 100, h: 20 }, size: null };
  const placed = run([a, b]);
  assert.ok(placed.has(a.id) && !placed.has(99));
  assert.equal(collisions([a, b], placed).labelText, 0, 'the unlabelled text is still kept clear');
});

test('150 labels place quickly', () => {
  const items = [...menu(30, { x: 100 }), ...menu(30, { x: 600, pitch: 30 }), ...grid(10, 9, { x: 1000, y: 100 })];
  assert.equal(items.length, 150);
  const t0 = performance.now();
  const placed = run(items);
  const ms = performance.now() - t0;
  assert.equal(placed.size, 150);
  assert.ok(ms < 1500, `took ${ms.toFixed(0)} ms`);
  console.log(`# 150 labels placed in ${ms.toFixed(0)} ms`);
});
