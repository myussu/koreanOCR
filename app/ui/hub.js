import { invoke, listen, mockCommands, inTauri } from './bridge.js';
import { latencyText, liveHistory } from './live-core.js';
import { settingsView } from './settings.js';

const view = document.getElementById('view');
const tabs = [...document.querySelectorAll('#tabs button')];

function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'text') el.textContent = v;
    else el.setAttribute(k, v);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null && kid !== false) el.append(kid);
  return el;
}

let tab = location.hash.slice(1) || 'review';

async function show(name) {
  tab = name;
  for (const b of tabs) b.setAttribute('aria-selected', String(b.dataset.tab === name));
  view.replaceChildren(h('p', { class: 'center', text: 'Loading…' }));
  try {
    view.replaceChildren(...(await { review, history, status, settings: settingsView }[name]()));
  } catch (e) {
    view.replaceChildren(h('div', { class: 'err', text: String(e) }));
  }
}

// ───────────── review ─────────────
async function review() {
  const s = await invoke('status');
  if (s.anki) return ankiReview(s);
  const due = await invoke('due_words', { limit: 50 });
  if (!due.length) return [h('p', { class: 'center', text: 'Nothing is due right now. Words you look up twice join this queue automatically.' })];
  const [word, ...rest] = due;
  const contexts = await invoke('word_contexts', { key: word.key });
  let revealed = false;

  const root = h('div');
  const draw = () => {
    root.replaceChildren(
      h('div', { class: 'center', text: `${due.length} due` }),
      h('div', { class: 'big', text: word.key }),
      word.level && h('div', { class: 'center', text: word.level }),
      revealed
        ? [
            h('div', { class: 'big', style: 'font-size:22px;font-family:var(--ui)', text: word.meaning ?? '(no meaning saved)' }),
            contexts.map((c) => h('div', { class: 'ctx' }, c.sentence, c.translation && h('small', { text: c.translation }))),
            h('div', { class: 'rate' }, [['Again', 1, ''], ['Hard', 2, ''], ['Good', 3, 'primary'], ['Easy', 4, '']].map(([label, n, cls]) => h('button', { class: `btn ${cls}`, onclick: () => rate(n), text: `${label}  (${n})` }))),
          ]
        : [contexts.slice(0, 1).map((c) => h('div', { class: 'ctx' }, c.sentence)), h('div', { class: 'rate' }, h('button', { class: 'btn primary', onclick: () => ((revealed = true), draw()), text: 'Show answer' }))],
    );
  };
  const rate = async (n) => {
    await invoke('review_word', { key: word.key, rating: n });
    show('review');
  };
  draw();
  addEventListener('keydown', function onKey(e) {
    if (tab !== 'review') return removeEventListener('keydown', onKey);
    if (!revealed && (e.key === ' ' || e.key === 'Enter')) ((revealed = true), draw());
    else if (revealed && '1234'.includes(e.key)) rate(Number(e.key));
  });
  return [root, rest.length > 0 && h('p', { class: 'center', text: `Next: ${rest.slice(0, 5).map((w) => w.key).join(' · ')}${rest.length > 5 ? ' …' : ''}` })];
}

// Reviews happen in Anki: this tab says what is waiting there.
async function ankiReview(s) {
  const out = [h('h4', { text: 'Review in Anki' })];
  try {
    const d = await invoke('anki_due');
    out.push(
      h('p', {}, [['due now', d.due], ['new', d.new]].map(([l, v]) => h('span', { class: 'stat' }, h('b', { text: v }), l))),
      h('p', { class: 'center', text: d.due + d.new ? 'Open Anki and study the Maru Korean cards; Maru reads your progress back so words you hold for weeks fade in the translations.' : 'Nothing is waiting in Anki. Words you add from a deep dive land there.' }),
    );
  } catch (e) {
    out.push(h('p', { class: 'center', text: `Anki is not reachable: ${String(e).slice(0, 160)}` }));
  }
  if (s.vocab) out.push(h('p', {}, [['words seen', s.vocab.total], ['in Anki', s.vocab.saved], ['known', s.vocab.known]].map(([l, v]) => h('span', { class: 'stat' }, h('b', { text: v }), l))));
  return out;
}

// ───────────── history ─────────────
let historyMode = 'all'; // 'all' (newest first) or 'live' (subtitles, in the order they were said)

async function history() {
  const all = await invoke('history');
  const live = liveHistory(all);
  const items = historyMode === 'live' ? live : [...all].reverse();
  const chip = (mode, label) => h('button', { class: 'chip', 'aria-pressed': String(historyMode === mode), text: label, onclick: () => { historyMode = mode; show('history'); } });
  const bar = h('div', { class: 'chips' }, chip('all', 'Everything'), chip('live', `Live subtitles (${live.length})`));
  if (!items.length) {
    const msg = historyMode === 'live' ? 'Live subtitles you have watched will appear here, in order, so you can read them after a cutscene.' : 'Lines you translate will appear here — handy for subtitles that disappear before you finish reading.';
    return [bar, h('p', { class: 'center', text: msg })];
  }
  const row = (i) =>
    h(
      'div',
      { class: i.live ? 'hist live' : 'hist', title: 'Click for the word-by-word breakdown', onclick: () => invoke('history_open', { text: i.direction === 'en_to_ko' ? i.translation : i.source }).catch(() => {}) },
      h('div', { class: i.direction === 'en_to_ko' ? 'en' : 'ko', text: i.source }),
      h('div', { class: i.direction === 'en_to_ko' ? 'ko' : 'en', text: i.translation }),
      h('div', { class: 'status', text: [i.live ? 'live' : '', i.app, new Date((i.appeared_at ?? i.at) * 1000).toLocaleTimeString(), latencyText(i)].filter(Boolean).join(' · ') }),
    );
  return [bar, ...items.map(row)];
}

// ───────────── status ─────────────
async function status() {
  const s = await invoke('status');
  const k = s.hotkeys;
  const row = (a, b) => h('tr', {}, h('td', { text: a }), h('td', {}, b));
  const on = (yes, detail) => h('span', { class: yes ? 'flag on' : 'flag off' }, yes ? `ON${detail ? ` · ${detail}` : ''}` : 'off');
  const area = s.live_area && `${Math.round(s.live_area.w)}×${Math.round(s.live_area.h)} at (${Math.round(s.live_area.x)}, ${Math.round(s.live_area.y)})`;
  return [
    h('h4', { text: 'Right now' }),
    h('table', {}, row('Hover mode', [on(s.hover_mode), ' ', h('kbd', { text: k.hover })]), row('Live subtitles', [on(s.live_mode, area), ' ', h('kbd', { text: k.continuous })]), row('Screen labels', [on(s.scan_visible), ' ', h('kbd', { text: k.scan })]), row('Lite (game) profile', s.lite ? on(true, 'lighter models') : 'off'), row('Speech', s.tts === 'ready' ? on(true, s.tts_source === 'local' ? 'voice on this PC' : 'voice from a server') : s.tts === 'off' ? 'off (Settings → Speech)' : s.tts), row('OCR reading model', s.ocr_reader === 'ready' ? on(true, 'PP-OCR + PaddleOCR-VL') : s.ocr_reader === 'off' ? 'not in use (PP-OCR only)' : s.ocr_reader)),
    s.vocab && h('p', {}, [['words', s.vocab.total], ['saved', s.vocab.saved], ['known', s.vocab.known], ['due', s.vocab.due_now]].map(([l, v]) => h('span', { class: 'stat' }, h('b', { text: v }), l))),
    h('h4', { text: 'Components' }),
    h('table', {}, row('Reviews', s.anki ? 'in Anki' : 'in Maru'), row('Language model', s.llm), row('Korean analyser', s.analyser), row('Dictionary', s.dictionary ? 'installed' : 'not installed (maru dict download)'), row('OCR', s.ocr ? 'ready' : 'not available (maru setup ocr)')),
    h('h4', { text: 'Shortcuts' }),
    h('table', {}, row('Translate selection', h('kbd', { text: k.translate })), row('Pick a region', h('kbd', { text: k.region })), row('Hover mode', h('kbd', { text: k.hover })), row('Continuous mode', h('kbd', { text: k.continuous })), row('Continuous: choose area', h('kbd', { text: k.continuous_area })), row('Label all Korean text on screen', h('kbd', { text: k.scan })), row('Translate and read aloud', h('kbd', { text: k.speak })), row('History', h('kbd', { text: k.history })), row('Dismiss panel', h('kbd', { text: k.dismiss }))),
    h('h4', { text: 'Notes' }),
    s.notes.map((n) => h('p', { class: 'status', text: n })),
  ];
}

// Toggles can change from a hotkey while this window is open: keep the status tab current.
setInterval(async () => {
  if (tab !== 'status' || document.hidden) return;
  try {
    view.replaceChildren(...(await status()));
  } catch {
    /* keep what is showing */
  }
}, 1500);

for (const b of tabs) b.addEventListener('click', () => show(b.dataset.tab));
listen('maru-tab', (name) => show(name));

if (!inTauri) {

  // A stand-in settings store for browser screenshots and tests.
  const flat = { 'hotkeys.translate': 'Ctrl+Alt+T | F6', 'hotkeys.region': 'Ctrl+Alt+R', 'hotkeys.hover': 'Ctrl+Alt+H', 'hotkeys.continuous': 'Ctrl+Alt+C', 'hotkeys.continuous_area': 'Ctrl+Alt+Shift+C', 'hotkeys.scan': 'Ctrl+Alt+S', 'hotkeys.speak': 'Ctrl+Alt+P', 'hotkeys.history': 'Ctrl+Alt+L', 'hotkeys.dismiss': 'Ctrl+Alt+Escape', 'ui.font_size_px': 18, 'ui.max_width_px': 760, 'ui.theme': 'auto', 'ui.placement': 'auto', 'ui.show_romanization': true, 'ui.show_pronunciation': true, 'ui.show_grammar_formula': true, 'learning.gloss_mode': 'fade_known', 'learning.register': 'polite', 'learning.track_lookups': true, 'learning.auto_save_after_lookups': 2, 'learning.known_stability_days': 21, 'learning.desired_retention': 0.9, 'learning.anki.enabled': false, 'learning.anki.deck': 'Maru', 'learning.anki.note_type': 'Basic', 'learning.anki.url': 'http://127.0.0.1:8765', 'ocr.vision_reader': true, 'ocr.backend': 'auto', 'ocr.device': 'cpu', 'ocr.min_confidence': 0.8, 'capture.use_accessibility': true, 'capture.use_clipboard_fallback': true, 'capture.use_ocr_fallback': true, 'capture.ocr_radius_px': 320, 'capture.max_chars': 500, 'continuous.interval_ms': 100, 'continuous.stable_ms': 150, 'continuous.linger_ms': 1500, 'continuous.context_lines': 4, 'continuous.diff_threshold': 0.01, 'continuous.history_len': 300, lite: false, 'llm.hf_repo': null, 'llm.model_path': null, 'llm.endpoint': null, 'llm.parallel': 2, 'llm.ctx_size': 4096, 'llm.gpu_layers': 'all', 'llm.flash_attn': true, 'llm.temperature': 0.2, 'llm.context_lines': 4, 'lite_profile.hf_repo': 'unsloth/gemma-4-E4B-it-qat-GGUF:UD-Q4_K_XL', 'lite_profile.ctx_size': 3072, 'lite_profile.parallel': 1, 'tts.enabled': false, 'tts.run_in_lite': false, 'tts.provider': 'local', 'tts.endpoint': null, 'tts.api_key': null, 'tts.model': 'qwen3-tts', 'tts.voice': 'sohee', 'tts.language': '', 'tts.instructions': null, 'tts.speed': 1.0, 'tts.cache_mb': 300, 'tts.device': 'auto', 'tts.port': 18095, 'learning.anki.enabled': false, 'learning.anki.deck': 'Korean', 'learning.anki.reverse': 'subdeck', 'learning.anki.audio': true, 'learning.anki.sync_after_add': true, 'learning.anki.auto_tags': true, 'learning.anki.tags': ['maru'], 'learning.anki.refresh_minutes': 30, 'learning.anki.url': 'http://127.0.0.1:8765', 'learning.anki.api_key': null, 'learning.anki.note_type': 'Maru Korean', 'learning.anki.reverse_subdeck': 'Reverse' };
  const nest = (f) => Object.entries(f).reduce((o, [k, v]) => (k.split('.').reduce((t, p, i, a) => (t[p] = i === a.length - 1 ? v : (t[p] ?? {})), o), o), {});
  Object.assign(flat, { 'continuous.cpu_ocr': false, 'continuous.max_in_flight': 0, 'continuous.filter_ocr_jitter': false });
  const mockDefaults = nest(flat);
  let mockValues = nest({ ...flat, 'llm.hf_repo': 'unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL' });
  let mockPending = [];
  mockCommands({
    get_settings: () => ({ values: mockValues, defaults: mockDefaults, path: 'C:\\Users\\you\\AppData\\Roaming\\maru\\Maru\\config\\config.toml', pending_restart: mockPending, lite_from_env: false }),
    save_settings: ({ changes }) => {
      const next = JSON.parse(JSON.stringify(mockValues));
      for (const [path, v] of Object.entries(changes)) path.split('.').reduce((t, p, i, a) => (i === a.length - 1 ? (t[p] = v === null ? path.split('.').reduce((d, k) => d?.[k], mockDefaults) : v) : t[p]), next);
      mockValues = next;
      const restart = Object.keys(changes).filter((p) => !p.startsWith('hotkeys.') && !p.startsWith('ui.'));
      mockPending = [...new Set([...mockPending, ...restart])];
      return { changed: Object.keys(changes), restart: mockPending, warnings: [] };
    },
    restart_app: () => {},
    anki_check: () => 'Anki is reachable (AnkiConnect v6); the note type "Maru Korean" is ready; 7 decks.',
    anki_sync: () => null,
    anki_due: () => ({ due: 12, new: 5 }),
    tts_test: () => Promise.reject('The local voice is not running yet. Save and restart Maru to start it.'),
    open_config_file: () => {},
  });
  mockCommands({
    due_words: () => [{ key: '공부하다', meaning: 'to study', level: 'TOPIK 1', state: 'learning', lookups: 2, saved: true }, { key: '학교', meaning: 'school', state: 'learning', lookups: 2, saved: true }],
    word_contexts: () => [{ sentence: '저는 학교에서 공부했어요', translation: 'I studied at school.' }],
    review_word: () => ({}),
    history: () => [{ id: 1, at: 1760000000, source: '저는 학교에서 공부했어요', translation: 'I studied at school.', direction: 'ko_to_en', app: 'chrome.exe', live: false, first_ms: null, total_ms: null }, { id: 2, at: 1760000030, source: '다시는 못 볼 줄 알았어요', translation: 'I thought I would never see you again.', direction: 'ko_to_en', app: 'game.exe', live: true, first_ms: 820, total_ms: 1410 }, { id: 3, at: 1760000034, source: '저도요', translation: 'Me too.', direction: 'ko_to_en', app: 'game.exe', live: true, first_ms: 610, total_ms: 790 }],
    history_open: () => {},
    status: () => ({ notes: ['Korean analyser: Kiwi', 'OCR: PP-OCR on cpu'], llm: 'ready', analyser: 'kiwi', ocr: true, dictionary: true, hotkeys: { translate: 'Ctrl+Alt+T | F6', region: 'Ctrl+Alt+R', hover: 'Ctrl+Alt+H | F8', continuous: 'Ctrl+Alt+C', continuous_area: 'Ctrl+Alt+Shift+C', scan: 'Ctrl+Alt+S', speak: 'Ctrl+Alt+P', history: 'Ctrl+Alt+L', dismiss: 'Ctrl+Alt+Escape' }, vocab: { total: 42, saved: 12, known: 20, due_now: 2 }, anki: false, hover_mode: true, live_mode: true, scan_visible: false, lite: false, live_area: { x: 320, y: 880, w: 1280, h: 120 }, ocr_reader: 'ready', tts: 'off', tts_source: '' }),
  });
}
show(tab);
