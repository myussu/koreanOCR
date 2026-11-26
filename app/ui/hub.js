import { invoke, listen, mockCommands, inTauri } from './bridge.js';
import { ankiWordCounts } from './anki.js';
import { canStar, historyRows, latencyText, liveHistory } from './live-core.js';
import { settingsView } from './settings.js';
import { resourcesView } from './resources.js';
import { readingView } from './reading.js';
import { mineSheet } from './mine.js';
import { captionsLine } from './captions-core.js';

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
let showRevision = 0;

async function show(name) {
  if(view.firstElementChild?.canLeave && !view.firstElementChild.canLeave())return;
  const revision=++showRevision;
  view.firstElementChild?.dispose?.();
  tab = name;
  for (const b of tabs) b.setAttribute('aria-selected', String(b.dataset.tab === name));
  view.replaceChildren(h('p', { class: 'center', text: 'Loading…' }));
  try {
    const children=await { review, history, status, settings: settingsView, resources: resourcesView, reading: readingView }[name]();
    if(revision!==showRevision){for(const child of children)child?.dispose?.();return;}
    view.replaceChildren(...children);
  } catch (e) {
    if(revision!==showRevision)return;
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
  if (s.vocab) out.push(h('p', {}, ankiWordCounts(s).map(([l, v]) => h('span', { class: 'stat' }, h('b', { text: v }), l))));
  return out;
}

// ───────────── history ─────────────
let historyMode = 'all'; // 'all' (newest first), 'live' (subtitles, in the order they were said) or 'starred'

async function history() {
  const [all, starred] = await Promise.all([invoke('history'), invoke('history_starred').catch(() => [])]);
  const live = liveHistory(all);
  const items = historyRows(historyMode, all, starred);
  const chip = (mode, label) => h('button', { class: 'chip', 'aria-pressed': String(historyMode === mode), text: label, onclick: () => { historyMode = mode; show('history'); } });
  const bar = h('div', { class: 'chips' }, chip('all', 'Everything'), chip('live', `Live subtitles (${live.length})`), chip('starred', `★ Starred (${starred.length})`));
  if (!items.length) {
    const msg = historyMode === 'starred'
      ? 'Lines you star while playing appear here to study later. Star the live subtitle on screen with its shortcut (Settings → Shortcuts) or the wheel\'s "Star this line", or click ☆ next to a live line below.'
      : historyMode === 'live' ? 'Live subtitles you have watched will appear here, in order, so you can read them after a cutscene.' : 'Lines you translate will appear here — handy for subtitles that disappear before you finish reading.';
    return [bar, h('p', { class: 'center', text: msg })];
  }
  const star = (i) =>
    canStar(i) &&
    h('button', {
      class: 'star', 'aria-pressed': String(!!i.starred), title: i.starred ? 'Take the star off' : 'Star this line to study it later', text: i.starred ? '★' : '☆',
      onclick: (e) => { e.stopPropagation(); invoke('history_star', { savedId: i.saved_id, starred: !i.starred }).then(() => show('history')).catch((err) => alert(String(err))); },
    });
  // "Make cards" opens the mining sheet (mine.js) under its line; one at a time.
  const mine = (i) =>
    i.live &&
    h('button', {
      class: i.mined ? 'mine-open done' : 'mine-open', title: 'Make Anki cards from this line: its words and the line itself', text: i.mined ? '✓ Cards made' : 'Make cards',
      onclick: (e) => {
        e.stopPropagation();
        const button = e.currentTarget;
        const open = view.querySelector('.mine');
        open?.remove();
        if (open?.dataset.for === button.dataset.for) return;
        const sheet = mineSheet(h, i, { onClose: () => sheet.remove(), onAdded: () => { i.mined = true; button.className = 'mine-open done'; button.textContent = '✓ Cards made'; } });
        sheet.dataset.for = button.dataset.for;
        button.closest('.hist').after(sheet);
      },
    });
  const row = (i, n) => {
    const r = h(
      'div',
      { class: i.live ? 'hist live' : 'hist', title: 'Click for the word-by-word breakdown', onclick: () => invoke('history_open', { text: i.direction === 'en_to_ko' ? i.translation : i.source }).catch(() => {}) },
      star(i),
      h('div', { class: i.direction === 'en_to_ko' ? 'en' : 'ko', text: i.source }),
      h('div', { class: i.direction === 'en_to_ko' ? 'ko' : 'en', text: i.translation }),
      h('div', { class: 'status', text: [i.live ? 'live' : '', i.app, new Date((i.appeared_at ?? i.at) * 1000).toLocaleTimeString(), latencyText(i)].filter(Boolean).join(' · ') }, mine(i)),
    );
    r.querySelector('.mine-open')?.setAttribute('data-for', String(n));
    return r;
  };
  const rows = items.map(row);
  // hub.html?mine#history opens the cards sheet of the first live line (how the sheet is photographed with demo data).
  if (new URLSearchParams(location.search).has('mine')) setTimeout(() => rows.find((r) => r.querySelector('.mine-open'))?.querySelector('.mine-open').click(), 0);
  return [bar, ...rows];
}

// ───────────── status ─────────────
async function status() {
  const s = await invoke('status');
  const k = s.hotkeys;
  const row = (a, b) => h('tr', {}, h('td', { text: a }), h('td', {}, b));
  const on = (yes, detail) => h('span', { class: yes ? 'flag on' : 'flag off' }, yes ? `ON${detail ? ` · ${detail}` : ''}` : 'off');
  const area = s.live_area && `${Math.round(s.live_area.w)}×${Math.round(s.live_area.h)} at (${Math.round(s.live_area.x)}, ${Math.round(s.live_area.y)})`;
  // Captions the browser extension sends (docs/YOUTUBE_CAPTIONS.md).
  const captions = captionsLine(await invoke('captions_status').catch(() => null));
  return [
    h('h4', { text: 'Right now' }),
    h('table', {}, row('Hover mode', [on(s.hover_mode), ' ', h('kbd', { text: k.hover })]), row('Live subtitles', [on(s.live_mode, area), ' ', h('kbd', { text: k.continuous })]), row('YouTube captions', captions.on ? on(true, captions.text) : captions.text), row('Screen labels', [on(s.scan_visible), ' ', h('kbd', { text: k.scan })]), row('Lite (game) profile', s.lite ? on(true, 'lighter models') : 'off'), row('Speech', s.tts === 'ready' ? on(true, s.tts_source === 'local' ? 'voice on this PC' : 'voice from a server') : s.tts === 'off' ? 'off (Settings → Speech)' : s.tts), row('OCR reading model', s.ocr_reader === 'ready' ? on(true, 'PP-OCR + PaddleOCR-VL') : s.ocr_reader === 'off' ? 'not in use (PP-OCR only)' : s.ocr_reader)),
    s.vocab && h('p', {}, [['words', s.vocab.total], ['saved', s.vocab.saved], ['known', s.vocab.known], ['due', s.vocab.due_now]].map(([l, v]) => h('span', { class: 'stat' }, h('b', { text: v }), l))),
    h('h4', { text: 'Components' }),
    h('table', {}, row('Reviews', s.anki ? 'in Anki' : 'in Maru'), row('Language model', s.llm), row('Korean analyser', s.analyser), row('Dictionary', s.dictionary ? 'installed' : 'not installed (maru dict download)'), row('OCR', s.ocr ? 'ready' : 'not available (maru setup ocr)')),
    h('h4', { text: 'Shortcuts' }),
    h('table', {}, row('Translate selection', h('kbd', { text: k.translate })), row('Pick a region', h('kbd', { text: k.region })), row('Hover mode', h('kbd', { text: k.hover })), row('Continuous mode', h('kbd', { text: k.continuous })), row('Continuous: choose area', h('kbd', { text: k.continuous_area })), row('Label all Korean text on screen', h('kbd', { text: k.scan })), row('Translate and read aloud', h('kbd', { text: k.speak })), row('History', h('kbd', { text: k.history })), row('Star the live subtitle line', h('kbd', { text: k.star || '—' })), row('Dismiss panel', h('kbd', { text: k.dismiss }))),
    h('h4', { text: 'Notes' }),
    ...s.notes.map((n) => h('p', { class: 'status', text: n })),
  ];
}

// Toggles can change from a hotkey while this window is open: keep the status tab current.
setInterval(async () => {
  if (tab !== 'status' || document.hidden) return;
  const revision=showRevision;
  try {
    const children=await status();
    if(revision!==showRevision||tab!=='status'){for(const child of children)child?.dispose?.();return;}
    view.replaceChildren(...children);
  } catch {
    /* keep what is showing */
  }
}, 1500);

for (const b of tabs) b.addEventListener('click', () => show(b.dataset.tab));
listen('maru-tab', (name) => show(name));

if (!inTauri) {

  // A stand-in settings store for browser screenshots and tests.
  const flat = { 'hotkeys.translate': 'Ctrl+Alt+T | F6', 'hotkeys.region': 'Ctrl+Alt+R', 'hotkeys.hover': 'Ctrl+Alt+H', 'hotkeys.continuous': 'Ctrl+Alt+C', 'hotkeys.continuous_area': 'Ctrl+Alt+Shift+C', 'hotkeys.scan': 'Ctrl+Alt+S', 'hotkeys.speak': 'Ctrl+Alt+P', 'hotkeys.history': 'Ctrl+Alt+L', 'hotkeys.dismiss': 'Ctrl+Alt+Escape', 'ui.font_size_px': 18, 'ui.max_width_px': 760, 'ui.theme': 'auto', 'ui.placement': 'auto', 'ui.show_romanization': true, 'ui.show_pronunciation': true, 'ui.show_grammar_formula': true, 'learning.gloss_mode': 'fade_known', 'learning.register': 'polite', 'learning.track_lookups': true, 'learning.auto_save_after_lookups': 2, 'learning.known_stability_days': 21, 'learning.desired_retention': 0.9, 'learning.anki.enabled': false, 'learning.anki.deck': 'Maru', 'learning.anki.note_type': 'Basic', 'learning.anki.url': 'http://127.0.0.1:8765', 'ocr.vision_reader': true, 'ocr.backend': 'auto', 'ocr.device': 'cpu', 'ocr.min_confidence': 0.8, 'ocr.threads': 0, 'capture.use_accessibility': true, 'capture.use_clipboard_fallback': true, 'capture.use_ocr_fallback': true, 'capture.ocr_radius_px': 320, 'capture.max_chars': 500, 'continuous.interval_ms': 100, 'continuous.stable_ms': 150, 'continuous.linger_ms': 1500, 'continuous.context_lines': 4, 'continuous.diff_threshold': 0.01, 'continuous.history_len': 300, lite: false, 'llm.hf_repo': null, 'llm.model_path': null, 'llm.endpoint': null, 'llm.parallel': 2, 'llm.ctx_size': 4096, 'llm.gpu_layers': 'all', 'llm.flash_attn': true, 'llm.temperature': 0.2, 'llm.context_lines': 4, 'lite_profile.hf_repo': 'unsloth/gemma-4-E4B-it-qat-GGUF:UD-Q4_K_XL', 'lite_profile.ctx_size': 3072, 'lite_profile.parallel': 1, 'tts.enabled': false, 'tts.run_in_lite': false, 'tts.provider': 'local', 'tts.endpoint': null, 'tts.api_key': null, 'tts.model': 'qwen3-tts', 'tts.voice': 'sohee', 'tts.language': '', 'tts.instructions': null, 'tts.speed': 1.0, 'tts.cache_mb': 300, 'tts.device': 'auto', 'tts.port': 18095, 'learning.anki.enabled': false, 'learning.anki.deck': 'Korean', 'learning.anki.reverse': 'subdeck', 'learning.anki.audio': true, 'learning.anki.sync_after_add': true, 'learning.anki.auto_tags': true, 'learning.anki.tags': ['maru'], 'learning.anki.refresh_minutes': 30, 'learning.anki.url': 'http://127.0.0.1:8765', 'learning.anki.api_key': null, 'learning.anki.note_type': 'Maru Korean', 'learning.anki.reverse_subdeck': 'Reverse' };
  const nest = (f) => Object.entries(f).reduce((o, [k, v]) => (k.split('.').reduce((t, p, i, a) => (t[p] = i === a.length - 1 ? v : (t[p] ?? {})), o), o), {});
  Object.assign(flat, { 'continuous.cpu_ocr': false, 'continuous.max_in_flight': 0, 'continuous.filter_ocr_jitter': false });
  Object.assign(flat, {'hotkeys.star':'Ctrl+Alt+B','learning.anki.sentence_note_type':'Maru Korean Sentence','hotkeys.launcher':'Ctrl+Alt+M','ui.launcher':true,'continuous.translation.model':'main','continuous.translation.hy_mt2_path':null,'continuous.translation.milmmt_path':null,'continuous.translation.hy_mt2_small_path':null,'continuous.translation.lfm_path':null,'continuous.translation.device':'gpu','continuous.translation.cpu_threads':4,'continuous.translation.dialogue_context':false,'continuous.translation.snapshot_endpoint':null,'continuous.translation.snapshot_model_name':'maru'});
  Object.assign(flat,{'llm.provider':'openai','llm.api_key':null,'llm.servers':[],'llm.model_name':'maru','llm.reasoning_effort':'none','continuous.translation.snapshot_provider':'openai','continuous.translation.snapshot_api_key':null});
  // One model per task, the two card decks and the per-day numbers; the demo's own file names four connections.
  Object.assign(flat, { 'tasks.version': 1, 'tasks.live.connection': 'local', 'tasks.live.model': '', 'tasks.capture.connection': 'local', 'tasks.capture.model': '', 'tasks.capture.while_live': '', 'tasks.capture.while_live_model': '', 'tasks.card_meanings.connection': 'local', 'tasks.card_meanings.model': '', 'tasks.card_sentence.connection': '', 'tasks.card_sentence.model': '', 'tasks.voice_reading.voice': '', 'tasks.voice_reading.endpoint': '', 'tasks.voice_reading.model': '', 'tasks.voice_cards.voice': '',
    'learning.anki.sentence_deck': '', 'learning.anki.new_words_per_day': 3, 'learning.anki.new_sentences_per_day': 1, 'learning.anki.suggest_app_decks': false, 'learning.anki.correct_forms': false, 'continuous.translation.backup_model': null, 'continuous.translation.backup_after_ms': 2500, 'llm.server_keep_minutes': 0 });
  Object.assign(flat, { 'captions.enabled': false, 'captions.port': 18098, 'captions.token': null, 'captions.allowed_origin': '' });
  const mockDefaults = nest(flat);
  const demoServers = [
    { name: 'NAS', endpoint: 'http://192.168.1.20:11434', model_name: 'gemma4-12b', provider: 'openai', api_key: null },
    { name: 'Claude', endpoint: 'https://api.anthropic.com', model_name: 'claude-sonnet-5-5', provider: 'openai', api_key: 'demo-key' },
    { name: 'Azure', endpoint: 'https://contoso.openai.azure.com', model_name: 'gpt-4.1-mini', provider: 'azure', api_key: 'demo-key' },
    { name: 'Gemini', endpoint: 'https://generativelanguage.googleapis.com/v1beta/openai', model_name: 'gemini-flash', provider: 'gemini', api_key: 'demo-key' },
  ];
  let mockValues = nest({ ...flat, 'llm.hf_repo': 'unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL', 'llm.servers': demoServers, 'llm.server_keep_minutes': 15, 'tasks.live.connection': 'NAS', 'tasks.capture.connection': 'Gemini', 'tasks.capture.while_live': 'Azure', 'continuous.translation.backup_model': 'lfm', 'continuous.translation.lfm_path': 'D:/models/lfm2-koen-Q5_K_M.gguf',
    'learning.anki.enabled': true, 'learning.anki.deck': 'Korean::Words', 'learning.anki.sentence_deck': 'Korean::Sentences', 'learning.anki.reverse': 'off', 'tts.enabled': true, 'captions.enabled': true, 'captions.token': 'demo0123456789abcdef0123456789ab' });
  // What each task uses in the demo, and what a test of it finds: one working, one out of quota, one not loaded.
  const demoPlace = (name) => (name === 'local' ? 'this_pc' : name === 'NAS' ? 'network' : 'hosted');
  const demoKnown = { NAS: ['working', 'NAS answered.'], Gemini: ['quota', 'Gemini has no quota left.'] };
  const demoTask = (task, tested) => {
    const route = mockValues.tasks[task];
    if (!route.connection) return { task, label: task, connection: '', model: '', place: null, state: 'off', message: 'Cards keep the translation the line already has.', fallbacks: [], checked_secs_ago: null };
    const local = route.connection === 'local';
    const server = demoServers.find((s) => s.name === route.connection);
    const [state, message] = demoKnown[route.connection] ?? (tested ? ['working', `${local ? "This PC's model" : route.connection} answered in 0.4 s.`] : local ? ['not_loaded', "This PC's model loads when it is first used."] : ['unknown', 'Not asked yet since Maru started. Press Test to check it.']);
    const fallbacks = task.startsWith('card_') ? [...(local ? [] : ["This PC's model"]), ...(task === 'card_meanings' ? ['the dictionary'] : [])] : [];
    return { task, label: task, connection: route.connection, model: local ? 'gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL' : server?.model_name ?? '', place: demoPlace(route.connection), state, message, fallbacks, checked_secs_ago: tested ? 0 : null };
  };
  let mockPending = [];
  let mockDiscoveryId=0;
  const mockDiscoveries=new Map();
  const demoStars=new Set([11]);
  mockCommands({
    get_settings: () => ({ values: mockValues, defaults: mockDefaults, path: 'C:\\Users\\you\\AppData\\Roaming\\maru\\Maru\\config\\config.toml', pending_restart: mockPending, lite_from_env: false, live_models:[{model:'hy_mt2',path:'D:/models/Hy-MT2-7B-Q4_K_M.gguf'},{model:'milmmt',path:'D:/models/MiLMMT-46-4B-v1.0.Q6_K.gguf'},{model:'hy_mt2_small',path:'D:/models/Hy-MT2-1.8B-Q4_K_M.gguf'},{model:'lfm',path:'D:/models/lfm2-koen-Q5_K_M.gguf'}] }),
    save_settings: ({ changes }) => {
      const next = JSON.parse(JSON.stringify(mockValues));
      for (const [path, v] of Object.entries(changes)) path.split('.').reduce((t, p, i, a) => (i === a.length - 1 ? (t[p] = v === null ? path.split('.').reduce((d, k) => d?.[k], mockDefaults) : v) : t[p]), next);
      mockValues = next;
      const restart = Object.keys(changes).filter((p) => !p.startsWith('hotkeys.') && !p.startsWith('ui.') && p!=='llm.servers' && !p.startsWith('tasks.card_'));
      mockPending = [...new Set([...mockPending, ...restart])];
      return { changed: Object.keys(changes), restart: mockPending, warnings: [] };
    },
    restart_app: () => {},
    task_status: () => ['live', 'capture', 'card_meanings', 'card_sentence'].map((task) => demoTask(task, false)),
    task_test: ({ task }) => new Promise((resolve) => setTimeout(() => resolve(demoTask(task, true)), 500)),
    connection_check: ({ name, endpoint }) => new Promise((resolve) => setTimeout(() => resolve(name === 'Gemini' ? { state: 'quota', message: 'Gemini has no quota left.', endpoint: null } : /^https?:\/\//.test(endpoint) ? { state: 'working', message: `${name || 'The server'} answered in 0.4 s.`, endpoint } : { state: 'input', message: 'Enter a complete server address, such as http://192.168.1.20:11434.', endpoint: null }), 450)),
    anki_check: () => 'Anki is reachable (AnkiConnect v6); the note type "Maru Korean" is ready; 7 decks.',
    anki_sync: () => null,
    anki_refresh: () => 7,
    anki_due: () => ({ due: 12, new: 5 }),
    tts_test: () => Promise.reject('The local voice is not running yet. Save and restart Maru to start it.'),
    test_model_connection: ({modelName}) => new Promise(resolve => setTimeout(() => resolve({reply:`Hello. (demo: ${modelName})`, took_ms:450}),450)),
    discovery_networks: () => [{cidr:'192.168.18.0/24',label:'Ethernet · 192.168.18.0/24'}],
    discovery_start: ({kind,target}) => {
      const id=++mockDiscoveryId;
      const endpoint=kind==='lan' ? 'http://192.168.18.217:11434' : target.includes('://') ? target : `http://${target.includes(':') ? target : `${target}:11434`}`;
      mockDiscoveries.set(id,{started:Date.now(),checked:0,total:kind==='lan'?762:3,done:false,cancelled:false,servers:[{endpoint,api:'Ollama (demo)',models:['gemma:12b','hy-mt2:7b']}],errors:[]});return id;
    },
    discovery_status: ({jobId}) => {
      const state=mockDiscoveries.get(jobId);if(!state)throw Error('Unknown search');
      if(!state.cancelled){state.done=Date.now()-state.started>1600;state.checked=state.done?state.total:Math.min(state.total,Math.floor((Date.now()-state.started)/10));}return state;
    },
    discovery_cancel: ({jobId}) => {const state=mockDiscoveries.get(jobId);if(state){state.done=true;state.cancelled=true;}},
    open_config_file: () => {},
  });
  mockCommands({
    due_words: () => [{ key: '공부하다', meaning: 'to study', level: 'TOPIK 1', state: 'learning', lookups: 2, saved: true }, { key: '학교', meaning: 'school', state: 'learning', lookups: 2, saved: true }],
    word_contexts: () => [{ sentence: '저는 학교에서 공부했어요', translation: 'I studied at school.' }],
    review_word: () => ({}),
    history: () => [{ id: 1, at: 1760000000, source: '저는 학교에서 공부했어요', translation: 'I studied at school.', direction: 'ko_to_en', app: 'chrome.exe', live: false, first_ms: null, total_ms: null }, { id: 2, at: 1760000030, source: '다시는 못 볼 줄 알았어요', translation: 'I thought I would never see you again.', direction: 'ko_to_en', app: 'game.exe', live: true, first_ms: 820, total_ms: 1410, saved_id: 11, starred: demoStars.has(11) }, { id: 3, at: 1760000034, source: '저도요', translation: 'Me too.', direction: 'ko_to_en', app: 'game.exe', live: true, first_ms: 610, total_ms: 790, saved_id: 12, starred: demoStars.has(12) }],
    history_starred: () => [{ id: 0, at: 1759990000, source: '이걸 타면 위로 갈 수 있어요.', translation: 'If you ride this, you can go up.', direction: 'ko_to_en', app: 'game.exe', live: true, first_ms: 400, total_ms: 900, saved_id: 3, starred: true }, ...[...demoStars].map((id) => id === 11 ? { id: 2, at: 1760000030, source: '다시는 못 볼 줄 알았어요', translation: 'I thought I would never see you again.', direction: 'ko_to_en', app: 'game.exe', live: true, saved_id: 11, starred: true } : { id: 3, at: 1760000034, source: '저도요', translation: 'Me too.', direction: 'ko_to_en', app: 'game.exe', live: true, saved_id: 12, starred: true })],
    mine_sheet: ({ sentence, translation, app, ask }) => new Promise((resolve) => setTimeout(() => resolve({ sentence, translation, app, app_name: 'Game',
      // The demo's card connection is out of quota, so this PC's model answered; "Retry with NAS" answers as NAS.
      note: ask?.connection ? null : 'Gemini has no quota left. This PC\'s model answered instead.',
      answered: ask?.connection && ask.connection !== 'local' ? { connection: ask.connection, model: 'gemma4-12b', place: 'network' } : { connection: 'local', model: 'gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL', place: 'this_pc' },
      attempts: ask?.connection ? [] : [{ connection: 'Gemini', model: 'gemini-flash', problem: 'quota', message: 'Gemini has no quota left.' }],
      retry: [{ connection: 'local', model: 'gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL', place: 'this_pc', after_live: false }, { connection: 'NAS', model: 'gemma4-12b', place: 'network', after_live: false }, { connection: 'Claude', model: 'claude-sonnet-5-5', place: 'hosted', after_live: false }],
      live_running: false, word_deck: 'Korean::Words', sentence_deck: 'Korean::Sentences', queue: { words: { deck: 'Korean::Words', waiting: 12, per_day: 3 }, sentences: { deck: 'Korean::Sentences', waiting: 5, per_day: 1 } }, sentence_in_anki: false, anki_error: null, reverse: 'off', audio: true, decks: ['Default', 'Korean::Words', 'Korean::Sentences'], suggestions: [{ deck: 'Korean::Words', reason: 'your deck for word cards', exists: true }],
      words: [{ key: '다시', surface: '다시는', pos: 'adverb', level: 'TOPIK 1', formula: '다시 + 는', dictionary: ['again', 'never again'], meaning: 'never again', meaning_es: 'nunca más', source: 'chosen', senses: [{ en: 'again', es: 'otra vez; de nuevo' }, { en: 'never again', es: 'nunca más' }], in_context: true, known: false, in_anki: false, pick: true }, { key: '못', surface: '못', pos: 'adverb', level: 'TOPIK 1', formula: '', dictionary: ['cannot'], meaning: 'cannot', in_context: true, known: true, in_anki: false, pick: false }, { key: '보다', surface: '볼', pos: 'verb', level: 'TOPIK 1', formula: '보다 + ㄹ', dictionary: ['to see', 'to look at', 'to watch'], meaning: 'to see', in_context: true, known: false, in_anki: true, pick: false }, { key: '알다', surface: '알았어요', pos: 'verb', level: 'TOPIK 1', formula: '알다 + 았 + 어요', corrected: '알아듣다', dictionary: ['know'], meaning: 'know', meaning_es: 'saber; conocer', source: 'one_sense', senses: [{ en: 'know', es: 'saber; conocer' }], in_context: false, known: false, in_anki: false, pick: true }], credit: 'Meanings: 한국어기초사전 (국립국어원, krdict.korean.go.kr), CC BY-SA 2.0 KR' }), 600)),
    mine_queue: ({ wordDeck, sentenceDeck }) => ({ words: { deck: wordDeck, waiting: wordDeck === 'Korean::Words' ? 12 : 0, per_day: 3 }, sentences: { deck: sentenceDeck, waiting: sentenceDeck === 'Korean::Sentences' ? 5 : 0, per_day: 1 } }),
    mine_sentence_translation: ({ ask }) => new Promise((resolve) => setTimeout(() => resolve(ask?.connection ? { translation: 'I thought I would never get to see you again.', answered: { connection: ask.connection, model: 'demo', place: 'network' }, attempts: [], note: null } : { translation: null, answered: null, attempts: [], note: null }), 400)),
    mine_add: ({ request }) => new Promise((resolve) => setTimeout(() => resolve({ audio_note: null, outcomes: [...request.words.map((w, n) => n === 1 ? { what: w.key, kind: 'word', result: 'failed', detail: 'Another Anki add is still running; wait for it to finish.' } : { what: w.key, kind: 'word', result: 'added', detail: request.reverse ? `2 cards: ${request.deck}, reverse in ${request.deck}::Reverse` : `1 card in ${request.deck}` }), ...(request.sentence_card ? [{ what: request.sentence, kind: 'sentence', result: 'added', detail: `sentence card in ${request.sentence_deck}` }] : [])] }), 700)),
    history_star: ({ savedId, starred }) => { if (starred) demoStars.add(savedId); else demoStars.delete(savedId); },
    history_open: () => {},
    captions_status: () => ({ enabled: true, listening: true, port: 18098, problem: null, accepted: 12, dropped: 3, refused: 0 }),
    status: () => ({ notes: ['Korean analyser: Kiwi', 'OCR: PP-OCR on cpu'], llm: 'loads on demand', analyser: 'kiwi', ocr: true, dictionary: true, learner_dictionary: true, hotkeys: { translate: 'Ctrl+Alt+T | F6', region: 'Ctrl+Alt+R', hover: 'Ctrl+Alt+H | F8', continuous: 'Ctrl+Alt+C', continuous_area: 'Ctrl+Alt+Shift+C', scan: 'Ctrl+Alt+S', speak: 'Ctrl+Alt+P', history: 'Ctrl+Alt+L', dismiss: 'Ctrl+Alt+Escape', star: 'Ctrl+Alt+B' }, vocab: { total: 42, saved: 12, known: 20, due_now: 2 }, anki: false, hover_mode: true, live_mode: true, scan_visible: false, lite: false, live_area: { x: 320, y: 880, w: 1280, h: 120 }, ocr_reader: 'loads on demand', tts: 'loads on demand', tts_source: 'local' }),
  });
}
show(tab);
