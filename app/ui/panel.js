import { invoke, listen, replayFromQuery, mockCommands, inTauri } from './bridge.js';
import { emptyState, reduce, openDeep, closeDeep, meaningFor, translationRows, sentenceOf, autoDeepTarget, readableText, sentenceRanges, VARIANT_LABEL } from './core.js';
import { Reader } from './reader.js';

const card = document.getElementById('card');
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
    h('h3', {}, key, h('span', { class: 'chip', text: e.pos }), entry?.hanja && h('span', { class: 'hanja', text: entry.hanja }), (entry?.level ?? e.level) && h('span', { class: 'chip level', text: entry?.level ?? e.level }), h('span', { class: 'speak-word' }, cfg.tts && h('button', { class: 'icon', title: 'hear this word', onclick: () => readAloud(spoken(e.surface)), text: '🔊' }), cfg.tts && spoken(e.surface) !== key && h('button', { class: 'icon', title: `hear the dictionary form, ${key}`, onclick: () => readAloud(key), text: '🔊 ' + key })), h('button', { class: 'icon', style: 'margin-left:auto', title: 'close', onclick: () => update(closeDeep(state)), text: '✕' })),
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
      h('button', { class: `btn${saved ? '' : ' primary'}`, onclick: () => act('save_word', { key }), disabled: saved, text: saved ? '✓ in your review queue' : 'Save to review' }),
      h('button', { class: 'btn', onclick: () => act('set_known', { key, known: !known }), text: known ? 'Not known after all' : 'I know this' }),
      cfg.anki && h('button', { class: 'btn', onclick: () => anki(key), text: 'Add to Anki' }),
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
        { class: 'bar' },
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
    for (const m of s.errors) kids.push(h('div', { class: 'err', text: m }));
    const t = s.timings;
    if (s.finished && Object.keys(t).length)
      kids.push(h('div', { class: 'foot' }, [['analysis', t.morphology], ['first word', t.first_token], ['total', t.total]].filter(([, v]) => v != null).map(([k, v]) => h('span', { text: `${k} ${v} ms` }))));
  }
  card.replaceChildren(...kids.filter(Boolean));
  card.style.maxWidth = '';
  reportSize();
}

// ───────────── actions ─────────────

function update(next) {
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
}

async function act(cmd, args) {
  try {
    const status = await invoke(cmd, args);
    if (status && state.deep) update(reduce(state, { type: 'word_state', id: state.deep.id, status }));
  } catch (e) {
    flash(String(e));
  }
}

async function anki(key) {
  try {
    const r = await invoke('export_anki', { key });
    flash(r == null ? 'Already in Anki' : 'Added to Anki');
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
listen('maru-hidden', () => reader.stop());
listen('maru-tts-control', (what) => {
  if (what === 'stop') reader.stop();
});
listen('maru-event', (e) => {
  // The app resets the window to a default size for every new request; make sure the real size is reported again.
  if (e.type === 'started') lastSize = '';
  update(reduce(state, e));
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
    ui_config: () => ({ ...cfg, tts: true }),
    deep_dive: () => 100,
    tts_plan: ({ text }) => text.split(/(?<=[.!?])\s+/).filter(Boolean),
    tts_clip: () => silentClip(),
    tts_reading: () => {},
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
