import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyState, reduce, openDeep, meaningFor, translationRows, sentenceOf, autoDeepTarget, snapshotCorrectionId, cardsLine, answeringText, shortModel } from '../core.js';

const word = (over = {}) => ({ index: 0, surface: '학교에서', lemma: '학교', content: true, state: 'new', lookups: 0, dict_glosses: ['school'], morphs: [], ...over });
const started = (id = 1, direction = 'ko_to_en') => ({ type: 'started', id, direction, capture: { text: '학교에서 공부해요' }, cached: false });

test('the right-click correction action is available only for a current OCR snapshot', () => {
  assert.equal(snapshotCorrectionId(emptyState()), null);
  for (const source of ['typed', 'selection', 'continuous']) assert.equal(snapshotCorrectionId({ id: 4, capture: { source } }), null);
  assert.equal(snapshotCorrectionId({ id: 4, capture: { source: 'ocr' } }), 4);
  assert.equal(snapshotCorrectionId({ id: null, capture: { source: 'ocr' } }), null);
});

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

test('an older start cannot replace a newer answer or its open deep dive', () => {
  let state = reduce(emptyState(), started(3));
  state = reduce(state, { type: 'translation', id: 3, variant: 'primary', delta: 'Current ' });
  state = openDeep(state, { id: 4, eojeol: word(), sentence: 's' });
  assert.equal(reduce(state, started(2)), state);
  state = reduce(state, { type: 'translation', id: 3, variant: 'primary', delta: 'answer.' });
  assert.equal(state.translations.primary, 'Current answer.');
  assert.equal(state.deep.id, 4);
});

test('a repeated start cannot erase text already streamed for that request', () => {
  let state = reduce(emptyState(), started(3));
  state = reduce(state, { type: 'translation', id: 3, variant: 'primary', delta: 'Current answer.' });
  assert.equal(reduce(state, started(3)), state);
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

test('a single-word scan label asks for its deep dive once the breakdown has arrived', () => {
  let s = reduce(emptyState(), started(7));
  assert.equal(autoDeepTarget(s, 7), null, 'nothing to open before the breakdown');
  s = reduce(s, { type: 'breakdown', id: 7, side: 'source', eojeols: [word({ index: 0, surface: '인벤토리' }), word({ index: 1, surface: '.', content: false })], romanization: '', pronunciation: '' });
  assert.equal(autoDeepTarget(s, 7)?.surface, '인벤토리');
  assert.equal(autoDeepTarget(s, 8), null, 'only for the request that asked');
});

test('two content words are not a single word, so no deep dive opens by itself', () => {
  let s = reduce(emptyState(), started(3));
  s = reduce(s, { type: 'breakdown', id: 3, side: 'source', eojeols: [word({ index: 0 }), word({ index: 1, surface: '공부' })], romanization: '', pronunciation: '' });
  assert.equal(autoDeepTarget(s, 3), null);
});

test('a Korean capture can become cards once its translation is done, with its request id and program', () => {
  const ev = [
    { type: 'started', id: 7, direction: 'ko_to_en', capture: { text: '상자를 열면 열쇠가 나옵니다', source: 'ocr', app: { exe: 'eden.exe' } }, cached: false },
    { type: 'translation', id: 7, variant: 'primary', delta: 'If you open the box' },
  ];
  let s = ev.reduce(reduce, emptyState());
  assert.equal(cardsLine(s), null, 'not while the translation streams');
  s = reduce(s, { type: 'translation_done', id: 7, variant: 'primary', text: 'If you open the box, a key comes out.' });
  assert.deepEqual(cardsLine(s), { id: 7, sentence: '상자를 열면 열쇠가 나옵니다', translation: 'If you open the box, a key comes out.', app: 'eden.exe' });
  const english = reduce(emptyState(), { ...ev[0], id: 8, direction: 'en_to_ko' });
  assert.equal(cardsLine(reduce(english, { type: 'translation_done', id: 8, variant: 'primary', text: '열쇠' })), null, 'English → Korean is not a Korean line to learn');
});

test('the panel names who answers a capture and a deep dive, the way Settings names connections', () => {
  let s = reduce(emptyState(), { type: 'started', id: 3, direction: 'ko_to_en', capture: { text: '안녕' }, cached: false });
  s = reduce(s, { type: 'answering', id: 3, connection: 'NAS', model: 'gemma4-12b', place: 'network' });
  assert.deepEqual(answeringText(s.answering), { text: 'NAS · gemma4-12b', title: 'Answered by NAS (gemma4-12b), on another computer. Settings → Captures chooses it.' });
  assert.equal(answeringText({ connection: 'local', model: 'unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL', place: 'this_pc' }).text, 'This PC · gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL');
  assert.equal(shortModel('D:\\models\\Hy-MT2-1.8B-Q4_K_M.gguf'), 'Hy-MT2-1.8B-Q4_K_M');
  assert.equal(answeringText(null), null);
  s = openDeep(s, { id: 4, eojeol: { surface: '안녕' }, sentence: '안녕', translation: 'hi', meaning: null });
  s = reduce(s, { type: 'answering', id: 4, connection: 'Claude', model: 'claude-sonnet-5-5', place: 'hosted' });
  assert.equal(s.deep.answering.connection, 'Claude', 'a deep dive keeps its own');
  assert.equal(s.answering.connection, 'NAS');
});
