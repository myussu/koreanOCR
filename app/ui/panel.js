import { invoke, listen, replayFromQuery, mockCommands, inTauri } from './bridge.js';
import { emptyState, reduce, openDeep, closeDeep, meaningFor, translationRows, sentenceOf, VARIANT_LABEL } from './core.js';

const card = document.getElementById('card');
let state = emptyState();
let cfg = { gloss_mode: 'fade_known', font_size_px: 18, show_romanization: true, show_pronunciation: true, show_grammar_formula: true, theme: 'auto', anki: false, opacity: 0.97 };
let pinned = false;
let deepSide = 'source';

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

function wordColumn(e, side) {
  if (!e.content) return h('span', { class: 'punct', text: e.surface });
  const gloss = state.glosses[side][e.index];
  const m = meaningFor(e, gloss, cfg.gloss_mode);
  const open = state.deep && state.deep.eojeol.index === e.index && deepSide === side;
  return h(
    'div',
    {
      class: `w ${e.state ?? 'new'}${open ? ' open' : ''}`,
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
  return h('div', { class: 'words' }, data.eojeols.map((e) => wordColumn(e, side)));
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
    h('h3', {}, key, h('span', { class: 'chip', text: e.pos }), entry?.hanja && h('span', { class: 'hanja', text: entry.hanja }), (entry?.level ?? e.level) && h('span', { class: 'chip level', text: entry?.level ?? e.level }), h('button', { class: 'icon', style: 'margin-left:auto', title: 'close', onclick: () => update(closeDeep(state)), text: '✕' })),
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
        h('button', { class: 'icon', 'aria-pressed': String(pinned), title: 'keep this open', onclick: togglePin, text: '📌' }),
        h('button', { class: 'icon', title: 'close', onclick: () => invoke('dismiss_panel'), text: '✕' }),
      ),
    );
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

listen('maru-event', (e) => update(reduce(state, e)));
listen('maru-notice', (n) => {
  state = { ...emptyState(), notice: n };
  render();
});

if (!inTauri) {
  mockCommands({
    ui_config: () => cfg,
    deep_dive: () => 100,
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
