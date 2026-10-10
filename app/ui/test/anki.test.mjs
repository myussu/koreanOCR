import test from 'node:test';
import assert from 'node:assert/strict';
import { cleanDeck, deckChoices, tagOf, addTags, removeTag, intervalText, addedText, reverseChoice } from '../anki.js';

const decks = ['Default', 'Korean', 'Korean::Games', 'Korean::Games::Elden Ring', 'Korean::YouTube', 'Spanish'];
const sugg = [
  { deck: 'Korean::Games::Elden Ring', reason: 'what you chose last time in Elden Ring', exists: true },
  { deck: 'Korean::Hades', reason: 'named after Hades', exists: false },
  { deck: 'Korean', reason: 'your default deck', exists: true },
];

test('deck names are tidied like the app does', () => {
  assert.equal(cleanDeck(' Korean :: Games ::'), 'Korean::Games');
  assert.equal(cleanDeck('::'), '');
  assert.equal(cleanDeck(null), '');
});

test('an empty box offers the suggestions in order', () => {
  const rows = deckChoices('', decks, sugg);
  assert.deepEqual(rows.map((r) => [r.deck, r.kind, r.exists]), [['Korean::Games::Elden Ring', 'suggest', true], ['Korean::Hades', 'suggest', false], ['Korean', 'suggest', true]]);
  assert.match(rows[0].reason, /last time/);
});

test('typing finds existing decks, best matches first', () => {
  const rows = deckChoices('korean::g', decks, sugg);
  assert.deepEqual(rows.map((r) => [r.deck, r.kind]), [['Korean::Games', 'existing'], ['Korean::Games::Elden Ring', 'existing'], ['korean::g', 'new']]);
  const parts = deckChoices('elden', decks, sugg).map((r) => r.deck);
  assert.equal(parts[0], 'Korean::Games::Elden Ring', 'matches a part of the name');
  assert.equal(deckChoices('ring', decks, sugg)[0].deck, 'Korean::Games::Elden Ring', 'or just contain it');
});

test('a name that is not a deck can be created, and an exact deck cannot', () => {
  const fresh = deckChoices('Korean::Hades', decks, sugg);
  assert.deepEqual(fresh.map((r) => [r.deck, r.kind]), [['Korean::Hades', 'new']], 'no matches: only the create row');
  const mixed = deckChoices('Korean::Gam', decks, sugg);
  assert.equal(mixed.at(-1).kind, 'new');
  assert.equal(mixed.at(-1).deck, 'Korean::Gam');
  const exact = deckChoices('korean::games', decks, sugg);
  assert.ok(exact.every((r) => r.kind === 'existing'), 'an existing deck (any capitals) is not offered as new');
  assert.equal(exact[0].deck, 'Korean::Games');
  assert.deepEqual(deckChoices('  ::  ', decks, sugg).map((r) => r.kind), ['suggest', 'suggest', 'suggest'], 'only stray colons count as empty');
});

test('the list stays short', () => {
  const many = Array.from({ length: 40 }, (_, i) => `Korean::Deck ${i}`);
  assert.equal(deckChoices('deck', many, []).length, 8, '7 matches and the create row');
});

test('tags are made safe and not repeated', () => {
  assert.equal(tagOf(' game::Elden Ring '), 'game::Elden_Ring');
  assert.equal(tagOf('a,b"c'), 'a_b_c');
  assert.equal(tagOf('::'), '');
  assert.deepEqual(addTags(['maru'], 'topik-1, Maru  new word'), ['maru', 'topik-1', 'new', 'word']);
  assert.deepEqual(addTags([], '   '), []);
  assert.deepEqual(removeTag(['a', 'b'], 'a'), ['b']);
});

test('words for how long a card has been held', () => {
  assert.equal(intervalText(0), 'still being learned');
  assert.equal(intervalText(1), 'held for 1 day');
  assert.equal(intervalText(12), 'held for 12 days');
  assert.equal(intervalText(60), 'held for 2 months');
  assert.equal(intervalText(800), 'held for 2.2 years');
});

test('what is said after adding', () => {
  assert.equal(addedText({ result: 'added', deck: 'Korean', reverse_deck: null, cards: 1, sync: false }), '✓ Added to Korean (1 card)');
  assert.equal(addedText({ result: 'added', deck: 'K', reverse_deck: 'K::Reverse', cards: 2, sync: true }), '✓ Added to K (2 cards; the English → Korean one is in K::Reverse) · syncing with AnkiWeb');
  assert.equal(addedText({ result: 'exists' }), '');
});

test('the reverse box overrides the configured choice only when it must', () => {
  assert.equal(reverseChoice(true, 'subdeck'), null);
  assert.equal(reverseChoice(true, 'same_deck'), null);
  assert.equal(reverseChoice(true, 'off'), 'subdeck', 'asked for it although it is off by default');
  assert.equal(reverseChoice(false, 'subdeck'), 'off');
});
