// What the "Make cards" sheet in History decides, kept free of the page so it can be tested.

/** The sheet's starting choices: the suggested words ticked, the sentence card unless Anki has the line, the first
 *  suggested deck, reverse cards and sound as configured. */
export function mineDraft(sheet) {
  return {
    translation: sheet.translation ?? '',
    // Words and the sentence each have their deck (Settings → Making cards); a sheet from before that has one.
    deck: sheet.word_deck || (sheet.suggestions?.[0]?.deck ?? ''),
    sentenceDeck: sheet.sentence_deck || sheet.word_deck || (sheet.suggestions?.[0]?.deck ?? ''),
    reverse: sheet.reverse !== 'off',
    audio: !!sheet.audio,
    sentenceCard: !sheet.sentence_in_anki,
    words: (sheet.words ?? []).map((w) => ({ key: w.key, surface: w.surface, pos: w.pos ?? '', level: w.level ?? null, meaning: w.meaning ?? '', meaningEs: w.meaning_es ?? '', picked: !!w.pick })),
  };
}

const picked = (draft) => draft.words.filter((w) => w.picked);

/** Anki is switched off in Maru (rather than closed or unreachable): it can be turned on from the sheet. */
export const ankiIsOff = (sheet) => /switched off/i.test(sheet.anki_error ?? '');

/** Why the add button is off, or null when it can be pressed. */
export function mineBlocker(draft, sheet) {
  if (sheet.anki_error) return /^anki/i.test(sheet.anki_error) ? sheet.anki_error : `Anki: ${sheet.anki_error}`;
  if (picked(draft).some((w) => !String(w.key ?? '').trim())) return 'A picked word has no dictionary form.';
  if (picked(draft).length && !draft.deck.trim()) return 'Choose a deck for the words.';
  if (draft.sentenceCard && !(draft.sentenceDeck ?? draft.deck).trim()) return 'Choose a deck for the sentence.';
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
    deck: draft.deck.trim() || (draft.sentenceDeck ?? '').trim(),
    sentence_deck: (draft.sentenceDeck ?? draft.deck).trim(),
    reverse: draft.reverse,
    audio: draft.audio,
    sentence_card: draft.sentenceCard,
    words: picked(draft).map(({ key, surface, pos, level, meaning, meaningEs }) => ({ key, surface, pos, level, meaning: meaning.trim(), meaning_es: (meaningEs ?? '').trim() })),
  };
}

/** Where a word's meaning came from, in a few words (`source` is set when the learner's dictionary took part). */
export function meaningHint(word) {
  switch (word.source) {
    case 'one_sense': return 'the dictionary’s only sense';
    case 'chosen': return 'the dictionary sense chosen for this line';
    case 'first_sense': return 'the dictionary’s first sense: check it fits';
    case 'model': return 'not in the dictionary: the model’s words';
    default: return word.in_context ? 'as used in this line' : 'from the dictionary';
  }
}

const PLACE = { this_pc: 'this PC', network: 'another computer', hosted: 'a hosted service' };
const who = (connection) => (connection === 'local' ? "This PC's model" : connection);

/** Who gave the meanings, in one line under the words. */
export function answeredText(sheet) {
  const a = sheet.answered;
  if (a) return `Meanings chosen by ${who(a.connection)} (${a.model}), on ${PLACE[a.place] ?? 'a server'}.`;
  const asked = (sheet.words ?? []).some((w) => w.source && w.source !== 'one_sense');
  if (sheet.attempts?.length || asked) return 'No model answered: the dictionary’s first sense is shown where a word has several.';
  return (sheet.words ?? []).length ? 'No model was needed: each word has one dictionary sense.' : '';
}

/**
 * The connections a line can be asked again with. This PC's model, while it would have to load beside live
 * subtitles, is offered for when they stop instead of now.
 */
export function retryChoices(sheet) {
  return (sheet.retry ?? []).map((r) => ({
    connection: r.connection,
    afterLive: !!r.after_live,
    current: sheet.answered?.connection === r.connection,
    label: r.after_live ? `Ask ${who(r.connection)} when live subtitles stop` : `Retry with ${r.connection === 'local' ? 'this PC' : r.connection}`,
    detail: `${r.model} · ${PLACE[r.place] ?? ''}`,
  }));
}

/** The `ask` argument of `mine_sheet` for one of those choices. */
export const askFor = (choice) => ({ connection: choice.connection, after_live: choice.afterLive });

/**
 * The draft for a sheet that was asked again: the new meanings, with your ticks, decks, translation and options
 * kept. A meaning you typed yourself is kept too (it differs from what the earlier sheet proposed).
 */
export function redraft(oldDraft, oldSheet, newSheet) {
  const next = mineDraft(newSheet);
  const before = new Map(oldDraft.words.map((w, i) => [w.key, { draft: w, sheet: oldSheet.words[i] }]));
  for (const w of next.words) {
    const old = before.get(w.key);
    if (!old) continue;
    w.picked = old.draft.picked;
    if (old.draft.meaning !== (old.sheet.meaning ?? '')) w.meaning = old.draft.meaning;
    if (old.draft.meaningEs !== (old.sheet.meaning_es ?? '')) w.meaningEs = old.draft.meaningEs;
  }
  return { ...next, translation: oldDraft.translation, deck: oldDraft.deck, sentenceDeck: oldDraft.sentenceDeck, reverse: oldDraft.reverse, audio: oldDraft.audio, sentenceCard: oldDraft.sentenceCard };
}

const day = (n) => (n === 0 ? 'today' : n === 1 ? 'tomorrow' : `in ${n} days`);
/** "today", "in 4 days", "in 4 to 5 days", "from today, the last in 3 days". */
function inDays(first, last) {
  if (first === last) return day(first);
  return first >= 2 ? `in ${first} to ${last} days` : `from ${day(first)}, the last ${day(last)}`;
}

/** Mirrors `mining::comes_up`: the days until the first and the last of `adding` cards behind `waiting` others. */
export function comesUp(waiting, adding, perDay) {
  const n = Math.max(1, perDay || 1);
  return adding > 0 ? [Math.floor(waiting / n), Math.floor((waiting + adding - 1) / n)] : null;
}

/**
 * What to say before adding: how many new cards already wait in each deck and when these would come up, at the
 * per-day numbers set in Maru. A deck typed over since the count was made is left out until it is counted again.
 */
export function queueLines(queue, draft) {
  if (!queue) return [];
  const sameDeck = (a, b) => String(a ?? '').trim().toLowerCase() === String(b ?? '').trim().toLowerCase();
  const line = (q, deck, adding, what) => {
    if (!adding || !sameDeck(q.deck, deck)) return null;
    const [first, last] = comesUp(q.waiting, adding, q.per_day);
    const ahead = q.waiting === 0 ? `No new cards wait in ${q.deck}` : `${q.waiting} new card${q.waiting === 1 ? '' : 's'} already wait${q.waiting === 1 ? 's' : ''} in ${q.deck}`;
    return `${ahead}. At ${q.per_day} a day, ${adding === 1 ? `this ${what}` : `these ${adding} ${what}s`} would come up ${inDays(first, last)}.`;
  };
  const words = draft.words.filter((w) => w.picked).length * (draft.reverse ? 2 : 1);
  return [line(queue.words, draft.deck, words, draft.reverse ? 'card' : 'word'), line(queue.sentences, draft.sentenceDeck ?? draft.deck, draft.sentenceCard ? 1 : 0, 'sentence')].filter(Boolean);
}

/**
 * A word whose dictionary form the model reads differently from the word splitter: what the sheet says, and the
 * form its button would switch the card to. The card keeps the splitter's form until you press it.
 */
export function formChoice(word, draftWord) {
  if (!word.corrected || word.corrected === word.key) return null;
  const usingModel = draftWord.key === word.corrected;
  return {
    text: usingModel ? `The card will ask ${word.corrected} (the model's reading); the word splitter read ${word.key}.` : `The word splitter read ${word.key}; the model reads ${word.corrected} in this line.`,
    button: usingModel ? `Use ${word.key}` : `Use ${word.corrected}`,
    next: usingModel ? word.key : word.corrected,
  };
}

/**
 * A dictionary form typed by hand (the word splitter cut the word wrongly, as with dialect or a misread line): the
 * card is made for the typed form, whose meanings the dictionary did not choose. `null` when nothing was typed.
 */
export function typedFormNote(word, draftWord) {
  const typed = String(draftWord.key ?? '').trim();
  if (!typed) return 'Type the dictionary form, or tick the word off.';
  if (typed === word.key || typed === word.corrected) return null;
  return `The card will ask ${typed}. The meanings were chosen for ${word.key}: check them.`;
}

/** A dictionary sense as an option: "ship; boat · barco". */
export const senseLabel = (sense) => [sense.en, sense.es].filter(Boolean).join(' · ');

/** The dictionary sense shown for a word, as an index into its senses; -1 when the meanings were edited by hand. */
export const senseIndex = (word, draftWord) => (word.senses ?? []).findIndex((s) => s.en === draftWord.meaning && (s.es ?? '') === (draftWord.meaningEs ?? ''));

/** Take another dictionary sense: both meanings change together. */
export function applySense(draftWord, sense) {
  draftWord.meaning = sense.en ?? '';
  draftWord.meaningEs = sense.es ?? '';
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
