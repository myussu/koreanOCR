import test from 'node:test';
import assert from 'node:assert/strict';
import { canMine, sameLine, withPendingLine } from '../history-core.js';

const capture = { id: 4, source: '상자를 열면 열쇠가 나옵니다', translation: 'If you open the box, a key comes out.', direction: 'ko_to_en', live: false };
const live = { id: 5, source: '저도요', translation: 'Me too.', direction: 'ko_to_en', live: true, saved_id: 12 };

test('a captured line can become cards as well as a live one, but not English or an untranslated line', () => {
  assert.ok(canMine(capture), 'a snapshot of a menu or a sentence');
  assert.ok(canMine(live));
  assert.ok(!canMine({ ...capture, direction: 'en_to_ko' }), 'English into Korean is not a Korean line to learn');
  assert.ok(!canMine({ ...capture, translation: '' }), 'a line still translating');
  assert.ok(!canMine(null));
});

test('the line the panel hands over is found by its request id, or by its text when the id changed', () => {
  assert.ok(sameLine(capture, { id: 4, sentence: 'whatever' }));
  assert.ok(sameLine(capture, { id: 99, sentence: ` ${capture.source} ` }));
  assert.ok(!sameLine(live, { id: 4, sentence: capture.source }));
});

test('a handed-over line History does not list comes first, so its cards sheet can still open', () => {
  const line = { id: 9, sentence: '열쇠', translation: 'key', app: 'eden.exe' };
  const rows = withPendingLine([live], line);
  assert.equal(rows.length, 2);
  assert.deepEqual({ ...rows[0], at: 0 }, { id: 9, at: 0, source: '열쇠', translation: 'key', direction: 'ko_to_en', app: 'eden.exe', live: false });
  assert.ok(canMine(rows[0]));
  assert.equal(withPendingLine([capture, live], { id: 4, sentence: capture.source }).length, 2, 'a listed line is not repeated');
  assert.deepEqual(withPendingLine([live], null), [live]);
});
