import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';

// tools/ is a plain script for the browser and CommonJS for Node.
const require = createRequire(import.meta.url);
const L = require('../../../tools/label-review-core.js');

test('spaces and punctuation are not part of what two readings are compared on', () => {
  assert.equal(L.normalize('오, 하고 있구나…'), '오하고있구나');
  assert.equal(L.normalize('"저게 뭐야?!"'), '저게뭐야');
  assert.equal(L.difference('오, 하고 있구나', '오 하고 있구나!'), 0);
  assert.equal(L.difference('', '...'), 0);
});

test('a different letter counts as a difference in proportion to the length', () => {
  assert.equal(L.difference('가나다라', '가나다마'), 0.25);
  assert.equal(L.difference('가나다라', ''), 1);
  assert.ok(Math.abs(L.difference('가나', '가나다라') - 0.5) < 1e-9);
});

test('readers disagree by the largest gap between any two of them, and one reading has nothing to disagree with', () => {
  assert.equal(L.disagreement(['가나다라']), null);
  assert.equal(L.disagreement(['가나다라', '', '  ']), null, 'empty readings do not count');
  assert.equal(L.disagreement(['가나다라', '가나다라', '가나다마']), 0.25);
});

test('the readings of a picture are the live draft first, then the other readers by name', () => {
  const item = { draft: ' 안녕 ', drafts: { 'paddle-vl': '안녕하세요', hybrid: '', windows: '안녕요' } };
  assert.deepEqual(L.readings(item), [
    { from: 'live', text: '안녕' },
    { from: 'paddle-vl', text: '안녕하세요' },
    { from: 'windows', text: '안녕요' },
  ]);
  assert.deepEqual(L.readings({}), []);
});

test('the box starts with the label if there is one, else the strongest reader that read something, else the live draft', () => {
  assert.deepEqual(L.startingText({ label: '내 글', draft: 'x', drafts: { 'paddle-vl': 'y' } }), { text: '내 글', from: 'label' });
  assert.deepEqual(L.startingText({ draft: 'live', drafts: { 'paddle-vl': 'vl' } }), { text: 'vl', from: 'paddle-vl' });
  assert.deepEqual(L.startingText({ draft: 'live', drafts: { 'paddle-vl': '' } }), { text: 'live', from: 'live' });
  assert.deepEqual(L.startingText({ drafts: { windows: 'w' } }), { text: 'w', from: 'windows' });
  assert.deepEqual(L.startingText({}), { text: '', from: 'none' });
  assert.equal(L.startingText({ label: '' }).from, 'label', 'an empty label that was written is still a label');
});

test('pictures still to do come first, and the ones whose readers disagree most come first among them', () => {
  const items = [
    { id: 'a', draft: '가나다라', drafts: { 'paddle-vl': '가나다라' } }, // agree
    { id: 'b', draft: '가나다라' }, // only one reading
    { id: 'c', draft: '가나다라', drafts: { 'paddle-vl': '마바사아' } }, // disagree
    { id: 'd', draft: '가나다라', label: '가나다라' }, // done
    { id: 'e', unreadable: 'e.unreadable' }, // done
  ];
  assert.deepEqual(L.order(items).map((i) => i.id), ['c', 'b', 'a', 'd', 'e']);
  assert.deepEqual(L.order(items, { disagreementsFirst: false }).map((i) => i.id), ['a', 'b', 'c', 'd', 'e']);
  assert.deepEqual(items.map((i) => i.id), ['a', 'b', 'c', 'd', 'e'], 'the list given is not reordered in place');
});

test('file names in a folder are grouped by picture, and a file with no picture is ignored', () => {
  const groups = L.groupFiles([
    '100-001.png', '100-001.json', '100-001.txt', '100-001.paddle-vl.txt', '100-001.tags',
    '100-002.PNG', '100-002.unreadable',
    '100-003.json', // no picture
    'notes.md',
  ]);
  assert.deepEqual(groups.map((g) => g.id), ['100-001', '100-002']);
  assert.equal(groups[0].png, '100-001.png');
  assert.equal(groups[0].json, '100-001.json');
  assert.equal(groups[0].label, '100-001.txt');
  assert.deepEqual(groups[0].drafts, { 'paddle-vl': '100-001.paddle-vl.txt' });
  assert.equal(groups[0].tags, '100-001.tags');
  assert.equal(groups[1].png, '100-002.PNG');
  assert.equal(groups[1].unreadable, '100-002.unreadable');
  assert.equal(groups[1].label, null);
});

test('a label is one line with single spaces', () => {
  assert.equal(L.cleanLabel('  오,\n하고   있구나 \r\n'), '오, 하고 있구나');
  assert.equal(L.cleanLabel(null), '');
});

test('progress counts what is labelled, unreadable and left, and how many of this session\'s labels were changed', () => {
  const items = [{ label: 'a' }, { label: 'b' }, { unreadable: 'x' }, {}, {}];
  assert.deepEqual(L.progress(items, { one: true, two: false, three: false }), { total: 5, labelled: 2, unreadable: 1, left: 2, confirmed: 3, changed: 1 });
});
