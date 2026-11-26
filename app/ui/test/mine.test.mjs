import test from 'node:test';
import assert from 'node:assert/strict';

import { addLabel, ankiIsOff, deckOptions, mineBlocker, mineDraft, mineRequest, outcomeSummary } from '../mine-core.js';

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
  assert.deepEqual(req.words, [{ key: '다시', surface: '다시는', pos: 'adverb', level: 'TOPIK 1', meaning: 'ever again' }]);
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
