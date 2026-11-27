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
  assert.deepEqual(L.startingText({ draft: '라이브', drafts: { 'paddle-vl': '브이엘' } }), { text: '브이엘', from: 'paddle-vl' });
  assert.deepEqual(L.startingText({ draft: '라이브', drafts: { 'paddle-vl': '' } }), { text: '라이브', from: 'live' });
  assert.deepEqual(L.startingText({ drafts: { windows: '윈도' } }), { text: '윈도', from: 'windows' });
  assert.deepEqual(L.startingText({}), { text: '', from: 'none' });
  assert.equal(L.startingText({ label: '' }).from, 'label', 'an empty label that was written is still a label');
});

test('a provisional label is what the box starts with, then an outside reader, and neither counts as done', () => {
  const agreed = { id: 'a', draft: 'live', drafts: { provisional: '둘이 같다', outside: '둘이 같다', hybrid: '둘이 같다', paddle: '둘이 갇다' } };
  assert.deepEqual(L.startingText(agreed), { text: '둘이 같다', from: 'provisional' });
  assert.deepEqual(L.startingText({ drafts: { outside: '밖', 'paddle-vl': '안', hybrid: '안' } }), { text: '밖', from: 'outside' });
  assert.ok(L.pending(agreed), 'a person still has to look at it');
  const [group] = L.groupFiles(['a.png', 'a.provisional.txt', 'a.outside.txt']);
  assert.equal(group.label, null, 'name.provisional.txt is a draft, never the label');
  assert.deepEqual(Object.keys(group.drafts).sort(), ['outside', 'provisional']);
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
  assert.equal(L.cleanLabel('  오,\n하고   있구나 \r\n'), '오,\n하고 있구나', 'line breaks stay: a whole screen is labelled one piece per line');
  assert.equal(L.cleanLabel('\n\n 체력   100 \n\n장비\n'), '체력 100\n장비');
  assert.equal(L.cleanLabel(null), '');
});

test('progress counts what is labelled, unreadable and left, and how many of this session\'s labels were changed', () => {
  const items = [{ label: 'a' }, { label: 'b' }, { unreadable: 'x' }, {}, {}];
  assert.deepEqual(L.progress(items, { one: true, two: false, three: false }), { total: 5, labelled: 2, unreadable: 1, notSubtitle: 0, left: 2, confirmed: 3, changed: 1 });
});

test('a picture can be marked as not a subtitle, apart from unreadable, and is then done', () => {
  const groups = L.groupFiles(['m.png', 'm.not-subtitle', 'u.png', 'u.unreadable', 'p.png', 'p.provisional.txt', 'n.png', 'n.outside.txt']);
  const by = Object.fromEntries(groups.map((g) => [g.id, g]));
  assert.equal(by.m.notSubtitle, 'm.not-subtitle');
  assert.equal(by.m.unreadable, null);
  assert.ok(!L.pending(by.m) && !L.pending(by.u));
  assert.deepEqual(L.progress(groups), { total: 4, labelled: 0, unreadable: 1, notSubtitle: 1, left: 2, confirmed: 0, changed: 0 });
});

test('pictures with a provisional label are told apart, and "no label at all" leaves them out', () => {
  const prov = { id: 'p', label: null, drafts: { provisional: '원숭이가 셋…' } };
  const none = { id: 'n', label: null, drafts: { outside: '원숭이가 셋' } };
  const done = { id: 'd', label: '네', drafts: {} };
  assert.ok(L.provisional(prov) && !L.provisional(none));
  assert.deepEqual([prov, none, done].filter(L.unlabelled).map((i) => i.id), ['n']);
});

test('an outside reading without Hangul is a comment: never listed, never the starting text', () => {
  const item = { label: null, drafts: { outside: 'The strip shows only a dark background.', paddle: '0 0' } };
  assert.deepEqual(L.readings(item).map((r) => r.from), ['paddle'], 'a local reading stays listed');
  assert.deepEqual(L.startingText(item), { text: '', from: 'none' }, 'but no reading without Hangul starts the box');
  assert.equal(L.startingText({ label: null, drafts: { outside: '폐하?', paddle: '떼하?' } }).text, '폐하?');
});
