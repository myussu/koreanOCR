// What the settings page shows and how it edits it. Pure functions, no DOM, so they are tested.
//
// Each field names its place in config.toml by dotted path ("ui.font_size_px"). The page keeps only
// what the user changed and sends that; the backend edits the file and says what needs a restart.

/** Mirrors `maru_core::settings::applies_live`: shortcuts and the panel's look need no restart. */
export const appliesLive = (path) => path.startsWith('hotkeys.') || path.startsWith('ui.');

const opts = (...pairs) => pairs.map(([value, label]) => ({ value, label }));

export const SECTIONS = [
  {
    id: 'shortcuts',
    title: 'Shortcuts',
    intro:
      'Every action can have several keys. Games often use Ctrl, Shift and Alt themselves, so add a plain key such as F7 or Pause as an alternative. A key registered here is taken away from every other program while Maru runs.',
    fields: [
      { path: 'hotkeys.translate', label: 'Translate selection', help: 'Selected text, or the text under the pointer', type: 'keys' },
      { path: 'hotkeys.scan', label: 'Label all Korean text on screen', help: 'Game menus: press again to rescan', type: 'keys' },
      { path: 'hotkeys.speak', label: 'Translate and read aloud', help: 'Reads the selection in Korean; again while reading to stop. Needs Speech turned on', type: 'keys' },
      { path: 'hotkeys.region', label: 'Pick a region', help: 'Drag a rectangle and translate what is inside', type: 'keys' },
      { path: 'hotkeys.hover', label: 'Hover mode on/off', help: 'Rest the pointer on a word', type: 'keys' },
      { path: 'hotkeys.continuous', label: 'Live subtitles on/off', help: 'Watches the last chosen area', type: 'keys' },
      { path: 'hotkeys.continuous_area', label: 'Live subtitles: choose area', help: 'Pick a new area and start there', type: 'keys' },
      { path: 'hotkeys.history', label: 'History', help: 'Recent lines', type: 'keys' },
      { path: 'hotkeys.dismiss', label: 'Dismiss', help: 'Hides the panel and the screen labels', type: 'keys' },
    ],
  },
  {
    id: 'appearance',
    title: 'Panel',
    fields: [
      { path: 'ui.font_size_px', label: 'Text size', type: 'range', min: 12, max: 32, step: 1, unit: 'px' },
      { path: 'ui.max_width_px', label: 'Panel width', type: 'range', min: 400, max: 1200, step: 20, unit: 'px' },
      { path: 'ui.theme', label: 'Theme', type: 'select', options: opts(['auto', 'Follow Windows'], ['light', 'Light'], ['dark', 'Dark']) },
      { path: 'ui.placement', label: 'Panel position', help: 'Relative to the text it explains', type: 'select', options: opts(['auto', 'Below, or above if there is no room'], ['below', 'Below'], ['above', 'Above']) },
      { path: 'ui.show_romanization', label: 'Show romanization', type: 'toggle' },
      { path: 'ui.show_pronunciation', label: 'Show pronunciation', help: 'How the word sounds when it differs from the spelling', type: 'toggle' },
      { path: 'ui.show_grammar_formula', label: 'Show how a word is built', help: 'e.g. 학교 + 에서', type: 'toggle' },
    ],
  },
  {
    id: 'learning',
    title: 'Learning',
    fields: [
      {
        path: 'learning.gloss_mode',
        label: 'Meanings under words',
        help: 'Also decides which screen labels are worth showing',
        type: 'select',
        options: opts(['all', 'Every word'], ['fade_known', 'Dim words I know'], ['hide_known', 'Hide words I know']),
      },
      { path: 'learning.register', label: 'Politeness for English → Korean', type: 'select', options: opts(['polite', 'Polite (해요)'], ['formal', 'Formal (합니다)'], ['casual', 'Casual (반말)']) },
      { path: 'learning.track_lookups', label: 'Count my lookups', help: 'Needed for fading known words and for review', type: 'toggle' },
      { path: 'learning.auto_save_after_lookups', label: 'Add to review after', help: 'Looked-up words join the review queue on this lookup', type: 'number', min: 1, max: 10, step: 1, unit: 'lookups' },
      { path: 'learning.known_stability_days', label: 'A word counts as known after', type: 'number', min: 1, max: 365, step: 1, unit: 'days of memory', advanced: true },
      { path: 'learning.desired_retention', label: 'Recall target for review', help: 'Higher means more reviews', type: 'percent', min: 70, max: 97, step: 1, advanced: true },
    ],
  },
  {
    id: 'screen',
    title: 'Reading the screen',
    fields: [
      { path: 'ocr.vision_reader', label: 'Use the OCR reading model', help: 'PaddleOCR-VL: better letters, about 2 GB of video memory. Install it with maru setup ocr-vl', type: 'toggle' },
      { path: 'capture.use_accessibility', label: 'Read selected text directly', help: 'The fastest and most exact way, where the program allows it', type: 'toggle' },
      { path: 'capture.use_clipboard_fallback', label: 'Copy the selection if that fails', help: 'Your clipboard is saved and put back', type: 'toggle' },
      { path: 'capture.use_ocr_fallback', label: 'Read the pixels if that fails', help: 'Images, videos and games', type: 'toggle' },
      { path: 'capture.ocr_radius_px', label: 'How far around the pointer to read', type: 'range', min: 100, max: 800, step: 20, unit: 'px' },
      { path: 'ocr.backend', label: 'Text reader', type: 'select', options: opts(['auto', 'Automatic'], ['paddle', 'PP-OCR (included)'], ['windows', 'Windows built-in']), advanced: true },
      { path: 'ocr.device', label: 'Run PP-OCR on', help: 'GPU choices need a build that supports them', type: 'select', options: opts(['cpu', 'CPU'], ['cuda:0', 'NVIDIA GPU (CUDA)'], ['directml:0', 'Any GPU (DirectML)']), advanced: true },
      { path: 'ocr.min_confidence', label: 'Warn the translator below this confidence', type: 'number', min: 0, max: 1, step: 0.05, advanced: true },
      { path: 'capture.max_chars', label: 'Longest text to process', type: 'number', min: 20, max: 5000, step: 10, unit: 'characters', advanced: true },
    ],
  },
  {
    id: 'live',
    title: 'Live subtitles',
    fields: [
      { path: 'continuous.interval_ms', label: 'Look at the area every', help: 'Lower is quicker to notice a new line and uses more CPU', type: 'range', min: 100, max: 1000, step: 25, unit: 'ms' },
      { path: 'continuous.stable_ms', label: 'Wait for the text to hold still', help: 'Avoids translating half-typed lines', type: 'range', min: 100, max: 1500, step: 25, unit: 'ms' },
      { path: 'continuous.diff_threshold', label: 'How much must change to count', type: 'number', min: 0.001, max: 0.2, step: 0.001, advanced: true },
      { path: 'continuous.history_len', label: 'Lines kept in the history', type: 'number', min: 10, max: 500, step: 10, advanced: true },
    ],
  },
  {
    id: 'model',
    title: 'Translation model',
    intro: 'These are read when Maru starts, so changing them asks for a restart. The language model loads again afterwards.',
    fields: [
      { path: 'lite', label: 'Game mode (lighter)', help: 'No OCR reading model, one model slot, a smaller context and a smaller translation model, so a game keeps its memory. The first start downloads the smaller model', type: 'toggle' },
      { path: 'llm.hf_repo', label: 'Model to download', help: 'A Hugging Face repository and quantisation, fetched on first start', type: 'text', nullable: true, placeholder: 'unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL' },
      { path: 'llm.model_path', label: 'Or a model file you already have', help: 'Wins over the download', type: 'text', nullable: true, placeholder: 'D:/models/model.gguf' },
      { path: 'llm.endpoint', label: 'Or a server that is already running', help: 'LM Studio, Ollama, a PC on your network', type: 'text', nullable: true, placeholder: 'http://127.0.0.1:1234/v1' },
      { path: 'llm.parallel', label: 'Requests at once', help: 'Two lets translation and word meanings run together', type: 'number', min: 1, max: 4, step: 1 },
      { path: 'llm.ctx_size', label: 'Context per request', type: 'number', min: 512, max: 32768, step: 512, unit: 'tokens', advanced: true },
      { path: 'llm.gpu_layers', label: 'Layers on the GPU', help: 'all, auto or a number', type: 'text', advanced: true },
      { path: 'llm.flash_attn', label: 'Flash attention', type: 'toggle', advanced: true },
      { path: 'llm.temperature', label: 'Temperature', help: 'Lower is more literal', type: 'number', min: 0, max: 2, step: 0.05, advanced: true },
      { path: 'llm.context_lines', label: 'Earlier lines the model sees', type: 'number', min: 0, max: 20, step: 1, advanced: true },
      { path: 'lite_profile.hf_repo', label: 'Game mode: model to download', help: 'Empty keeps the model above', type: 'text', advanced: true },
      { path: 'lite_profile.ctx_size', label: 'Game mode: context', type: 'number', min: 512, max: 32768, step: 512, advanced: true },
      { path: 'lite_profile.parallel', label: 'Game mode: requests at once', type: 'number', min: 1, max: 4, step: 1, advanced: true },
    ],
  },
];

const SPEECH_ONLY_SERVER = { path: 'tts.provider', value: 'endpoint' };
const SPEECH_ONLY_LOCAL = { path: 'tts.provider', value: 'local' };

export const SPEECH = {
  id: 'speech',
  title: 'Speech',
  intro:
    'Hear Korean read aloud, for studying. A voice on this PC uses the GPU; a server on another PC (or a paid service) does not. Anything that speaks the OpenAI speech API works, including the same local voice running on a second computer. Changes need a restart; "Test voice" tries the saved settings.',
  action: 'tts-test',
  fields: [
    { path: 'tts.enabled', label: 'Read Korean aloud', type: 'toggle' },
    {
      path: 'tts.provider',
      label: 'Where the voice runs',
      type: 'select',
      options: opts(['local', 'On this PC (uses the GPU)'], ['endpoint', 'A server: another PC, or a paid service']),
    },
    { path: 'tts.endpoint', label: 'Server address', help: 'e.g. http://192.168.1.20:8080 or https://api.openai.com/v1', type: 'text', nullable: true, placeholder: 'http://192.168.1.20:8080', when: SPEECH_ONLY_SERVER },
    { path: 'tts.api_key', label: 'API key', help: 'Only paid services need one. Stored in config.toml on this PC', type: 'password', nullable: true, when: SPEECH_ONLY_SERVER },
    { path: 'tts.model', label: 'Model name', help: 'qwen3-tts for the local voice; tts-1 or gpt-4o-mini-tts for OpenAI', type: 'text', when: SPEECH_ONLY_SERVER },
    { path: 'tts.voice', label: 'Voice', help: 'sohee is the Korean voice of the local model; services have their own names', type: 'text' },
    { path: 'tts.language', label: 'Language label', help: 'Empty = Korean for the local voice, the server decides otherwise', type: 'text', advanced: true },
    { path: 'tts.instructions', label: 'Speaking style', help: 'e.g. "Speak clearly and a little slowly". Only some models obey it', type: 'text', nullable: true, advanced: true },
    { path: 'tts.speed', label: 'Speed', help: 'The local voice ignores this', type: 'number', min: 0.25, max: 4, step: 0.05, advanced: true },
    { path: 'tts.cache_mb', label: 'Keep spoken sentences up to', help: 'Hearing a sentence again is then instant and free. 0 = off', type: 'number', min: 0, max: 20000, step: 50, unit: 'MB', advanced: true },
    { path: 'tts.device', label: 'Run the local voice on', type: 'select', options: opts(['auto', 'Automatic'], ['CUDA0', 'NVIDIA (CUDA)'], ['Vulkan0', 'Vulkan'], ['CPU', 'CPU']), advanced: true, when: SPEECH_ONLY_LOCAL },
    { path: 'tts.run_in_lite', label: 'Also in game mode', help: 'Off, so a game keeps the GPU', type: 'toggle', advanced: true, when: SPEECH_ONLY_LOCAL },
    { path: 'tts.port', label: 'Local voice port', type: 'number', min: 1024, max: 65535, step: 1, advanced: true, when: SPEECH_ONLY_LOCAL },
  ],
};

SECTIONS.splice(SECTIONS.findIndex((x) => x.id === 'model'), 0, SPEECH);

export const ANKI = {
  id: 'anki',
  title: 'Anki',
  intro:
    'Anki is where words are reviewed: Maru adds them (to the deck you choose, with tags and spoken audio), reads back how well you hold them so known words fade, and asks Anki to sync with AnkiWeb. Your AnkiWeb login stays inside Anki (Tools → Preferences → Syncing); Maru never sees it. Needs Anki open with the AnkiConnect add-on (code 2055492159). Changes need a restart.',
  action: 'anki',
  fields: [
    { path: 'learning.anki.enabled', label: 'Review in Anki', type: 'toggle' },
    { path: 'learning.anki.deck', label: 'Default deck', help: 'Suggested when nothing better is known; other decks are offered per program (Korean::Games::Elden Ring)', type: 'text' },
    { path: 'learning.anki.reverse', label: 'English → Korean card', help: 'Each word also gets a card that shows the English and reveals the Korean', type: 'select', options: opts(['subdeck', 'In a Reverse subdeck of the same deck'], ['same_deck', 'In the same deck'], ['off', 'None']) },
    { path: 'learning.anki.audio', label: 'Add spoken audio', help: 'The word and its sentence, from the voice in Speech', type: 'toggle' },
    { path: 'learning.anki.sync_after_add', label: 'Sync with AnkiWeb after adding', help: 'Asks Anki to sync a few seconds after the last word, so your other copies of Anki get it', type: 'toggle' },
    { path: 'learning.anki.auto_tags', label: 'Tag by level, type, program and month', help: 'topik-1, pos::verb, app::elden-ring, added::2026-10', type: 'toggle' },
    { path: 'learning.anki.tags', label: 'Tags every word gets', help: 'Separated by commas', type: 'tags' },
    { path: 'learning.anki.refresh_minutes', label: 'Read your progress back every', help: '0 = only when Maru starts', type: 'number', min: 0, max: 1440, step: 5, unit: 'minutes', advanced: true },
    { path: 'learning.anki.url', label: 'AnkiConnect address', type: 'text', advanced: true },
    { path: 'learning.anki.api_key', label: 'AnkiConnect API key', help: 'Only if you set one in the add-on', type: 'password', nullable: true, advanced: true },
    { path: 'learning.anki.note_type', label: 'Note type', help: 'Maru creates and keeps it up to date', type: 'text', advanced: true },
    { path: 'learning.anki.reverse_subdeck', label: 'Reverse subdeck name', type: 'text', advanced: true },
  ],
};

SECTIONS.splice(SECTIONS.findIndex((x) => x.id === 'screen'), 0, ANKI);

export const ALL_FIELDS = SECTIONS.flatMap((s) => s.fields);
/** Does a field show up now? Some only matter for one choice of another setting. */
export function isVisible(field, draft, current, defaults) {
  return !field.when || shown(draft, current, defaults, field.when.path) === field.when.value;
}

export const labelFor = (path) => ALL_FIELDS.find((f) => f.path === path)?.label ?? path;

// ───────────── values ─────────────

export const valueAt = (obj, path) => path.split('.').reduce((o, k) => (o == null ? undefined : o[k]), obj);

/** Settings as the page edits them: numbers rounded, percent fields as whole percents. */
export function toEditor(field, raw) {
  if (field.type === 'percent') return Math.round((raw ?? 0) * 100);
  if (field.type === 'number' || field.type === 'range') return typeof raw === 'number' ? Math.round(raw * 1e6) / 1e6 : raw;
  if (field.type === 'text' || field.type === 'password') return raw ?? '';
  if (field.type === 'tags') return Array.isArray(raw) ? raw.join(', ') : '';
  return raw;
}

/** The value to store for what the editor holds: nullable empty text removes the key. */
export function fromEditor(field, edited) {
  if (field.type === 'percent') return Math.round(edited) / 100;
  if (field.type === 'tags') return String(edited).split(',').map((t) => t.trim()).filter(Boolean);
  if (field.type === 'text' || field.type === 'password') {
    const t = String(edited).trim();
    return field.nullable && t === '' ? null : t;
  }
  return edited;
}

const same = (a, b) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
export const sameValue = same;

/**
 * Track one edit. `draft` maps path → new value (null = back to the default); an edit that puts
 * the value back where the file already has it leaves the draft.
 */
export function withEdit(draft, current, path, value) {
  const next = { ...draft };
  if (same(value, valueAt(current, path))) delete next[path];
  else next[path] = value;
  return next;
}

/** What the user sees for a field: the draft if edited (null = default), else the file. */
export function shown(draft, current, defaults, path) {
  if (path in draft) return draft[path] === null ? valueAt(defaults, path) : draft[path];
  return valueAt(current, path);
}

export function isDefault(draft, current, defaults, path) {
  return same(shown(draft, current, defaults, path), valueAt(defaults, path));
}

/** Put a field back to its default: a `null` change removes the key from the file. */
export function withReset(draft, current, defaults, path) {
  if (same(valueAt(current, path), valueAt(defaults, path)) && !(path in draft)) return draft;
  return { ...draft, [path]: null };
}

// ───────────── shortcuts ─────────────

/** `"Ctrl+Alt+T | F6"` → `["Ctrl+Alt+T", "F6"]` (the backend accepts `|` and ", " as separators). */
export function splitKeys(text) {
  return String(text ?? '')
    .split('|')
    .map((s) => s.trim())
    .filter(Boolean);
}
export const joinKeys = (list) => list.join(' | ');

const NAMED = {
  Escape: 'Escape', Space: 'Space', Enter: 'Enter', NumpadEnter: 'NumpadEnter', Tab: 'Tab', Backspace: 'Backspace', Delete: 'Delete', Insert: 'Insert',
  Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
  Pause: 'Pause', ScrollLock: 'ScrollLock', PrintScreen: 'PrintScreen', CapsLock: 'CapsLock', NumLock: 'NumLock',
  NumpadAdd: 'NumpadAdd', NumpadSubtract: 'NumpadSubtract', NumpadMultiply: 'NumpadMultiply', NumpadDivide: 'NumpadDivide', NumpadDecimal: 'NumpadDecimal',
  Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\', Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/',
};

/** The shortcut name for a physical key (`KeyboardEvent.code`), or null for modifiers and unknown keys. */
export function keyName(code) {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;
  if (/^Numpad\d$/.test(code)) return code;
  return NAMED[code] ?? null;
}

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/;

/**
 * A key combination typed into the page as a shortcut, `"Ctrl+Alt+T"`.
 * Returns null while only modifiers are down, `'cancel'` for a bare Escape, `'unknown'` for a key
 * the shortcut system has no name for.
 */
export function comboFromEvent(e) {
  if (MODIFIER_CODES.test(e.code)) return null;
  const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Super'].filter(Boolean);
  if (e.code === 'Escape' && !mods.length) return 'cancel';
  const key = keyName(e.code);
  return key ? [...mods, key].join('+') : 'unknown';
}

/** Shortcuts without a modifier are taken from every program; some keys are safer than others. */
const QUIET_KEYS = /^(Pause|ScrollLock|Insert|PrintScreen|Numpad\w+|F(1[3-9]|2[0-4]))$/;
const FUNCTION_KEYS = /^F([1-9]|1[0-2])$/;
export function keyRisk(combo) {
  const parts = combo.split('+');
  const key = parts[parts.length - 1];
  if (parts.length > 1) return null;
  if (QUIET_KEYS.test(key)) return null;
  if (FUNCTION_KEYS.test(key)) return `${key} also works as a normal key in browsers and many games`;
  return `${key} would stop working everywhere else`;
}
