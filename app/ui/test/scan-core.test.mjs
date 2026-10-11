import assert from 'node:assert/strict';
import { test } from 'node:test';

import { CHIP_H, chipFor, chipPlacement, nextItemId, piecesOf, unionRect } from '../scan-core.js';

test('a new item gets a number nobody uses', () => {
  assert.equal(nextItemId([]), 0);
  assert.equal(nextItemId([0, 1, 2]), 3);
  assert.equal(nextItemId(new Map([[5, 1], [2, 1]]).keys()), 6, 'gaps are not reused');
});

test('two or more picked labels can be joined, one joined label can be split, nothing else gets a chip', () => {
  assert.equal(chipFor([]), null);
  assert.equal(chipFor([{ data: {} }]), null, 'one ordinary label: nothing to do');
  assert.deepEqual(chipFor([{ data: {} }, { data: {} }]), { kind: 'join', n: 2 });
  assert.deepEqual(chipFor([{}, {}, {}]), { kind: 'join', n: 3 });
  assert.deepEqual(chipFor([{ parts: [1, 2] }]), { kind: 'split' });
  assert.equal(chipFor([{ parts: [] }]), null);
  assert.deepEqual(chipFor([{ parts: [1, 2] }, { data: {} }]), { kind: 'join', n: 2 }, 'a joined label can be joined again');
});

test('the union is the box around everything', () => {
  assert.equal(unionRect([]), null);
  assert.deepEqual(unionRect([{ x: 100, y: 100, w: 40, h: 18 }, { x: 150, y: 104, w: 60, h: 18 }]), { x: 100, y: 100, w: 110, h: 22 });
});

test('the chip goes above the labels, below when there is no room, and stays on screen', () => {
  const bounds = { w: 800, h: 600 };
  const size = { w: 110, h: CHIP_H };
  assert.deepEqual(chipPlacement({ x: 200, y: 300, w: 100, h: 18 }, size, bounds), { x: 200, y: 300 - CHIP_H - 6, w: 110, h: CHIP_H });
  assert.equal(chipPlacement({ x: 200, y: 10, w: 100, h: 18 }, size, bounds).y, 34, 'no room above: below the labels');
  assert.equal(chipPlacement({ x: 780, y: 300, w: 15, h: 18 }, size, bounds).x, 800 - 110 - 4, 'pushed in from the right edge');
  assert.equal(chipPlacement({ x: -20, y: 300, w: 15, h: 18 }, size, bounds).x, 4, 'and from the left edge');
  assert.ok(chipPlacement({ x: 200, y: 590, w: 100, h: 18 }, size, bounds).y + CHIP_H <= 600, 'never off the bottom');
});

test('the backend gets each picked label as text and position', () => {
  const entries = [{ data: { text: '새', rect: { x: 1, y: 2, w: 3, h: 4 }, label: 'New' } }, { data: { text: '게임', rect: { x: 5, y: 2, w: 6, h: 4 } } }];
  assert.deepEqual(piecesOf(entries), [{ text: '새', rect: { x: 1, y: 2, w: 3, h: 4 } }, { text: '게임', rect: { x: 5, y: 2, w: 6, h: 4 } }]);
});
