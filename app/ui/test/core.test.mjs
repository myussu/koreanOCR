import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyState, reduce, openDeep, meaningFor, translationRows, sentenceOf } from '../core.js';

const word = (over = {}) => ({ index: 0, surface: '학교에서', lemma: '학교', content: true, state: 'new', lookups: 0, dict_glosses: ['school'], morphs: [], ...over });
const started = (id = 1, direction = 'ko_to_en') => ({ type: 'started', id, direction, capture: { text: '학교에서 공부해요' }, cached: false });

test('events build up one request', () => {
  let s = reduce(emptyState(), started());
  s = reduce(s, { type: 'breakdown', id: 1, side: 'source', eojeols: [word()], romanization: 'x', pronunciation: 'y' });
  s = reduce(s, { type: 'translation', id: 1, variant: 'primary', delta: 'I study ' });
  s = reduce(s, { type: 'translation', id: 1, variant: 'primary', delta: 'at school.' });
  s = reduce(s, { type: 'gloss', id: 1, side: 'source', gloss: { index: 0, meaning: 'at school', note: null } });
  s = reduce(s, { type: 'translation_done', id: 1, variant: 'primary', text: 'I study at school.' });
  s = reduce(s, { type: 'done', id: 1 });
  assert.equal(s.translations.primary, 'I study at school.');
  assert.equal(s.glosses.source[0].meaning, 'at school');
  assert.ok(s.finished && s.done.primary);
  assert.equal(s.source.eojeols.length, 1);
});

test('events of a superseded request are ignored and the state object is untouched', () => {
  const s1 = reduce(emptyState(), started(2));
  const s2 = reduce(s1, { type: 'translation', id: 1, variant: 'primary', delta: 'stale' });
  assert.equal(s2, s1);
});

test('a new request resets everything, including an open deep dive', () => {
  let s = reduce(emptyState(), started(1));
  s = openDeep(s, { id: 9, eojeol: word(), sentence: 's' });
  s = reduce(s, started(3));
  assert.equal(s.id, 3);
  assert.equal(s.deep, null);
  assert.deepEqual(s.translations, {});
});

test('deep dive events are routed by their own id and stream into sections', () => {
  let s = reduce(emptyState(), started(1));
  s = reduce(s, { type: 'breakdown', id: 1, side: 'source', eojeols: [word(), word({ index: 1, surface: '공부해요', lemma: '공부하다' })], romanization: '', pronunciation: '' });
  s = openDeep(s, { id: 5, eojeol: word(), sentence: 's' });
  s = reduce(s, { type: 'deep_entries', id: 5, entries: [{ lemma: '학교' }], conjugation: [{ form: '학교', label: 'noun' }] });
  s = reduce(s, { type: 'deep_delta', id: 5, section: 'usage', delta: 'A ' });
  s = reduce(s, { type: 'deep_delta', id: 5, section: 'usage', delta: 'school.' });
  s = reduce(s, { type: 'done', id: 5 });
  assert.equal(s.deep.sections.usage, 'A school.');
  assert.ok(s.deep.finished);
  assert.equal(s.deep.entries[0].lemma, '학교');
  assert.equal(s.finished, false, 'the translation request is a different request');
});

test('a lookup updates the state of every word with that lemma immediately', () => {
  let s = reduce(emptyState(), started(1));
  s = reduce(s, { type: 'breakdown', id: 1, side: 'source', eojeols: [word(), word({ index: 1, surface: '학교는' })], romanization: '', pronunciation: '' });
  s = openDeep(s, { id: 5, eojeol: word(), sentence: 's' });
  s = reduce(s, { type: 'word_state', id: 5, status: { key: '학교', state: 'learning', lookups: 1, saved: false } });
  assert.deepEqual(s.source.eojeols.map((e) => e.state), ['learning', 'learning']);
  assert.equal(s.deep.status.lookups, 1);
});

test('meaning selection follows the learner\'s gloss mode', () => {
  const ctx = { meaning: 'at school', note: 'location' };
  assert.equal(meaningFor(word(), ctx, 'all').source, 'context');
  assert.equal(meaningFor(word(), undefined, 'all').text, 'school', 'dictionary meaning shows until the model answers');
  assert.equal(meaningFor(word(), undefined, 'all').source, 'dictionary');
  assert.equal(meaningFor(word({ dict_glosses: [] }), undefined, 'all').source, 'pending');

  const known = word({ state: 'known' });
  assert.deepEqual([meaningFor(known, ctx, 'fade_known').faded, meaningFor(known, ctx, 'fade_known').text], [true, 'at school']);
  assert.equal(meaningFor(known, ctx, 'hide_known').source, 'hidden');
  assert.equal(meaningFor(known, ctx, 'all').faded, false);
  assert.equal(meaningFor(word({ content: false }), ctx, 'all').source, 'hidden', 'punctuation gets no gloss');
});

test('english to korean rows come in register order with the Korean first', () => {
  let s = reduce(emptyState(), started(1, 'en_to_ko'));
  s = reduce(s, { type: 'translation', id: 1, variant: 'casual', delta: '고마워' });
  s = reduce(s, { type: 'translation', id: 1, variant: 'polite', delta: '감사합니다' });
  assert.deepEqual(translationRows(s).map((r) => r.variant), ['polite', 'casual']);
  assert.equal(sentenceOf(s, 'target'), '감사합니다');
});

test('errors accumulate and do not end the request', () => {
  let s = reduce(emptyState(), started(1));
  s = reduce(s, { type: 'error', id: 1, stage: 'llm', message: 'boom' });
  assert.deepEqual(s.errors, ['boom']);
  assert.equal(s.finished, false);
});
