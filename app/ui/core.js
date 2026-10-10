// Pure state logic for the panel. No DOM, no Tauri: testable with `node --test`.

export const emptyState = () => ({
  id: null,
  direction: null,
  capture: null,
  cached: false,
  source: null, // { eojeols, romanization, pronunciation }
  target: null,
  translations: {}, // variant -> text (streams in)
  done: {}, // variant -> true once complete
  glosses: { source: {}, target: {} }, // index -> { meaning, note }
  notes: [],
  errors: [],
  timings: {},
  finished: false,
  deep: null,
  notice: null,
});

const sideKey = (side) => (side === 'target' ? 'target' : 'source');

function updateWordStates(state, status) {
  // A lookup/save/known change must show immediately on every word with that lemma.
  for (const side of ['source', 'target']) {
    const list = state[side]?.eojeols;
    if (!list) continue;
    for (const e of list) {
      if ((e.lemma ?? e.surface) === status.key) {
        e.state = status.state;
        e.lookups = status.lookups;
      }
    }
  }
}

/** Apply one engine event. Returns a new state object (the old one is not mutated). */
export function reduce(prev, event) {
  const state = structuredClone(prev);
  const deepId = state.deep?.id;

  // Deep-dive events carry their own request id.
  if (deepId != null && event.id === deepId) {
    const d = state.deep;
    switch (event.type) {
      case 'word_state':
        d.status = event.status;
        updateWordStates(state, event.status);
        break;
      case 'deep_entries':
        d.entries = event.entries;
        d.conjugation = event.conjugation;
        break;
      case 'deep_delta':
        d.sections[event.section] = (d.sections[event.section] ?? '') + event.delta;
        break;
      case 'error':
        d.errors.push(event.message);
        break;
      case 'done':
        d.finished = true;
        break;
    }
    return state;
  }

  if (event.type === 'started') {
    const fresh = emptyState();
    fresh.id = event.id;
    fresh.direction = event.direction;
    fresh.capture = event.capture;
    fresh.cached = event.cached;
    return fresh;
  }
  if (event.id !== state.id) return prev; // superseded request: ignore

  switch (event.type) {
    case 'breakdown':
      state[sideKey(event.side)] = {
        eojeols: event.eojeols,
        romanization: event.romanization,
        pronunciation: event.pronunciation,
      };
      break;
    case 'translation':
      state.translations[event.variant] = (state.translations[event.variant] ?? '') + event.delta;
      break;
    case 'translation_done':
      state.translations[event.variant] = event.text;
      state.done[event.variant] = true;
      break;
    case 'gloss':
      state.glosses[sideKey(event.side)][event.gloss.index] = { meaning: event.gloss.meaning, note: event.gloss.note ?? null };
      break;
    case 'note':
      state.notes.push(event.text);
      break;
    case 'timing':
      state.timings[event.stage] = event.ms;
      break;
    case 'error':
      state.errors.push(event.message);
      break;
    case 'word_state':
      updateWordStates(state, event.status);
      break;
    case 'done':
      state.finished = true;
      break;
  }
  return state;
}

/** Open the deep-dive card for a word. */
export function openDeep(prev, { id, eojeol, sentence, translation, meaning }) {
  const state = structuredClone(prev);
  state.deep = { id, eojeol, sentence, translation, meaning, entries: [], conjugation: [], sections: {}, status: null, errors: [], finished: false };
  return state;
}

/**
 * The word to open a deep dive for, when the request `id` was started by clicking a scan label that
 * is a single word: its one content word, once the breakdown has arrived. Otherwise null.
 */
export function autoDeepTarget(state, id) {
  if (state.id !== id || !state.source) return null;
  const content = state.source.eojeols.filter((e) => e.content);
  return content.length === 1 ? content[0] : null;
}

export const closeDeep = (prev) => ({ ...structuredClone(prev), deep: null });

/**
 * What to print under a word.
 * `gloss` is the in-context meaning from the model (may not have arrived yet).
 * Returns { text, note, source: 'context'|'dictionary'|'pending'|'hidden', faded }.
 */
export function meaningFor(eojeol, gloss, mode) {
  if (!eojeol.content) return { text: '', note: null, source: 'hidden', faded: false };
  const known = eojeol.state === 'known';
  if (known && mode === 'hide_known') return { text: '', note: null, source: 'hidden', faded: true };
  const faded = known && mode === 'fade_known';
  if (gloss?.meaning) return { text: gloss.meaning, note: gloss.note, source: 'context', faded };
  const dict = eojeol.dict_glosses?.[0];
  if (dict) return { text: dict, note: null, source: 'dictionary', faded };
  return { text: '', note: null, source: known ? 'hidden' : 'pending', faded };
}

/** Variants to show, primary first. Korean → English has just one. */
export function translationRows(state) {
  const order = state.direction === 'en_to_ko' ? ['polite', 'formal', 'casual'] : ['primary'];
  return order
    .filter((v) => state.translations[v] != null)
    .map((v) => ({ variant: v, text: state.translations[v], done: !!state.done[v] }));
}

export const VARIANT_LABEL = { primary: '', polite: '해요체 · polite', formal: '합쇼체 · formal', casual: '반말 · casual' };

/** The sentence a deep dive should be grounded in. */
export function sentenceOf(state, side = 'source') {
  if (side === 'target') return state.translations.polite ?? state.translations.formal ?? state.translations.casual ?? '';
  return state.capture?.text ?? '';
}
