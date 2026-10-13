import { invoke, listen, replayFromQuery, mockCommands, inTauri } from './bridge.js';
import { emptyState, reduce, openDeep, closeDeep, meaningFor, translationRows, sentenceOf, autoDeepTarget, readableText, sentenceRanges, snapshotCorrectionId, VARIANT_LABEL } from './core.js';
import { contextMenu } from './context-menu.js';
import { Reader } from './reader.js';
import { cleanDeck, deckChoices, addTags, removeTag, intervalText, addedText, reverseChoice, canSubmitAnki } from './anki.js';

const card = document.getElementById('card');
const snapshotMenu = contextMenu();
let state = emptyState();
let cfg = { gloss_mode: 'fade_known', font_size_px: 18, show_romanization: true, show_pronunciation: true, show_grammar_formula: true, theme: 'auto', anki: false, tts: false, opacity: 0.97 };
let pinned = false;
let deepSide = 'source';

// ───────────── reading aloud ─────────────

function playable(clip) {
  const audio = new Audio(`data:${clip.mime};base64,${clip.data}`);
  return {
    play: () => audio.play(),
    pause: () => audio.pause(),
    release: () => {
      audio.removeAttribute('src');
      audio.load();
    },
    set onended(fn) {
      audio.onended = fn;
    },
    get onended() {
      return audio.onended;
    },
  };
}

let wasReading = false;
const reader = new Reader({
  fetchClip: (text) => invoke('tts_clip', { text }),
  createAudio: playable,
  onChange: () => {
    // the app keeps the panel up while it reads
    if (reader.active !== wasReading) {
      wasReading = reader.active;
      invoke('tts_reading', { active: wasReading }).catch(() => {});
    }
    render();
  },
});

async function readAloud(text = readableText(state)) {
  if (!text) return;
  try {
    const sentences = await invoke('tts_plan', { text });
    reader.start(sentences);
  } catch (e) {
    flash(String(e));
  }
}

/** Which of the words shown are in the sentence being read now. */
function speakingWords(side) {
  if (!reader.active || reader.index < 0) return new Set();
  const eojeols = state[side]?.eojeols;
  const ranges = eojeols && sentenceRanges(eojeols, reader.sentences);
  const range = ranges?.[reader.index];
  const out = new Set();
  if (range) for (let i = range[0]; i < range[1]; i++) out.add(eojeols[i].index);
  return out;
}

function readerStrip() {
  if (reader.status === 'idle') return null;
  const n = reader.sentences.length;
  const busy = reader.status === 'loading';
  return h(
    'div',
    { class: `reader ${reader.status}` },
    h('button', { class: 'icon', title: 'previous sentence', onclick: () => reader.prev(), disabled: reader.index <= 0, text: '⏮' }),
    h('button', { class: 'icon', title: reader.status === 'paused' ? 'continue' : 'pause', onclick: () => reader.toggle(), text: reader.status === 'paused' || reader.status === 'error' ? '▶' : '⏸' }),
    h('button', { class: 'icon', title: 'next sentence', onclick: () => reader.next(), disabled: reader.index + 1 >= n, text: '⏭' }),
    h('button', { class: 'icon', title: 'say this sentence again', onclick: () => reader.replay(), text: '↺' }),
    h('button', { class: 'icon', 'aria-pressed': String(reader.repeat > 1), title: reader.repeat > 1 ? `each sentence ${reader.repeat}×: click to change` : 'say each sentence more than once', onclick: () => reader.setRepeat(reader.repeat >= 3 ? 1 : reader.repeat + 1), text: reader.repeat > 1 ? `${reader.repeat}×` : '1×' }),
    h('span', { class: 'count', text: `${reader.index + 1}/${n}` }),
    h('span', { class: `now${busy ? ' busy' : ''}`, title: reader.text, text: reader.status === 'error' ? reader.error : busy ? `${reader.text} …` : reader.text }),
    h('button', { class: 'icon', title: 'stop', onclick: () => reader.stop(), text: '⏹' }),
  );
}

/** Tiny DOM helper. Text always goes through textContent: model output is untrusted. */
function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'text') el.textContent = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null && kid !== false) el.append(kid);
  return el;
}

const SOURCE_LABEL = { accessibility: 'selection', clipboard: 'copied', ocr: 'OCR', hover: 'hover', continuous: 'live', typed: 'typed' };

// ───────────── rendering ─────────────

function wordColumn(e, side, speaking = new Set()) {
  if (!e.content) return h('span', { class: `punct${speaking.has(e.index) ? ' speaking' : ''}`, text: e.surface });
  const gloss = state.glosses[side][e.index];
  const m = meaningFor(e, gloss, cfg.gloss_mode);
  const open = state.deep && state.deep.eojeol.index === e.index && deepSide === side;
  return h(
    'div',
    {
      class: `w ${e.state ?? 'new'}${open ? ' open' : ''}${speaking.has(e.index) ? ' speaking' : ''}`,
      title: [e.formula, m.note].filter(Boolean).join('\n') || null,
      onclick: () => startDeep(e, side),
    },
    h('div', { class: 'ko', text: e.surface.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, '') }),
    cfg.show_romanization && h('div', { class: 'rom', text: e.romanization }),
    cfg.show_pronunciation && e.pronunciation && e.pronunciation !== e.surface.replace(/[^\p{L}\p{N}]/gu, '') && h('div', { class: 'pron', text: `[${e.pronunciation}]` }),
    m.source !== 'hidden' && h('div', { class: `mean ${m.source}`, text: m.source === 'pending' ? '···' : m.text }),
    (e.level || e.state === 'learning' || e.state === 'known') &&
      h('div', { class: 'badges' }, e.level && h('span', { class: 'chip level', text: e.level }), e.state !== 'new' && h('span', { class: `dot ${e.state}`, title: e.state === 'known' ? 'you know this' : `looked up ${e.lookups}×` })),
  );
}

function wordsBlock(side) {
  const data = state[side];
  if (!data) return null;
  const speaking = speakingWords(side);
  return h('div', { class: 'words' }, data.eojeols.map((e) => wordColumn(e, side, speaking)));
}

function translationBlock() {
  const rows = translationRows(state);
  if (!rows.length && !state.notes.length) return null;
  const korean = state.direction === 'en_to_ko';
  return h(
    'div',
    { class: 'tr' },
    rows.map((r) =>
      h(
        'div',
        { class: `row ${r.variant}${korean ? ' ko' : ''}` },
        VARIANT_LABEL[r.variant] && h('span', { class: 'lbl', text: VARIANT_LABEL[r.variant] }),
        h('span', { class: `txt${r.done ? '' : ' streaming'}`, text: r.text }),
      ),
    ),
    state.notes.length > 0 && h('div', { class: 'notes' }, state.notes.map((n) => h('div', { text: n }))),
  );
}

/** A word as it should be said: without the punctuation stuck to it. */
const spoken = (surface) => surface.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, '');

function deepBlock() {
  const d = state.deep;
  if (!d) return null;
  const e = d.eojeol;
  const key = e.lemma ?? e.surface;
  const entry = d.entries[0];
  const steps = d.conjugation;
  const saved = d.status?.saved;
  const known = d.status?.state === 'known';
  return h(
    'div',
    { class: 'deep' },
    h('h3', {}, key, h('span', { class: 'chip', text: e.pos }), entry?.hanja && h('span', { class: 'hanja', text: entry.hanja }), (entry?.level ?? e.level) && h('span', { class: 'chip level', text: entry?.level ?? e.level }), h('span', { class: 'speak-word' }, cfg.tts && h('button', { class: 'icon', title: 'hear this word', onclick: () => readAloud(spoken(e.surface)), text: '🔊' }), cfg.tts && spoken(e.surface) !== key && h('button', { class: 'icon', title: `hear the dictionary form, ${key}`, onclick: () => readAloud(key), text: '🔊 ' + key })), h('button', { class: 'icon', style: 'margin-left:auto', title: 'close', onclick: () => (closeAnki(), (ankiUI = freshAnki()), update(closeDeep(state))), text: '✕' })),
    steps.length > 1 && [
      h('h4', { text: 'How this form is built' }),
      h('div', { class: 'built' }, steps.flatMap((s, i) => [i > 0 && h('span', { class: 'plus', text: '+' }), h('div', { class: 'step' }, h('b', { text: s.form }), h('span', { text: s.label }))])),
    ],
    d.entries.length > 0 && [
      h('h4', { text: 'Dictionary' }),
      d.entries.slice(0, 3).map((en) => h('div', { class: 'entry' }, h('span', { class: 'pos', text: en.pos }), en.senses.slice(0, 3).map((s) => s.gloss).join(' · '))),
    ],
    d.sections.usage && [h('h4', { text: 'In this sentence' }), h('div', { class: 'text', text: d.sections.usage.trim() })],
    d.sections.examples && [h('h4', { text: 'Examples' }), h('div', { class: 'text ko', text: d.sections.examples.trim() })],
    d.sections.nuance && [h('h4', { text: 'Don’t confuse with' }), h('div', { class: 'text', text: d.sections.nuance.trim() })],
    !d.finished && d.errors.length === 0 && h('div', { class: 'status', text: 'thinking…' }),
    d.errors.map((m) => h('div', { class: 'err', text: m })),
    h(
      'div',
      { class: 'actions' },
      !cfg.anki && h('button', { class: `btn${saved ? '' : ' primary'}`, onclick: () => act('save_word', { key }), disabled: saved, text: saved ? '✓ in your review queue' : 'Save to review' }),
      h('button', { class: 'btn', onclick: () => act('set_known', { key, known: !known }), text: known ? 'Not known after all' : 'I know this' }),
      d.status && h('span', { class: 'status', text: `looked up ${d.status.lookups}×${d.status.saved ? ' · queued' : ''}` }),
      d.message && h('span', { class: 'status', text: d.message }),
    ),
  );
}

function render() {
  const s = state;
  const kids = [];
  if (s.notice) kids.push(h('div', { class: `notice ${s.notice.level}`, text: s.notice.text }));
  if (s.id != null) {
    const dir = s.direction === 'en_to_ko' ? 'English → 한국어' : '한국어 → English';
    kids.push(
      h(
        'div',
        { class: 'bar', title: 'drag to move' },
        h('span', { class: 'grip', text: '⠿' }),
        h('span', { class: 'dir', text: dir }),
        h('span', { class: 'chip', text: SOURCE_LABEL[s.capture?.source] ?? s.capture?.source ?? '' }),
        s.cached && h('span', { class: 'chip', text: 'cached' }),
        s.capture?.confidence != null && s.capture.confidence < 0.8 && h('span', { class: 'chip', title: 'OCR was unsure; check the text', text: 'OCR uncertain' }),
        h('span', { class: 'grow' }),
        cfg.tts && readableText(s) && h('button', { class: 'icon', 'aria-pressed': String(reader.active), title: reader.active ? 'stop reading' : 'read aloud', onclick: () => (reader.active ? reader.stop() : readAloud()), text: '🔊' }),
        h('button', { class: 'icon', 'aria-pressed': String(pinned), title: 'keep this open', onclick: togglePin, text: '📌' }),
        h('button', { class: 'icon', title: 'close', onclick: () => invoke('dismiss_panel'), text: '✕' }),
      ),
    );
    kids.push(readerStrip());
    if (s.direction === 'en_to_ko') kids.push(translationBlock(), s.target && h('div', { class: 'tr' }, wordsBlock('target')));
    else kids.push(wordsBlock('source') ?? h('div', { class: 'empty', text: 'Analysing…' }), translationBlock());
    kids.push(deepBlock());
    if (s.deep) kids.push(...[ankiBlock()].flat());
    for (const m of s.errors) kids.push(h('div', { class: 'err', text: m }));
    const t = s.timings;
    if (s.finished && Object.keys(t).length)
      kids.push(h('div', { class: 'foot' }, [['analysis', t.morphology], ['first word', t.first_token], ['total', t.total]].filter(([, v]) => v != null).map(([k, v]) => h('span', { text: `${k} ${v} ms` }))));
  }
  card.replaceChildren(...kids.filter(Boolean));
  card.style.maxWidth = '';
  reportSize();
  restoreTyping();
}

// ───────────── actions ─────────────

function update(next) {
  if (next.id !== state.id) snapshotMenu.close();
  state = next;
  render();
}

function togglePin() {
  pinned = !pinned;
  invoke('set_pinned', { pinned });
  render();
}

async function startDeep(eojeol, side) {
  deepSide = side;
  const sentence = side === 'target' ? sentenceOf(state, 'target') : sentenceOf(state);
  const translation = side === 'target' ? state.capture?.text : state.translations.primary;
  const gloss = state.glosses[side][eojeol.index];
  const meaning = gloss?.meaning ?? eojeol.dict_glosses?.[0] ?? null;
  const id = await invoke('deep_dive', { request: { eojeol, sentence, translation: translation ?? null, meaning, app: state.capture?.app ?? null } });
  update(openDeep(state, { id, eojeol, sentence, translation, meaning }));
  if (cfg.anki) loadAnki(state.deep);
}

async function act(cmd, args) {
  try {
    const status = await invoke(cmd, args);
    if (status && state.deep) update(reduce(state, { type: 'word_state', id: state.deep.id, status }));
  } catch (e) {
    flash(String(e));
  }
}

function flash(message) {
  if (!state.deep) return;
  const next = structuredClone(state);
  next.deep.message = message;
  update(next);
}

// ───────────── adding to Anki ─────────────

// The panel's window never takes the keyboard (clicking it must not clear the selection you made
// elsewhere), so while a text box here is in use the app is asked to allow it, and told when done.
let typing = false;
function setTyping(on) {
  if (typing === on) return;
  typing = on;
  invoke('panel_typing', { on }).catch(() => {});
}

const freshAnki = () => ({ dirty: false, word: null, open: false, overview: null, error: null, loading: false, query: '', deck: '', tags: [], tagText: '', reverse: true, audio: true, busy: false, reply: null, message: null, focus: null, caret: 0 });
let ankiUI = freshAnki();

async function loadAnki(d) {
  const key = d.eojeol.lemma ?? d.eojeol.surface;
  ankiUI = { ...freshAnki(), word: key, loading: true };
  render();
  try {
    const o = await invoke('anki_overview', { key, app: state.capture?.app?.exe ?? null });
    if (ankiUI.word !== key) return;
    ankiUI.overview = o;
    ankiUI.loading = false;
    ankiUI.deck = o.suggestions[0]?.deck ?? '';
    ankiUI.query = ankiUI.deck;
    ankiUI.tags = [...o.tags];
    ankiUI.reverse = o.reverse !== 'off';
    ankiUI.audio = o.audio;
    if (o.error) ankiUI.error = o.error;
  } catch (e) {
    if (ankiUI.word !== key) return;
    ankiUI.loading = false;
    ankiUI.error = String(e);
  }
  render();
}

function closeAnki() {
  ankiUI.open = false;
  ankiUI.focus = null;
  setTyping(false);
  render();
}

async function submitAnki() {
  const u = ankiUI;
  if (!canSubmitAnki(u)) return;
  const deck = cleanDeck(u.query || u.deck);
  if (!deck) {
    u.message = 'Choose or type a deck first.';
    return render();
  }
  u.busy = true;
  u.message = null;
  render();
  try {
    const reply = await invoke('anki_add', { request: { key: u.word, deck, tags: addTags(u.tags, u.tagText), reverse: reverseChoice(u.reverse, u.overview?.reverse), app: state.capture?.app?.exe ?? null, audio: u.audio } });
    u.reply = reply;
    u.message = reply.result === 'added' ? addedText(reply) : `Already in Anki, in ${reply.deck}.`;
    if (reply.note) u.message += ` ${reply.note}`;
    if (reply.warning) u.message += ` ${reply.warning}`;
    u.open = reply.result === 'exists';
    if (reply.result === 'added' && !reply.warning) {
      u.overview = { ...u.overview, existing: { note_id: 0, deck: reply.deck, interval_days: 0 } };
      closeAnki();
    }
    if (reply.warning) u.open = true;
  } catch (e) {
    u.message = String(e);
  }
  u.busy = false;
  render();
}

async function addExample() {
  const u = ankiUI;
  if (u.busy) return;
  u.busy = true;
  render();
  try {
    u.message = (await invoke('anki_add_example', { key: u.word, audio: u.audio })) ? '✓ Added this sentence to the card.' : 'The card already has this sentence.';
  } catch (e) {
    u.message = String(e);
  }
  u.busy = false;
  render();
}

function pick(deck) {
  ankiUI.deck = deck;
  ankiUI.query = deck;
  ankiUI.dirty = false;
  ankiUI.focus = 'deck';
  render();
}

/** Text boxes get the keyboard on press, then keep it while the popover is open. */
function typingInput(kind, props) {
  const input = h('input', {
    type: 'text',
    spellcheck: 'false',
    autocomplete: 'off',
    'data-anki-input': kind,
    onpointerdown: () => setTyping(true),
    onfocus: () => {
      ankiUI.focus = kind;
    },
    ...props,
  });
  return input;
}

function ankiBlock() {
  const u = ankiUI;
  if (!cfg.anki || !u.word) return null;
  const o = u.overview;
  const bits = [];
  if (u.loading) bits.push(h('span', { class: 'status', text: 'asking Anki…' }));
  else if (u.error && !o?.connected) {
    bits.push(h('button', { class: 'btn', title: u.error, onclick: () => state.deep && loadAnki(state.deep), text: 'Anki is not reachable · retry' }));
    bits.push(h('span', { class: 'status', text: 'Open Anki (with the AnkiConnect add-on).' }));
  } else if (o?.existing) {
    bits.push(h('span', { class: 'chip level', title: 'this word is already a card', text: `✓ in Anki · ${o.existing.deck}${o.existing.interval_days ? ` · ${intervalText(o.existing.interval_days)}` : ''}` }));
    bits.push(h('button', { class: 'btn', disabled: u.busy, title: 'put the sentence you just met it in on its card', onclick: addExample, text: '＋ this sentence' }));
  } else if (o) {
    bits.push(h('button', { class: `btn${u.open ? '' : ' primary'}`, 'aria-expanded': String(u.open), onclick: () => (u.open ? closeAnki() : ((u.open = true), (u.focus = 'deck'), render())), text: 'Add to Anki ▾' }));
  }
  if (u.message) bits.push(h('span', { class: 'status', text: u.message }));
  const row = h('div', { class: 'actions anki-row' }, bits);
  if (!u.open || !o) return row;

  // until you type, all the suggestions show (the box starts with the best one filled in)
  const choices = deckChoices(u.dirty ? u.query : '', o.decks, o.suggestions);
  const deckInput = typingInput('deck', {
    class: 'anki-deck',
    placeholder: 'Pick a deck or type a new one (Korean::Games::…)',
    value: u.query,
    oninput: (e) => {
      u.query = e.target.value;
      u.dirty = true;
      u.caret = e.target.selectionStart;
      render();
    },
    onkeydown: (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        const first = choices[0];
        if (first && u.query.trim() && cleanDeck(u.query) !== first.deck && first.kind === 'existing') pick(first.deck);
        else submitAnki();
      }
    },
  });
  const tagInput = typingInput('tag', {
    class: 'anki-tag',
    placeholder: '+ tag',
    value: u.tagText,
    oninput: (e) => {
      u.tagText = e.target.value;
      u.caret = e.target.selectionStart;
    },
    onkeydown: (e) => {
      if (e.key === 'Enter' || e.key === ',' || e.key === ' ') {
        if (!u.tagText.trim()) return;
        e.preventDefault();
        u.tags = addTags(u.tags, u.tagText);
        u.tagText = '';
        render();
      }
    },
  });
  const here = cleanDeck(u.query);
  return [
    row,
    h(
      'div',
      { class: 'anki-box' },
      h('label', { class: 'anki-label', text: 'Deck' }),
      deckInput,
      h(
        'div',
        { class: 'anki-choices', role: 'listbox' },
        choices.map((c) =>
          h(
            'button',
            { class: `anki-choice ${c.kind}${cleanDeck(c.deck).toLowerCase() === here.toLowerCase() ? ' on' : ''}`, role: 'option', onpointerdown: (e) => (e.preventDefault(), pick(c.deck)) },
            h('span', { class: 'name', text: c.kind === 'new' ? `Create “${c.deck}”` : c.deck }),
            c.reason && c.kind !== 'new' && h('span', { class: 'why', text: `${c.reason}${c.exists ? '' : ' · new'}` }),
          ),
        ),
      ),
      h('label', { class: 'anki-label', text: 'Tags' }),
      h('div', { class: 'anki-tags' }, u.tags.map((t) => h('span', { class: 'tagchip' }, t, h('button', { title: 'remove', onclick: () => ((u.tags = removeTag(u.tags, t)), render()), text: '×' }))), tagInput),
      h(
        'div',
        { class: 'anki-opts' },
        h('label', {}, h('input', { type: 'checkbox', checked: u.reverse, onchange: (e) => (u.reverse = e.target.checked) }), ' also English → Korean'),
        o.audio && h('label', {}, h('input', { type: 'checkbox', checked: u.audio, onchange: (e) => (u.audio = e.target.checked) }), ' spoken audio'),
      ),
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn primary', disabled: u.busy || !here, onclick: submitAnki, text: u.busy ? 'Adding…' : `Add to ${here || 'a deck'}` }),
        h('button', { class: 'btn', onclick: closeAnki, text: 'Cancel' }),
      ),
    ),
  ];
}

/** The text box that had the keyboard keeps it (and the cursor) when the panel redraws. */
function restoreTyping() {
  if (!ankiUI.open || !ankiUI.focus) return;
  const el = card.querySelector(`[data-anki-input="${ankiUI.focus}"]`);
  if (!el || document.activeElement === el) return;
  setTyping(true);
  el.focus({ preventScroll: true });
  const at = Math.min(ankiUI.caret || el.value.length, el.value.length);
  try {
    el.setSelectionRange(at, at);
  } catch {
    /* not a text box */
  }
}

// ───────────── moving the panel ─────────────

// Drag it by the top bar (anywhere that is not a button). The app keeps it where it is put and pins it.
// Moves are measured in screen pixels from where the drag began, so the window moving under the
// pointer does not matter, and are sent at most once per frame.
let drag = null;
card.addEventListener('pointerdown', (e) => {
  if (e.button !== 0 || !e.target.closest('.bar') || e.target.closest('button')) return;
  drag = { x: e.screenX, y: e.screenY, id: e.pointerId, queued: false, dx: 0, dy: 0 };
  document.documentElement.setPointerCapture(e.pointerId); // the bar is rebuilt as results stream in; the page is not
  pinned = true;
  invoke('panel_drag', { phase: 'start', dx: 0, dy: 0 }).catch(() => {});
  document.body.classList.add('dragging');
  e.preventDefault();
});
document.documentElement.addEventListener('pointermove', (e) => {
  if (!drag) return;
  const dpr = window.devicePixelRatio || 1;
  drag.dx = (e.screenX - drag.x) * dpr;
  drag.dy = (e.screenY - drag.y) * dpr;
  if (drag.queued) return;
  drag.queued = true;
  requestAnimationFrame(() => {
    if (!drag) return;
    drag.queued = false;
    invoke('panel_drag', { phase: 'move', dx: drag.dx, dy: drag.dy }).catch(() => {});
  });
});
const endDrag = () => {
  if (!drag) return;
  invoke('panel_drag', { phase: 'move', dx: drag.dx, dy: drag.dy }).catch(() => {}).finally(() => invoke('panel_drag', { phase: 'end', dx: 0, dy: 0 }).catch(() => {}));
  drag = null;
  document.body.classList.remove('dragging');
  render(); // shows the pin
};
document.documentElement.addEventListener('pointerup', endDrag);
document.documentElement.addEventListener('pointercancel', endDrag);

card.addEventListener('contextmenu', (event) => {
  const requestId = snapshotCorrectionId(state);
  if (requestId == null || event.target.closest('input, textarea, [contenteditable]')) return;
  event.preventDefault();
  snapshotMenu.open({ x: event.clientX, y: event.clientY }, [
    { label: 'Correct Korean OCR…', action: () => invoke('correction_open_snapshot', { requestId }).catch(e => flash(String(e))) },
    { label: 'Cancel' },
  ]);
});

// ───────────── sizing ─────────────

let lastSize = '';
function reportSize() {
  requestAnimationFrame(() => {
    const r = card.getBoundingClientRect();
    if (!r.width) return;
    const dpr = window.devicePixelRatio || 1;
    // +12 for the body padding that holds the shadow
    const w = Math.ceil((r.width + 12) * dpr), hh = Math.ceil((r.height + 12) * dpr);
    const key = `${w}x${hh}`;
    if (key !== lastSize) {
      lastSize = key;
      invoke('resize_panel', { width: w, height: hh }).catch(() => {});
    }
  });
}

// ───────────── start-up ─────────────

function applyConfig(c) {
  cfg = { ...cfg, ...c };
  document.documentElement.style.setProperty('--size', `${cfg.font_size_px}px`);
  if (cfg.theme === 'light' || cfg.theme === 'dark') document.documentElement.dataset.theme = cfg.theme;
  card.style.maxWidth = `${cfg.max_width_px ?? 760}px`;
}

// A click on a scan label that is a single word asks for its deep dive as soon as the breakdown is in.
let autoDeepId = null;
listen('maru-auto-deep', (id) => {
  autoDeepId = id;
});
// The hotkey that reads the selection aloud asks for it before the text has arrived.
let autoSpeakId = null;
listen('maru-auto-speak', (id) => {
  autoSpeakId = id;
});
listen('maru-hidden', () => { snapshotMenu.close(); reader.stop(); });
listen('maru-tts-control', (what) => {
  if (what === 'stop') reader.stop();
});
listen('maru-event', (e) => {
  const next = reduce(state, e);
  if (next === state) return;
  // The app resets the window to a default size for every new request; make sure the real size is reported again.
  if (e.type === 'started') lastSize = '';
  update(next);
  if (autoSpeakId != null && state.id === autoSpeakId) {
    const ready = state.direction === 'en_to_ko' ? e.type === 'translation_done' && e.variant === 'primary' : e.type === 'started';
    if (ready) {
      autoSpeakId = null;
      readAloud();
    }
  }
  if (autoDeepId == null) return;
  const target = autoDeepTarget(state, autoDeepId);
  if (target) {
    autoDeepId = null;
    startDeep(target, 'source');
  } else if (e.type === 'done' && e.id === autoDeepId) {
    autoDeepId = null;
  }
});
listen('maru-config', () => {
  invoke('ui_config').then((c) => {
    applyConfig(c);
    lastSize = '';
    render();
  }).catch(() => {});
});
listen('maru-notice', (n) => {
  lastSize = '';
  state = { ...emptyState(), notice: n };
  render();
});

/** A short silent WAV, so the reader can be tried in a plain browser. */
function silentClip(seconds = 0.4) {
  const rate = 8000, n = Math.floor(rate * seconds);
  const buf = new Uint8Array(44 + n * 2), v = new DataView(buf.buffer);
  const put = (o, str) => [...str].forEach((c, i) => (buf[o + i] = c.charCodeAt(0)));
  put(0, 'RIFF'); v.setUint32(4, 36 + n * 2, true); put(8, 'WAVEfmt '); v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
  v.setUint32(24, rate, true); v.setUint32(28, rate * 2, true); v.setUint16(32, 2, true); v.setUint16(34, 16, true); put(36, 'data'); v.setUint32(40, n * 2, true);
  return { mime: 'audio/wav', data: btoa(String.fromCharCode(...buf)), seconds, cached: false, took_ms: 5 };
}

if (!inTauri) {
  mockCommands({
    ui_config: () => ({ ...cfg, tts: true, anki: true }),
    deep_dive: () => 100,
    tts_plan: ({ text }) => text.split(/(?<=[.!?])\s+/).filter(Boolean),
    tts_clip: () => silentClip(),
    tts_reading: () => {},
    panel_drag: () => {},
    panel_typing: () => {},
    correction_open_snapshot: () => {location.href='correction.html?demo=snapshot';},
    anki_overview: ({ key }) => ({ connected: true, error: null, decks: ['Default', 'Korean', 'Korean::Games', 'Korean::Games::Elden Ring', 'Korean::YouTube'], suggestions: [{ deck: 'Korean::Games::Elden Ring', reason: 'what you chose last time in Elden Ring', exists: true }, { deck: 'Korean::Hades', reason: 'named after Hades', exists: false }, { deck: 'Korean', reason: 'your default deck', exists: true }], tags: ['maru', 'topik-1', 'pos::verb', 'app::elden-ring', 'added::2026-10'], existing: null, reverse: 'subdeck', app_name: 'Elden Ring', audio: true, key }),
    anki_add: ({ request }) => ({ result: 'added', deck: request.deck, reverse_deck: request.reverse === 'off' ? null : `${request.deck}::Reverse`, cards: request.reverse === 'off' ? 1 : 2, sync: true, note: null }),
    anki_add_example: () => true,
    resize_panel: () => {},
    dismiss_panel: () => {},
    set_pinned: () => {},
    save_word: ({ key }) => ({ key, state: 'learning', lookups: 2, saved: true }),
    set_known: ({ key, known }) => ({ key, state: known ? 'known' : 'learning', lookups: 1, saved: false }),
  });
}

invoke('ui_config').then(applyConfig).catch(() => {}).finally(() => {
  render();
  replayFromQuery();
});
