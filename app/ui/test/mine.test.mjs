import test from 'node:test';
import assert from 'node:assert/strict';

import { addLabel, ankiIsOff, applySense, deckOptions, meaningHint, mineBlocker, mineDraft, mineRequest, outcomeSummary, senseIndex, senseLabel } from '../mine-core.js';

const sheet = (over = {}) => ({
  sentence: '다시는 못 볼 줄 알았어요',
  translation: 'I thought I would never see you again.',
  app: 'game.exe',
  reverse: 'subdeck',
  audio: true,
  sentence_in_anki: false,
  anki_error: null,
  decks: ['Default', 'Korean', 'korean::games'],
  suggestions: [{ deck: 'Korean::Games', reason: 'what you chose last time', exists: true }, { deck: 'Korean', reason: 'your default deck', exists: true }],
  words: [
    { key: '다시', surface: '다시는', pos: 'adverb', level: 'TOPIK 1', meaning: 'ever again', pick: true },
    { key: '보다', surface: '볼', pos: 'verb', level: null, meaning: 'to see', pick: false },
  ],
  ...over,
});

test('the sheet starts with the suggested words, the first suggested deck, and the sentence card unless Anki has the line', () => {
  const d = mineDraft(sheet());
  assert.deepEqual(d.words.map((w) => w.picked), [true, false]);
  assert.equal(d.deck, 'Korean::Games');
  assert.equal(d.sentenceCard, true);
  assert.equal(d.reverse, true);
  assert.equal(mineDraft(sheet({ sentence_in_anki: true, reverse: 'off', suggestions: [] })).sentenceCard, false);
  assert.equal(mineDraft(sheet({ reverse: 'off' })).reverse, false);
  assert.equal(mineDraft(sheet({ suggestions: [] })).deck, '');
});

test('adding is blocked, with a reason, until Anki is there, a deck is chosen, something is ticked and every ticked word has a meaning', () => {
  const d = mineDraft(sheet());
  assert.equal(mineBlocker(d, sheet()), null);
  assert.match(mineBlocker(d, sheet({ anki_error: 'cannot reach AnkiConnect' })), /^Anki: cannot reach/);
  assert.equal(mineBlocker(d, sheet({ anki_error: 'Anki is switched off: turn it on in Settings → Anki' })), 'Anki is switched off: turn it on in Settings → Anki');
  assert.equal(mineBlocker({ ...d, deck: '  ' }, sheet()), 'Choose a deck.');
  assert.equal(mineBlocker({ ...d, sentenceCard: false, words: d.words.map((w) => ({ ...w, picked: false })) }, sheet()), 'Tick a word or the sentence card.');
  assert.equal(mineBlocker({ ...d, words: [{ ...d.words[0], meaning: ' ' }] }, sheet()), 'Give “다시는” a meaning, or untick it.');
});

test('the button says what will be added and how many cards that makes', () => {
  const d = mineDraft(sheet());
  assert.deepEqual(addLabel(d), { text: 'Add 1 word and the sentence', cards: '3 cards' });
  assert.deepEqual(addLabel({ ...d, reverse: false, sentenceCard: false }), { text: 'Add 1 word', cards: '1 card' });
  assert.deepEqual(addLabel({ ...d, words: d.words.map((w) => ({ ...w, picked: true })) }), { text: 'Add 2 words and the sentence', cards: '5 cards' });
  assert.deepEqual(addLabel({ ...d, words: [] }), { text: 'Add the sentence', cards: '1 card' });
});

test('the request carries only the ticked words, trimmed, and the line it came from', () => {
  const d = mineDraft(sheet());
  d.words[0].meaning = '  ever again ';
  d.translation = ' I never thought I would see you. ';
  const req = mineRequest(d, { saved_id: 12 }, sheet());
  assert.deepEqual(req.words, [{ key: '다시', surface: '다시는', pos: 'adverb', level: 'TOPIK 1', meaning: 'ever again', meaning_es: '' }]);
  assert.equal(req.translation, 'I never thought I would see you.');
  assert.equal(req.saved_id, 12);
  assert.equal(req.sentence_card, true);
  assert.equal(req.app, 'game.exe');
  assert.equal(mineRequest(d, {}, sheet()).saved_id, null);
});

test('results are summed up in plain words', () => {
  const reply = { outcomes: [{ result: 'added' }, { result: 'added' }, { result: 'exists' }, { result: 'failed' }] };
  assert.equal(outcomeSummary(reply), '2 added · 1 already in Anki · 1 failed');
  assert.equal(outcomeSummary({ outcomes: [{ result: 'exists' }] }), '1 already in Anki');
});

test('deck choices list suggestions first, then every other deck once', () => {
  assert.deepEqual(deckOptions(sheet()).map((d) => d.deck), ['Korean::Games', 'Korean', 'Default']);
});

test('only Anki switched off in Maru offers to turn it on; a closed Anki does not', () => {
  assert.equal(ankiIsOff(sheet({ anki_error: 'Anki is switched off: turn it on in Settings → Anki' })), true);
  assert.equal(ankiIsOff(sheet({ anki_error: 'cannot reach AnkiConnect at http://127.0.0.1:8765' })), false);
  assert.equal(ankiIsOff(sheet()), false);
});

test('with the learner’s dictionary each word carries a Spanish meaning into the request, trimmed', () => {
  const s = sheet({ words: [{ key: '배', surface: '배가', pos: 'noun', level: null, meaning: 'ship; boat', meaning_es: 'barco', source: 'chosen', pick: true }] });
  const d = mineDraft(s);
  assert.equal(d.words[0].meaningEs, 'barco');
  d.words[0].meaningEs = ' barco, nave ';
  assert.deepEqual(mineRequest(d, {}, s).words, [{ key: '배', surface: '배가', pos: 'noun', level: null, meaning: 'ship; boat', meaning_es: 'barco, nave' }]);
});

test('each meaning says where it came from, and without the learner’s dictionary the old wording stays', () => {
  assert.equal(meaningHint({ source: 'one_sense' }), 'the dictionary’s only sense');
  assert.equal(meaningHint({ source: 'chosen', in_context: true }), 'the dictionary sense chosen for this line');
  assert.match(meaningHint({ source: 'first_sense' }), /check it fits/);
  assert.match(meaningHint({ source: 'model', in_context: true }), /not in the dictionary/);
  assert.equal(meaningHint({ source: null, in_context: true }), 'as used in this line');
  assert.equal(meaningHint({ in_context: false }), 'from the dictionary');
});

test('choosing another dictionary sense changes both meanings, and an edited meaning matches no sense', () => {
  const word = { senses: [{ en: 'belly; stomach', es: 'barriga' }, { en: 'ship; boat', es: 'barco' }, { en: 'womb', es: '' }] };
  const d = { meaning: 'belly; stomach', meaningEs: 'barriga' };
  assert.equal(senseIndex(word, d), 0);
  applySense(d, word.senses[1]);
  assert.deepEqual(d, { meaning: 'ship; boat', meaningEs: 'barco' });
  assert.equal(senseIndex(word, d), 1);
  d.meaning = 'the ship';
  assert.equal(senseIndex(word, d), -1);
  assert.equal(senseLabel(word.senses[1]), 'ship; boat · barco');
  assert.equal(senseLabel(word.senses[2]), 'womb', 'a sense without Spanish shows the English alone');
  assert.equal(senseIndex({}, d), -1, 'a word without dictionary senses');
});
