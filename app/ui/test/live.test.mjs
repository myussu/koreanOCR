import assert from 'node:assert/strict';
import { test } from 'node:test';

import { bandStyle, clampCenter, createBand, fontFor, latencyText } from '../live-core.js';

test('the font follows the original text and stays readable', () => {
  assert.equal(fontFor(30), 24);
  assert.equal(fontFor(8), 15, 'tiny text is not copied');
  assert.equal(fontFor(120), 40, 'huge text is capped');
  assert.equal(fontFor(undefined), 22);
  assert.equal(fontFor(0), 22);
});

test('the band covers the original and grows away from the edge it is anchored to', () => {
  const win = { w: 900, h: 300 };
  const cover = { x: 200, y: 200, w: 500, h: 40 };
  const up = bandStyle({ cover, win, anchorBottom: true });
  assert.equal(up.bottom, 300 - 240 - 10, 'its bottom edge sits just under the original');
  assert.equal(up.top, undefined);
  assert.equal(up.minHeight, 60);
  assert.equal(up.minWidth, 520);
  assert.equal(up.center, 450);
  const down = bandStyle({ cover: { x: 200, y: 30, w: 500, h: 40 }, win, anchorBottom: false });
  assert.equal(down.top, 20);
  assert.equal(down.bottom, undefined);
});

test('a wide original does not make the band wider than the window', () => {
  const st = bandStyle({ cover: { x: 0, y: 100, w: 900, h: 40 }, win: { w: 900, h: 300 }, anchorBottom: true });
  assert.equal(st.minWidth, 884);
  assert.equal(st.maxWidth, 884);
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
