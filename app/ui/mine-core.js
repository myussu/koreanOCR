// What the "Make cards" sheet in History decides, kept free of the page so it can be tested.

/** The sheet's starting choices: the suggested words ticked, the sentence card unless Anki has the line, the first
 *  suggested deck, reverse cards and sound as configured. */
export function mineDraft(sheet) {
  return {
    translation: sheet.translation ?? '',
    deck: sheet.suggestions?.[0]?.deck ?? '',
    reverse: sheet.reverse !== 'off',
    audio: !!sheet.audio,
    sentenceCard: !sheet.sentence_in_anki,
    words: (sheet.words ?? []).map((w) => ({ key: w.key, surface: w.surface, pos: w.pos ?? '', level: w.level ?? null, meaning: w.meaning ?? '', picked: !!w.pick })),
  };
}

const picked = (draft) => draft.words.filter((w) => w.picked);

/** Anki is switched off in Maru (rather than closed or unreachable): it can be turned on from the sheet. */
export const ankiIsOff = (sheet) => /switched off/i.test(sheet.anki_error ?? '');

/** Why the add button is off, or null when it can be pressed. */
export function mineBlocker(draft, sheet) {
  if (sheet.anki_error) return /^anki/i.test(sheet.anki_error) ? sheet.anki_error : `Anki: ${sheet.anki_error}`;
  if (!draft.deck.trim()) return 'Choose a deck.';
  if (!picked(draft).length && !draft.sentenceCard) return 'Tick a word or the sentence card.';
  const bare = picked(draft).find((w) => !w.meaning.trim());
  if (bare) return `Give “${bare.surface}” a meaning, or untick it.`;
  return null;
}

/** "Add 2 words and the sentence" — and how many cards that makes. */
export function addLabel(draft) {
  const n = picked(draft).length;
  const words = n === 1 ? '1 word' : `${n} words`;
  const text = n && draft.sentenceCard ? `Add ${words} and the sentence` : n ? `Add ${words}` : draft.sentenceCard ? 'Add the sentence' : 'Add';
  const cards = n * (draft.reverse ? 2 : 1) + (draft.sentenceCard ? 1 : 0);
  return { text, cards: cards === 1 ? '1 card' : `${cards} cards` };
}

/** The request for `mine_add` (nested fields are Rust's snake_case). */
export function mineRequest(draft, item, sheet) {
  return {
    saved_id: item.saved_id ?? null,
    sentence: sheet.sentence,
    translation: draft.translation.trim(),
    app: sheet.app ?? null,
    deck: draft.deck.trim(),
    reverse: draft.reverse,
    audio: draft.audio,
    sentence_card: draft.sentenceCard,
    words: picked(draft).map(({ key, surface, pos, level, meaning }) => ({ key, surface, pos, level, meaning: meaning.trim() })),
  };
}

/** "2 added · 1 already in Anki · 1 failed". */
export function outcomeSummary(reply) {
  const count = (r) => reply.outcomes.filter((o) => o.result === r).length;
  return [[count('added'), 'added'], [count('exists'), 'already in Anki'], [count('failed'), 'failed']]
    .filter(([n]) => n)
    .map(([n, what]) => `${n} ${what}`)
    .join(' · ');
}

/** The decks offered while typing: suggestions first (with why), then every other deck. */
export function deckOptions(sheet) {
  const seen = new Set();
  const out = [];
  for (const s of sheet.suggestions ?? []) if (!seen.has(s.deck.toLowerCase())) { seen.add(s.deck.toLowerCase()); out.push({ deck: s.deck, label: s.reason }); }
  for (const d of sheet.decks ?? []) if (!seen.has(d.toLowerCase())) { seen.add(d.toLowerCase()); out.push({ deck: d, label: '' }); }
  return out;
}
