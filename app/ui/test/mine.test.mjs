import test from 'node:test';
import assert from 'node:assert/strict';

import { addLabel, ankiIsOff, answeredText, applySense, askFor, comesUp, deckOptions, formChoice, meaningHint, mineBlocker, mineDraft, mineRequest, outcomeSummary, queueLines, redraft, retryChoices, senseIndex, senseLabel } from '../mine-core.js';

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
  assert.equal(mineBlocker({ ...d, deck: '  ' }, sheet()), 'Choose a deck for the words.');
  assert.equal(mineBlocker({ ...d, sentenceDeck: ' ' }, sheet()), 'Choose a deck for the sentence.');
  assert.equal(mineBlocker({ ...d, deck: ' ', words: d.words.map((w) => ({ ...w, picked: false })) }, sheet()), null, 'a sentence alone needs only its own deck');
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

const twoDecks = { word_deck: 'Korean::Words', sentence_deck: 'Korean::Sentences', reverse: 'off', queue: { words: { deck: 'Korean::Words', waiting: 12, per_day: 3 }, sentences: { deck: 'Korean::Sentences', waiting: 5, per_day: 1 } } };

test('words and the sentence each start in their own deck and the request carries both', () => {
  const d = mineDraft(sheet(twoDecks));
  assert.deepEqual([d.deck, d.sentenceDeck], ['Korean::Words', 'Korean::Sentences'], 'the configured decks, not a deck per program');
  const req = mineRequest(d, {}, sheet(twoDecks));
  assert.deepEqual([req.deck, req.sentence_deck], ['Korean::Words', 'Korean::Sentences']);
  const one = mineDraft(sheet());
  assert.equal(one.sentenceDeck, one.deck, 'a sheet without a sentence deck uses the words deck');
  assert.equal(mineRequest(one, {}, sheet()).sentence_deck, 'Korean::Games');
});

test('before adding, the sheet says how many new cards wait in each deck and when these would come up', () => {
  const d = mineDraft(sheet(twoDecks));
  assert.deepEqual(queueLines(twoDecks.queue, d), [
    '12 new cards already wait in Korean::Words. At 3 a day, this word would come up in 4 days.',
    '5 new cards already wait in Korean::Sentences. At 1 a day, this sentence would come up in 5 days.',
  ]);
  const both = { ...d, words: d.words.map((w) => ({ ...w, picked: true })) };
  assert.equal(queueLines({ ...twoDecks.queue, words: { deck: 'Korean::Words', waiting: 13, per_day: 3 } }, both)[0], '13 new cards already wait in Korean::Words. At 3 a day, these 2 words would come up in 4 days.');
  assert.match(queueLines({ ...twoDecks.queue, words: { deck: 'Korean::Words', waiting: 14, per_day: 3 } }, both)[0], /these 2 words would come up in 4 to 5 days\.$/);
  assert.match(queueLines({ ...twoDecks.queue, words: { deck: 'Korean::Words', waiting: 0, per_day: 3 } }, d)[0], /^No new cards wait in Korean::Words\. At 3 a day, this word would come up today\.$/);
  assert.match(queueLines({ ...twoDecks.queue, words: { deck: 'Korean::Words', waiting: 1, per_day: 1 } }, d)[0], /^1 new card already waits in Korean::Words\..* tomorrow\.$/);
  assert.match(queueLines({ ...twoDecks.queue, words: { deck: 'Korean::Words', waiting: 2, per_day: 3 } }, both)[0], /from today, the last tomorrow\.$/);
  // Reverse cards double the word cards; nothing ticked says nothing; an unknown queue says nothing.
  assert.match(queueLines(twoDecks.queue, { ...d, reverse: true })[0], /these 2 cards would come up in 4 days/);
  assert.deepEqual(queueLines(twoDecks.queue, { ...d, sentenceCard: false, words: d.words.map((w) => ({ ...w, picked: false })) }), []);
  assert.deepEqual(queueLines(null, d), []);
  // A deck typed over is not answered with the other deck's count.
  assert.equal(queueLines(twoDecks.queue, { ...d, deck: 'Other' }).length, 1);
  assert.deepEqual(comesUp(12, 4, 3), [4, 5]);
  assert.equal(comesUp(3, 0, 3), null);
});

test('the sheet says which model chose the meanings, or that none was needed, or that none answered', () => {
  assert.equal(answeredText(sheet({ answered: { connection: 'local', model: 'gemma-4-12B', place: 'this_pc' } })), "Meanings chosen by This PC's model (gemma-4-12B), on this PC.");
  assert.equal(answeredText(sheet({ answered: { connection: 'NAS', model: 'gemma4-12b', place: 'network' } })), 'Meanings chosen by NAS (gemma4-12b), on another computer.');
  assert.equal(answeredText(sheet({ answered: null, attempts: [{ connection: 'Gemini', problem: 'quota' }] })), 'No model answered: the dictionary’s first sense is shown where a word has several.');
  assert.equal(answeredText(sheet({ answered: null, attempts: [], words: [{ key: '학교', source: 'one_sense' }] })), 'No model was needed: each word has one dictionary sense.');
  assert.equal(answeredText(sheet({ answered: null, attempts: [], words: [{ key: '저', source: 'first_sense' }] })), 'No model answered: the dictionary’s first sense is shown where a word has several.');
});

test('a line can be asked again with another connection, and this PC waits for live subtitles when it would have to load', () => {
  const retry = [{ connection: 'local', model: 'gemma', place: 'this_pc', after_live: true }, { connection: 'NAS', model: 'gemma4-12b', place: 'network', after_live: false }, { connection: 'Claude', model: 'sonnet', place: 'hosted', after_live: false }];
  const choices = retryChoices(sheet({ retry, answered: { connection: 'NAS', model: 'gemma4-12b', place: 'network' } }));
  assert.deepEqual(choices.map((c) => c.label), ["Ask This PC's model when live subtitles stop", 'Retry with NAS', 'Retry with Claude']);
  assert.deepEqual(choices.map((c) => c.current), [false, true, false]);
  assert.deepEqual(askFor(choices[0]), { connection: 'local', after_live: true });
  assert.deepEqual(askFor(choices[2]), { connection: 'Claude', after_live: false });
  assert.equal(retryChoices(sheet({ retry: [{ connection: 'local', model: 'g', place: 'this_pc', after_live: false }] }))[0].label, 'Retry with this PC');
  assert.deepEqual(retryChoices(sheet()), []);
});

test('asking again keeps your ticks, decks and what you typed, and takes the new meanings for the rest', () => {
  const first = sheet({ ...twoDecks, words: [{ key: '다시', surface: '다시는', meaning: 'again', meaning_es: 'otra vez', pick: true }, { key: '보다', surface: '볼', meaning: 'to see', meaning_es: 'ver', pick: false }] });
  const d = mineDraft(first);
  d.words[1].picked = true;
  d.words[1].meaning = 'to meet';
  d.translation = 'my own words';
  d.deck = 'Korean::Other';
  d.sentenceCard = false;
  const again = sheet({ ...twoDecks, translation: 'ignored', words: [{ key: '다시', surface: '다시는', meaning: 'never again', meaning_es: 'nunca más', pick: true }, { key: '보다', surface: '볼', meaning: 'to look at', meaning_es: 'mirar', pick: false }] });
  const next = redraft(d, first, again);
  assert.deepEqual([next.words[0].meaning, next.words[0].meaningEs], ['never again', 'nunca más'], 'the new choice replaces an untouched meaning');
  assert.deepEqual([next.words[1].meaning, next.words[1].meaningEs, next.words[1].picked], ['to meet', 'mirar', true], 'a typed meaning and a tick are kept');
  assert.deepEqual([next.translation, next.deck, next.sentenceDeck, next.sentenceCard], ['my own words', 'Korean::Other', 'Korean::Sentences', false]);
});

test('a dictionary form the model corrects is shown beside the splitter’s, and the card changes only when you say so', () => {
  const word = { key: '꾸다', surface: '주꾸마', corrected: '주다', meaning: 'to give', pick: true };
  const d = mineDraft(sheet({ words: [word] }));
  assert.equal(d.words[0].key, '꾸다', 'the card keeps the word splitter’s form at first');
  const choice = formChoice(word, d.words[0]);
  assert.deepEqual(choice, { text: 'The word splitter read 꾸다; the model reads 주다 in this line.', button: 'Use 주다', next: '주다' });
  d.words[0].key = choice.next;
  assert.equal(mineRequest(d, {}, sheet({ words: [word] })).words[0].key, '주다', 'the card then asks the corrected word');
  assert.deepEqual(formChoice(word, d.words[0]), { text: "The card will ask 주다 (the model's reading); the word splitter read 꾸다.", button: 'Use 꾸다', next: '꾸다' });
  assert.equal(formChoice({ key: '학교' }, { key: '학교' }), null);
  assert.equal(formChoice({ key: '학교', corrected: '학교' }, { key: '학교' }), null);
});
