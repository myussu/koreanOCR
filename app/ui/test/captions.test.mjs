import test from 'node:test';
import assert from 'node:assert/strict';
import { captionsLine } from '../captions-core.js';
import { ALL_FIELDS, isVisible, sectionOf, fromEditor } from '../settings-core.js';

test('the status row says off, why it is not listening, or what it has taken', () => {
  assert.deepEqual(captionsLine(null), { on: false, text: 'off (Settings → Live subtitles)' });
  assert.deepEqual(captionsLine({ enabled: false, listening: false }), { on: false, text: 'off (Settings → Live subtitles)' });
  const failed = captionsLine({ enabled: true, listening: false, problem: 'YouTube captions cannot listen on port 18098 of this PC.' });
  assert.equal(failed.on, false);
  assert.match(failed.text, /^not listening: YouTube captions cannot listen on port 18098/);
  assert.deepEqual(captionsLine({ enabled: true, listening: true, port: 18098, accepted: 0, dropped: 0, refused: 0 }), { on: true, text: 'port 18098 on this PC · 0 lines taken' });
  assert.equal(captionsLine({ enabled: true, listening: true, port: 18098, accepted: 1, dropped: 1, refused: 0 }).text, 'port 18098 on this PC · 1 line taken · 1 repeat or empty line dropped');
});

test('refused requests point at the code in the extension', () => {
  const line = captionsLine({ enabled: true, listening: true, port: 19000, accepted: 12, dropped: 3, refused: 2 });
  assert.equal(line.text, 'port 19000 on this PC · 12 lines taken · 3 repeats or empty lines dropped · 2 requests refused (check the code in the extension)');
});

test('the captions settings sit with live subtitles and the code only shows once captions are on', () => {
  const field = (path) => ALL_FIELDS.find((f) => f.path === path);
  for (const path of ['captions.enabled', 'captions.token', 'captions.port', 'captions.allowed_origin']) assert.equal(sectionOf(path), 'live', path);
  const defaults = { captions: { enabled: false, token: null, port: 18098, allowed_origin: '' } };
  assert.equal(isVisible(field('captions.token'), {}, defaults, defaults), false);
  assert.equal(isVisible(field('captions.token'), { 'captions.enabled': true }, defaults, defaults), true);
  // The code is shown as text so it can be copied, and emptying it removes the key (Maru then makes a new one).
  assert.equal(field('captions.token').type, 'text');
  assert.equal(fromEditor(field('captions.token'), '  '), null);
  assert.ok(field('captions.port').advanced && field('captions.allowed_origin').advanced);
});
