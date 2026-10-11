import assert from 'node:assert/strict';
import { test } from 'node:test';

import { bandStyle, bandWidth, chooseSide, clampCenter, createBand, estimateHeight, fitFont, fontFor, gapFor, latencyText, liveHistory, roomOn } from '../live-core.js';

test('live history follows capture order without changing the normal history array', () => {
  const items = [{ id: 3, live: true }, { id: 1, live: true }, { id: 2, live: false }];
  assert.deepEqual(liveHistory(items).map((i) => i.id), [1, 3]);
  assert.deepEqual(items.map((i) => i.id), [3, 1, 2]);
});

test('the font follows the original text and stays readable', () => {
  assert.equal(fontFor(30), 24);
  assert.equal(fontFor(8), 15, 'tiny text is not copied');
  assert.equal(fontFor(120), 40, 'huge text is capped');
  assert.equal(fontFor(undefined), 22);
  assert.equal(fontFor(0), 22);
});

test('the gap to the original is a third of a line, at least a few pixels', () => {
  assert.equal(gapFor(34), 12);
  assert.equal(gapFor(10), 8);
  assert.equal(gapFor(undefined), 8);
});

test('the band is at least a comfortable line wide, as wide as the original with some to spare, and never wider than the window', () => {
  assert.equal(bandWidth({ cover: { w: 500 }, win: { w: 900 } }), 640);
  assert.equal(bandWidth({ cover: { w: 100 }, win: { w: 900 } }), 640);
  assert.equal(bandWidth({ cover: { w: 900 }, win: { w: 900 } }), 884);
  assert.equal(bandWidth({ cover: { w: 50 }, win: { w: 100 } }), 120, 'but never a sliver');
});

test('a long original is expected to need more lines than a short one', () => {
  assert.equal(estimateHeight({ sourceChars: 13, font: 24, maxWidth: 640 }), 43, 'one line');
  assert.equal(estimateHeight({ sourceChars: 60, font: 24, maxWidth: 640 }), 106, 'three lines');
  assert.ok(estimateHeight({ sourceChars: undefined, font: 24, maxWidth: 640 }) > 0, 'an unknown length still gives a guess');
});

test('room is what is left on a side of the original after the gap and the window edge', () => {
  const at = { cover: { x: 200, y: 200, w: 500, h: 40 }, win: { w: 900, h: 300 }, gap: 12 };
  assert.equal(roomOn('above', at), 180);
  assert.equal(roomOn('below', at), 40);
});

test('the band goes on the preferred side when it fits there, on the other when only that fits, and on the roomier when neither does', () => {
  const win = { w: 900, h: 300 };
  const low = { x: 200, y: 200, w: 500, h: 40 };
  assert.equal(chooseSide({ cover: low, win, gap: 12, needed: 100, prefer: 'above' }), 'above');
  assert.equal(chooseSide({ cover: low, win, gap: 12, needed: 100, prefer: 'below' }), 'above', 'no room below, so above');
  const mid = { x: 200, y: 100, w: 500, h: 40 };
  assert.equal(chooseSide({ cover: mid, win, gap: 12, needed: 200, prefer: 'above' }), 'below', 'neither fits: the roomier side');
  assert.equal(chooseSide({ cover: low, win, gap: 12, needed: 20, prefer: undefined }), 'below', 'below is the default');
});

test('the band is pinned to the edge of the original on its side and never reaches over it', () => {
  const win = { w: 900, h: 300 };
  const low = { x: 200, y: 200, w: 500, h: 40 };
  const up = bandStyle({ cover: low, win, side: 'above', gap: 12 });
  assert.equal(up.bottom, 300 - 200 + 12, 'its bottom edge is a gap above the original');
  assert.equal(up.top, undefined);
  assert.equal(up.center, 450);
  const high = { x: 200, y: 30, w: 500, h: 40 };
  const down = bandStyle({ cover: high, win, side: 'below', gap: 12 });
  assert.equal(down.top, 30 + 40 + 12, 'its top edge is a gap under the original');
  assert.equal(down.bottom, undefined);
});

test('the font shrinks in steps only as far as needed, and not below a readable size', () => {
  const measure = (f) => f * 5;
  assert.equal(fitFont(24, 200, measure), 24, 'fits already');
  assert.equal(fitFont(24, 100, measure), 20);
  assert.equal(fitFont(24, 10, measure), 14, 'a floor');
});

test('the band is kept inside the window', () => {
  assert.equal(clampCenter(450, 300, 900), 450);
  assert.equal(clampCenter(20, 300, 900), 158, 'pushed right');
  assert.equal(clampCenter(880, 300, 900), 742, 'pushed left');
});

test('a translation stays up until its original is gone and then lingers', () => {
  const b = createBand();
  let r = b.line({ id: 5, text: 'Hel', done: false, linger_ms: 1500 });
  assert.equal(r.show.text, 'Hel');
  assert.equal(r.hideIn, null);
  r = b.line({ id: 5, text: 'Hello.', done: true, linger_ms: 1500 });
  assert.equal(r.show.text, 'Hello.');
  assert.equal(r.hideIn, null, 'the original is still on screen');
  r = b.cleared(5);
  assert.equal(r.hideIn, 1500);
});

test('an original that goes before its translation is ready keeps the band until it is done, then lingers', () => {
  const b = createBand();
  b.line({ id: 7, text: 'Hel', done: false, linger_ms: 1000 });
  assert.equal(b.cleared(7).hideIn, null, 'still streaming');
  const r = b.line({ id: 7, text: 'Hello.', done: true, linger_ms: 1000 });
  assert.equal(r.hideIn, 1000);
});

test('a newer line takes over and cancels a pending hide; an older one finishing late is ignored', () => {
  const b = createBand();
  b.line({ id: 1, text: 'One.', done: true, linger_ms: 1500 });
  assert.equal(b.cleared(1).hideIn, 1500);
  const next = b.line({ id: 2, text: 'Two', done: false, linger_ms: 1500 });
  assert.equal(next.show.id, 2);
  assert.equal(next.cancelHide, true, 'the new line cancels the pending hide');
  const late = b.line({ id: 1, text: 'One. (late)', done: true, linger_ms: 1500 });
  assert.equal(late.show, null);
});

test('a clear that names a line not shown yet applies when that line arrives', () => {
  const b = createBand();
  b.line({ id: 1, text: 'One.', done: true, linger_ms: 500 });
  b.cleared(2); // line 2's original is already gone, its translation has not started
  const r = b.line({ id: 2, text: 'Two.', done: true, linger_ms: 500 });
  assert.equal(r.show.text, 'Two.');
  assert.equal(r.hideIn, 500, 'it shows briefly instead of staying up forever');
});

test('turning live mode off resets everything', () => {
  const b = createBand();
  b.line({ id: 9, text: 'Hi.', done: true, linger_ms: 500 });
  b.cleared(9);
  assert.equal(b.off().cancelHide, true);
  assert.equal(b.line({ id: 1, text: 'Again.', done: true, linger_ms: 500 }).show.text, 'Again.', 'ids start over');
});

test('the history shows how fast a live line was, and nothing for other lines', () => {
  assert.equal(latencyText({ first_ms: 820, total_ms: 1410 }), '0.8 s to the first word · 1.4 s in all');
  assert.equal(latencyText({ first_ms: null, total_ms: null }), '');
  assert.equal(latencyText({}), '');
});
