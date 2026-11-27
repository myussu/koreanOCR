// What the settings page shows and how it edits it. Pure functions, no DOM, so they are tested.
//
// The page is organised by what you do with Maru, not by how Maru is built: Live subtitles, Captures, Making
// cards, Voice, Connections, Shortcuts, Advanced. Each field names its place in config.toml by dotted path
// ("ui.font_size_px"), says in one sentence what it does, and lists the features that use it (`uses`), which is
// also what the search looks through. There are three kinds of control and they look different on purpose:
// `choice` (cards: pick one), `toggle` (a switch) and typed fields (`text`, `password`, `number`, `percent`,
// `tags`, `keys`). `route` and `route_optional` are choices whose cards are the connections.
//
// The page keeps only what the user changed and sends that; the backend edits the file and says what needs a restart.
import { LIVE_MODELS } from './settings-live-core.js';

/** Mirrors `maru_core::settings::applies_live`: what the running app picks up without a restart. */
export const appliesLive = (path) => path.startsWith('hotkeys.') || path.startsWith('ui.') || path === 'llm.servers' || path.startsWith('tasks.card_');

const opts = (...triples) => triples.map(([value, label, detail]) => ({ value, label, ...(detail ? { detail } : {}) }));

/** The features a setting can belong to, as the page names them. */
export const FEATURES = { live: 'Live subtitles', snapshot: 'Snapshot', scan: 'Screen scan', hover: 'Hover', cards: 'Make cards', voice: 'Reading aloud', review: 'Review', all: 'Everything' };
const { live: LIVE, snapshot: SNAP, scan: SCAN, hover: HOVER, cards: CARDS, voice: VOICE, review: REVIEW, all: ALL } = FEATURES;

const SMALL_LIVE = LIVE_MODELS.filter((m) => m.path);
const liveIs = (id) => ({ path: 'tasks.live.model', value: id, and: { path: 'tasks.live.connection', value: 'local' } });
const LIVE_LOCAL = { path: 'tasks.live.connection', value: 'local' };
const LIVE_ON_SERVER = { path: 'tasks.live.connection', not: 'local' };
const LIVE_SMALL = { path: 'tasks.live.model', in: SMALL_LIVE.map((m) => m.id), and: LIVE_LOCAL };
const SPEECH_SERVER = { path: 'tts.provider', value: 'endpoint' };
// How each task reads the screen (`[reading]`): the names are the ones `maru_core::reading` knows.
const FINDERS = opts(['pp-ocrv6-small', 'Small and quick', 'Measured best on game subtitles'], ['standard', 'Standard', 'The one Maru has always used'], ['pp-ocrv6-medium', 'Medium', 'Only if it is on this PC']);
// The measured choice is marked on the page; the reason shows when the pointer rests on the card.
const recommend = (options, value, why) => options.map((o) => (o.value === value ? { ...o, recommended: why } : o));
const LIVE_FINDERS = recommend(FINDERS, 'pp-ocrv6-small', 'Read your labelled game subtitles as well as the standard one, 35–60 % faster (runs 3 and 4)');
const LETTERS = opts(['standard', 'Standard'], ['qota-ocr', 'Qota-OCR', 'A community model; only if it is on this PC']);
const READERS = recommend(opts(['paddleocr-vl-1.5', 'PaddleOCR-VL 1.5'], ['paddleocr-vl-1.6', 'PaddleOCR-VL 1.6', 'Install it with maru setup ocr-vl']), 'paddleocr-vl-1.6', 'Better than 1.5 on game subtitles in run 3 (17 lines better, 6 worse), at the same speed');
const SPEECH_LOCAL = { path: 'tts.provider', value: 'local' };

export const SECTIONS = [
  {
    id: 'live',
    title: 'Live subtitles',
    intro: 'Translates the subtitle area of a game or video while you play, line by line.',
    tasks: ['live'],
    fields: [
      { path: 'tasks.live.connection', label: 'Translate live lines with', help: 'The connection that translates each subtitle line as it appears.', type: 'route', task: 'live', uses: [LIVE] },
      { path: 'tasks.live.model', label: 'Model on this PC for live lines', help: 'A translation-only model leaves more video memory for the game but cannot explain words.', type: 'choice', uses: [LIVE], when: LIVE_LOCAL,
        options: LIVE_MODELS.map((m) => ({ value: m.id === 'main' ? '' : m.id, label: m.name, detail: m.note })) },
      ...SMALL_LIVE.map((m) => ({ path: m.path, label: `${m.name} file`, help: 'The GGUF file of this model on this PC.', type: 'text', nullable: true, placeholder: `D:/models/${m.file}`, uses: [LIVE], when: liveIs(m.id) })),
      { path: 'continuous.translation.device', label: 'Run that model on', help: 'The CPU is slower but leaves all the video memory to the game.', type: 'choice', uses: [LIVE], when: LIVE_SMALL, options: opts(['gpu', 'Graphics card', 'Fastest'], ['cpu', 'Processor', 'Leaves the graphics card to the game']) },
      { path: 'continuous.translation.backup_model', label: 'If the server does not answer', help: 'A small model on this PC translates a line the server fails or is slow with.', type: 'choice', nullable: true, uses: [LIVE], when: LIVE_ON_SERVER,
        options: [{ value: null, label: 'Wait for the server', detail: 'No model is loaded on this PC' }, ...SMALL_LIVE.map((m) => ({ value: m.id, label: m.name, detail: 'Loads when live subtitles start; its file must be set' }))] },
      { path: 'continuous.context_lines', label: 'Earlier lines the model sees', help: 'Keeps names and pronouns consistent from one line to the next.', type: 'number', min: 0, max: 20, step: 1, unit: 'lines', uses: [LIVE] },
      { path: 'continuous.linger_ms', label: 'Keep the translation after the line is gone', help: 'How long the translation stays on screen once the original has disappeared.', type: 'number', min: 0, max: 5000, step: 100, unit: 'ms', uses: [LIVE] },
      { path: 'reading.live.detector', label: 'Find subtitle lines with', help: 'The part of the text reader that finds the lines; on game subtitles the small one misread the fewest and is the quickest.', type: 'choice', uses: [LIVE], options: LIVE_FINDERS },
      { path: 'reading.live.reader', label: 'Check live letters with the reading model', help: 'Off is the measured setting, since the text reader alone misread fewer game subtitles; on, it takes about 2 GB of video memory while live subtitles run.', type: 'toggle', uses: [LIVE] },
      { path: 'reading.live.reader_model', label: 'Reading model for live subtitles', help: 'Only one reading model runs at a time, so when captures use one too, live subtitles use theirs.', type: 'choice', uses: [LIVE], when: { path: 'reading.live.reader', value: true }, options: READERS, advanced: true },
      { path: 'reading.live.recogniser', label: 'Read the letters of live subtitles with', help: 'The part of the text reader that reads the letters in each line.', type: 'choice', uses: [LIVE], options: LETTERS, advanced: true },
      { path: 'captions.enabled', label: 'Take YouTube captions from the browser', help: "Maru's browser extension sends a video's own Korean captions as text, so they are not read from the picture.", type: 'toggle', uses: [LIVE, CARDS] },
      { path: 'captions.token', label: 'Code for the browser extension', help: 'Copy it into the extension; Maru writes one when captions are switched on and saved, and emptying it makes a new one.', type: 'text', nullable: true, uses: [LIVE], when: { path: 'captions.enabled', value: true } },
      { path: 'captions.port', label: 'Port the browser extension sends to', help: 'On this PC only; change it here and in the extension if another program uses it.', type: 'number', min: 1024, max: 65535, step: 1, uses: [LIVE], advanced: true },
      { path: 'captions.allowed_origin', label: 'Only this browser extension may send captions', help: "The address the extension's popup shows; empty accepts any extension that has the code, and web pages are always refused.", type: 'text', placeholder: 'chrome-extension://…', uses: [LIVE], advanced: true },
      { path: 'continuous.interval_ms', label: 'Look at the subtitle area every', help: 'Lower notices a new line sooner and uses more processor.', type: 'number', min: 50, max: 1000, step: 25, unit: 'ms', uses: [LIVE], advanced: true },
      { path: 'continuous.stable_ms', label: 'A picture this still is a finished line', help: 'A line that is still fading in is read sooner than this.', type: 'number', min: 50, max: 1500, step: 25, unit: 'ms', uses: [LIVE], advanced: true },
      { path: 'continuous.max_in_flight', label: 'Live lines translated at once', help: '1 translates lines in order; 2 lets them overlap; 0 sets no limit.', type: 'number', min: 0, max: 2, step: 1, uses: [LIVE], advanced: true },
      { path: 'continuous.filter_ocr_jitter', label: 'Compare the lettering before translating a re-read line', help: 'Checks the subtitle pixels so a line read twice with different letters is translated once.', type: 'toggle', uses: [LIVE], advanced: true },
      { path: 'continuous.diff_threshold', label: 'How much of the picture must change', help: 'The share of the area that has to differ for it to be read again.', type: 'number', min: 0.001, max: 0.2, step: 0.001, uses: [LIVE], advanced: true },
      { path: 'continuous.ocr_scale', label: 'Enlarge the subtitle area before reading it', help: 'The text reader sees the area this many times its size: 1 is as it is, and 2 is slower and has not been shown to help.', type: 'number', min: 1, max: 3, step: 0.5, unit: 'times', uses: [LIVE], advanced: true },
      { path: 'continuous.drop_stray_lines', label: 'Drop marks read beside the subtitle', help: 'Leaves out lines with no Hangul read next to a subtitle (M, 0, FIRST), never Latin letters or digits inside a Korean line.', type: 'toggle', uses: [LIVE], advanced: true },
      { path: 'continuous.save_reads', label: 'Save the subtitles it reads', help: 'Keeps a picture and the text read of every live line on this PC, to build a test set from your own games; nothing is sent anywhere.', type: 'toggle', uses: [LIVE], advanced: true },
      { path: 'continuous.save_reads_dir', label: 'Folder for the subtitles it saves', help: 'Empty is Documents\maru-evalset\raw, a folder the label page can open (browsers refuse folders under AppData).', type: 'text', nullable: true, placeholder: 'D:/maru-evalset/raw', uses: [LIVE], when: { path: 'continuous.save_reads', value: true }, advanced: true },
      { path: 'continuous.history_len', label: 'Lines kept in the history', help: 'How many recent lines History shows.', type: 'number', min: 10, max: 2000, step: 10, unit: 'lines', uses: [LIVE, CARDS], advanced: true },
      { path: 'continuous.translation.backup_after_ms', label: 'Wait for the server before using the backup', help: 'How long the server may take to start answering a line.', type: 'number', min: 300, max: 20000, step: 100, unit: 'ms', uses: [LIVE], advanced: true },
      { path: 'continuous.translation.cpu_threads', label: 'Processor threads for the translation-only model', help: 'Used when that model runs on the processor.', type: 'number', min: 1, max: 32, step: 1, unit: 'threads', uses: [LIVE], advanced: true },
      { path: 'continuous.translation.dialogue_context', label: 'Give translation-only models the earlier lines', help: 'Sometimes fixes pronouns and sometimes changes the meaning; off is the tested setting.', type: 'toggle', uses: [LIVE], advanced: true },
    ],
  },
  {
    id: 'captures',
    title: 'Captures',
    subtitle: 'snapshot, hover and screen scan',
    intro: 'What you ask for yourself: translating a selection or a region, resting the pointer on a word, labelling a whole screen.',
    tasks: ['capture'],
    fields: [
      { path: 'tasks.capture.connection', label: 'Translate captures with', help: 'The connection that translates a snapshot, labels a scanned screen and explains a word; hover uses the dictionary only.', type: 'route', task: 'capture', uses: [SNAP, SCAN] },
      { path: 'tasks.capture.while_live', label: 'While live subtitles are on, use', help: 'Another connection for captures while live subtitles run, so this PC does not swap models in the middle of a game.', type: 'route_optional', none: 'The same connection', uses: [SNAP, SCAN, LIVE] },
      { path: 'learning.gloss_mode', label: 'Meanings under words', help: 'Which words get a meaning under them, and which screen labels are worth showing.', type: 'choice', uses: [SNAP, SCAN], options: opts(['all', 'Every word'], ['fade_known', 'Dim the words I know'], ['hide_known', 'Hide the words I know']) },
      { path: 'learning.register', label: 'Politeness for English → Korean', help: 'The speech level Maru writes when you translate English into Korean.', type: 'choice', uses: [SNAP], options: opts(['polite', 'Polite', '해요'], ['formal', 'Formal', '합니다'], ['casual', 'Casual', '반말']) },
      { path: 'reading.capture.reader', label: 'Use the reading model for sharper letters', help: 'Sharper letters on menus and stylised text; it loads with the first capture that needs it and then takes about 2 GB of video memory (maru setup ocr-vl).', type: 'toggle', uses: [SNAP] },
      { path: 'reading.capture.reader_model', label: 'Reading model for captures', help: 'Which reading model checks the letters of a capture.', type: 'choice', uses: [SNAP], when: { path: 'reading.capture.reader', value: true }, options: READERS },
      { path: 'reading.capture.detector', label: 'Find the lines of a capture with', help: 'Also used by the screen scan, which reads the whole screen at full size without the reading model.', type: 'choice', uses: [SNAP, SCAN], options: FINDERS, advanced: true },
      { path: 'reading.capture.recogniser', label: 'Read the letters of a capture with', help: 'Also used by the screen scan.', type: 'choice', uses: [SNAP, SCAN], options: LETTERS, advanced: true },
      { path: 'capture.use_accessibility', label: 'Read selected text directly', help: 'The exact text of a selection, where the program allows it.', type: 'toggle', uses: [SNAP, HOVER] },
      { path: 'capture.use_clipboard_fallback', label: 'Copy the selection if that fails', help: 'Your clipboard is saved and put back.', type: 'toggle', uses: [SNAP] },
      { path: 'capture.use_ocr_fallback', label: 'Read the pixels if that fails', help: 'For images, videos and games, where text cannot be selected.', type: 'toggle', uses: [SNAP, HOVER] },
      { path: 'capture.ocr_radius_px', label: 'How far around the pointer to read', help: 'The size of the area read when nothing is selected.', type: 'number', min: 100, max: 800, step: 20, unit: 'px', uses: [SNAP, HOVER] },
      { path: 'ui.font_size_px', label: 'Panel text size', help: 'The size of the text in the panel that shows a capture.', type: 'number', min: 12, max: 32, step: 1, unit: 'px', uses: [SNAP, HOVER] },
      { path: 'ui.max_width_px', label: 'Panel width', help: 'The widest the panel may get.', type: 'number', min: 400, max: 1200, step: 20, unit: 'px', uses: [SNAP, HOVER] },
      { path: 'ui.theme', label: 'Colours', help: 'Light or dark, or whatever Windows uses.', type: 'choice', uses: [ALL], options: opts(['auto', 'Follow Windows'], ['light', 'Light'], ['dark', 'Dark']) },
      { path: 'ui.placement', label: 'Panel position', help: 'Where the panel goes relative to the text it explains.', type: 'choice', uses: [SNAP, HOVER], options: opts(['auto', 'Below, or above', 'Above when there is no room below'], ['below', 'Below'], ['above', 'Above']) },
      { path: 'ui.show_romanization', label: 'Show romanization', help: 'The word written in Latin letters.', type: 'toggle', uses: [SNAP, HOVER] },
      { path: 'ui.show_pronunciation', label: 'Show pronunciation', help: 'How the word sounds when that differs from its spelling.', type: 'toggle', uses: [SNAP, HOVER] },
      { path: 'ui.show_grammar_formula', label: 'Show how a word is built', help: 'For example 학교 + 에서.', type: 'toggle', uses: [SNAP, HOVER] },
      { path: 'ui.launcher', label: 'Show the floating controls at start-up', help: 'The small Maru button over the game; the tray or its shortcut can show it again.', type: 'toggle', uses: [ALL] },
      { path: 'ocr.backend', label: 'Text reader', help: 'Which reader turns pixels into text.', type: 'choice', uses: [SNAP, SCAN, LIVE, HOVER], advanced: true, options: opts(['auto', 'Automatic'], ['paddle', 'PP-OCR', 'Included with Maru'], ['windows', 'Windows built-in']) },
      { path: 'ocr.device', label: 'Run the text reader on', help: 'The graphics card reads a subtitle line in about a third of the time and frees the processor; if it cannot, the processor is used.', type: 'choice', uses: [SNAP, SCAN, LIVE, HOVER], advanced: true, options: opts(['cpu', 'Processor'], ['directml:0', 'Any graphics card', 'DirectML'], ['cuda:0', 'NVIDIA graphics card', 'Needs a build with CUDA']) },
      { path: 'ocr.threads', label: 'Processor threads for the text reader', help: '0 uses every core; a small number leaves cores to the game during live subtitles.', type: 'number', min: 0, max: 64, step: 1, unit: 'threads', uses: [SNAP, SCAN, LIVE, HOVER], advanced: true },
      { path: 'ocr.min_confidence', label: 'Tell the translator the text may be misread below', help: 'A reading less certain than this is flagged to the model as possibly wrong.', type: 'number', min: 0, max: 1, step: 0.05, uses: [SNAP, LIVE], advanced: true },
      { path: 'capture.max_chars', label: 'Longest text to translate', help: 'A longer selection is cut at the end of a sentence.', type: 'number', min: 20, max: 5000, step: 10, unit: 'characters', uses: [SNAP], advanced: true },
      { path: 'llm.context_lines', label: 'Earlier captures the model sees', help: 'Recent lines from the same program, for continuity.', type: 'number', min: 0, max: 20, step: 1, unit: 'lines', uses: [SNAP], advanced: true },
      { path: 'llm.temperature', label: 'How freely the model words things', help: 'Lower is more literal.', type: 'number', min: 0, max: 2, step: 0.05, uses: [SNAP, SCAN, LIVE], advanced: true },
    ],
  },
  {
    id: 'cards',
    title: 'Making cards',
    intro: 'Turns a line from History into Anki cards: one for each word you pick and one for the sentence. Review happens in Anki.',
    tasks: ['card_meanings', 'card_sentence'],
    action: 'anki',
    fields: [
      { path: 'tasks.card_meanings.connection', label: 'Choose word meanings with', help: "Asked first for the meaning of each word in its line; if it cannot answer, this PC's model is asked, then the dictionary is used.", type: 'route', task: 'card_meanings', uses: [CARDS] },
      { path: 'tasks.card_sentence.connection', label: 'Translate the sentence again with', help: 'A fresh translation for the sentence card, instead of the one made when the line was read.', type: 'route_optional', none: 'Keep the translation the line has', uses: [CARDS] },
      { path: 'learning.anki.enabled', label: 'Send cards to Anki', help: 'Anki must be open with the AnkiConnect add-on (code 2055492159).', type: 'toggle', uses: [CARDS, REVIEW] },
      { path: 'learning.anki.deck', label: 'Deck for word cards', help: 'Where a card for a word goes unless you choose another deck when adding.', type: 'text', uses: [CARDS] },
      { path: 'learning.anki.sentence_deck', label: 'Deck for sentence cards', help: 'Where a card for a whole line goes; empty uses the deck for word cards.', type: 'text', uses: [CARDS] },
      { path: 'learning.anki.new_words_per_day', label: 'New word cards you study per day', help: "Only used to tell you when the cards you add would come up; Anki's own limits are not changed.", type: 'number', min: 1, max: 200, step: 1, unit: 'a day', uses: [CARDS] },
      { path: 'learning.anki.new_sentences_per_day', label: 'New sentence cards you study per day', help: 'The same, for the sentence deck.', type: 'number', min: 1, max: 200, step: 1, unit: 'a day', uses: [CARDS] },
      { path: 'learning.anki.reverse', label: 'English → Korean card', help: 'A second card per word that shows the meaning and asks for the Korean.', type: 'choice', uses: [CARDS], options: opts(['off', 'None'], ['subdeck', 'In a Reverse subdeck'], ['same_deck', 'In the same deck']) },
      { path: 'learning.anki.audio', label: 'Add the spoken word and sentence', help: 'Uses the voice for cards (see Voice).', type: 'toggle', uses: [CARDS, VOICE] },
      { path: 'learning.anki.sync_after_add', label: 'Sync with AnkiWeb after adding', help: 'Asks Anki to sync a few seconds after the last card, so your other devices get it.', type: 'toggle', uses: [CARDS] },
      { path: 'learning.anki.auto_tags', label: 'Tag by level, type, program and month', help: 'For example topik-1, pos::verb, maru::elden-ring, added::2026-11.', type: 'toggle', uses: [CARDS] },
      { path: 'learning.anki.tags', label: 'Tags every card gets', help: 'Separated by commas.', type: 'tags', uses: [CARDS] },
      { path: 'learning.track_lookups', label: 'Count my lookups', help: 'Needed to dim the words you know and to offer words for review.', type: 'toggle', uses: [SNAP, REVIEW] },
      { path: 'learning.auto_save_after_lookups', label: 'Offer a word for review after', help: 'A word joins the review list the time you look it up this often.', type: 'number', min: 1, max: 10, step: 1, unit: 'lookups', uses: [REVIEW] },
      { path: 'learning.anki.suggest_app_decks', label: 'Suggest a deck per program', help: 'Offers decks such as Korean::Games::Elden Ring when adding; off, the program is only a tag.', type: 'toggle', uses: [CARDS], advanced: true },
      { path: 'learning.anki.correct_forms', label: "Let the model correct a word's dictionary form", help: 'For dialect and misread text: the sheet then shows both forms; in tests only Claude Sonnet used it well, and it changed what smaller models chose.', type: 'toggle', uses: [CARDS], advanced: true },
      { path: 'learning.known_stability_days', label: 'A word counts as known after', help: 'How long your memory of it must hold.', type: 'number', min: 1, max: 365, step: 1, unit: 'days', uses: [REVIEW, SNAP], advanced: true },
      { path: 'learning.desired_retention', label: 'Recall target for review', help: 'Higher means more reviews.', type: 'percent', min: 70, max: 97, step: 1, uses: [REVIEW], advanced: true },
      { path: 'learning.anki.refresh_minutes', label: 'Read your progress back from Anki every', help: '0 reads it only when Maru starts.', type: 'number', min: 0, max: 1440, step: 5, unit: 'minutes', uses: [REVIEW], advanced: true },
      { path: 'learning.anki.url', label: 'AnkiConnect address', help: 'Where the add-on listens on this PC.', type: 'text', uses: [CARDS], advanced: true },
      { path: 'learning.anki.api_key', label: 'AnkiConnect key', help: 'Only if you set one in the add-on.', type: 'password', nullable: true, uses: [CARDS], advanced: true },
      { path: 'learning.anki.note_type', label: 'Note type for words', help: 'Maru creates it in Anki and keeps it up to date.', type: 'text', uses: [CARDS], advanced: true },
      { path: 'learning.anki.sentence_note_type', label: 'Note type for sentences', help: 'Maru creates it in Anki and keeps it up to date.', type: 'text', uses: [CARDS], advanced: true },
      { path: 'learning.anki.reverse_subdeck', label: 'Name of the reverse subdeck', help: 'Used when the English → Korean card goes in a subdeck.', type: 'text', uses: [CARDS], advanced: true },
    ],
  },
  {
    id: 'voice',
    title: 'Voice',
    intro: 'Korean read aloud: while you read or play, and on the cards you make.',
    action: 'tts-test',
    fields: [
      { path: 'tts.enabled', label: 'Read Korean aloud', help: 'Turns the voice on for reading and for cards.', type: 'toggle', uses: [VOICE, CARDS] },
      { path: 'tts.provider', label: 'Where the voice runs', help: 'A voice on this PC uses its graphics card; a server does not.', type: 'choice', uses: [VOICE, CARDS], options: opts(['local', 'On this PC'], ['endpoint', 'A server', 'Another PC or a paid service']) },
      { path: 'tts.endpoint', label: 'Voice server address', help: 'For example http://192.168.1.20:8080.', type: 'text', nullable: true, placeholder: 'http://192.168.1.20:8080', uses: [VOICE, CARDS], when: SPEECH_SERVER },
      { path: 'tts.api_key', label: 'Voice server key', help: 'Only paid services need one; it is stored in config.toml on this PC.', type: 'password', nullable: true, uses: [VOICE, CARDS], when: SPEECH_SERVER },
      { path: 'tts.model', label: 'Voice model name', help: 'The name the server expects.', type: 'text', uses: [VOICE, CARDS], when: SPEECH_SERVER },
      { path: 'tts.voice', label: 'Speaker', help: 'sohee is the Korean speaker of the voice on this PC; servers have their own names.', type: 'text', uses: [VOICE, CARDS] },
      { path: 'tts.language', label: 'Language label sent to the voice', help: 'Empty lets the voice decide.', type: 'text', uses: [VOICE, CARDS], advanced: true },
      { path: 'tts.instructions', label: 'Speaking style', help: 'For example "speak clearly and a little slowly"; only some voices follow it.', type: 'text', nullable: true, uses: [VOICE, CARDS], advanced: true },
      { path: 'tts.speed', label: 'Speed', help: '1 is normal; the voice on this PC ignores it.', type: 'number', min: 0.25, max: 4, step: 0.05, uses: [VOICE, CARDS], advanced: true },
      { path: 'tasks.voice_reading.endpoint', label: 'While reading, use another voice server', help: 'With an address such as http://127.0.0.1:18097, reading goes to that server and the voice above loads only to make card sound.', type: 'text', uses: [VOICE] },
      { path: 'tasks.voice_reading.voice', label: 'Speaker while reading', help: 'Empty: the speaker above, or the other server\'s own choice (F1 is a Supertonic 3 speaker).', type: 'text', uses: [VOICE] },
      { path: 'tasks.voice_reading.model', label: 'Model name sent to the reading server', help: 'Empty: the voice model name above.', type: 'text', uses: [VOICE], advanced: true },
      { path: 'tasks.voice_cards.voice', label: 'Speaker on cards', help: 'Empty: the speaker above.', type: 'text', uses: [CARDS, VOICE] },
      { path: 'tts.cache_mb', label: 'Keep spoken sentences up to', help: 'Hearing a sentence again is then instant; 0 keeps none.', type: 'number', min: 0, max: 20000, step: 50, unit: 'MB', uses: [VOICE, CARDS], advanced: true },
      { path: 'tts.device', label: 'Run the voice on', help: 'Which processor the voice on this PC uses.', type: 'choice', uses: [VOICE, CARDS], advanced: true, when: SPEECH_LOCAL, options: opts(['auto', 'Automatic'], ['CUDA0', 'NVIDIA', 'CUDA'], ['Vulkan0', 'Any graphics card', 'Vulkan'], ['CPU', 'Processor']) },
      { path: 'tts.run_in_lite', label: 'Keep the voice in game mode', help: 'Off, so a game keeps the graphics card.', type: 'toggle', uses: [VOICE], advanced: true, when: SPEECH_LOCAL },
      { path: 'tts.port', label: 'Port of the voice on this PC', help: 'Change it only if another program uses this port.', type: 'number', min: 1024, max: 65535, step: 1, uses: [VOICE, CARDS], advanced: true, when: SPEECH_LOCAL },
    ],
  },
  {
    id: 'connections',
    title: 'Connections',
    intro: "The models Maru can ask: the one this PC runs, and servers you add (another computer on your network, or a hosted service). Each task above picks one of them by name.",
    custom: 'connections',
    fields: [
      { path: 'llm.hf_repo', label: 'Model this PC downloads', help: 'A Hugging Face repository and quantisation, fetched the first time it is used.', type: 'text', nullable: true, placeholder: 'unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL', uses: [ALL] },
      { path: 'llm.model_path', label: 'Or a model file you already have', help: 'A GGUF file on this PC; it wins over the download.', type: 'text', nullable: true, placeholder: 'D:/models/model.gguf', uses: [ALL] },
      { path: 'lite', label: 'Game mode', help: "A smaller model and no reading model on this PC, so a game keeps its video memory.", type: 'toggle', uses: [ALL] },
      { path: 'llm.server_keep_minutes', label: 'Keep a model loaded on another computer', help: 'Minutes it stays loaded after the last request; Maru unloads it when live subtitles stop and when it quits (0 leaves this to the server).', type: 'number', min: 0, max: 240, step: 5, unit: 'minutes', uses: [LIVE, SNAP, CARDS] },
      { path: 'llm.reasoning_effort', label: 'Thinking, for servers that think first', help: 'none stops a thinking model from spending its answer on reasoning; empty sends nothing.', type: 'text', placeholder: 'none', uses: [ALL], advanced: true },
      { path: 'llm.parallel', label: 'Requests this PC answers at once', help: 'Two lets a translation and its word meanings run together.', type: 'number', min: 1, max: 4, step: 1, uses: [SNAP, SCAN], advanced: true },
      { path: 'llm.ctx_size', label: 'Room per request on this PC', help: 'How much text one request may hold.', type: 'number', min: 512, max: 32768, step: 512, unit: 'tokens', uses: [ALL], advanced: true },
      { path: 'llm.gpu_layers', label: "Layers of this PC's model on the graphics card", help: 'all, auto or a number.', type: 'text', uses: [ALL], advanced: true },
      { path: 'llm.flash_attn', label: 'Flash attention', help: "A faster way to run this PC's model; turn it off only if the model fails to load.", type: 'toggle', uses: [ALL], advanced: true },
      { path: 'lite_profile.hf_repo', label: 'Game mode: model to download', help: "Empty keeps this PC's usual model.", type: 'text', uses: [ALL], advanced: true },
      { path: 'lite_profile.ctx_size', label: 'Game mode: room per request', help: 'Smaller than usual, to save memory.', type: 'number', min: 512, max: 32768, step: 512, unit: 'tokens', uses: [ALL], advanced: true },
      { path: 'lite_profile.parallel', label: 'Game mode: requests at once', help: 'One, so a translation and its meanings take turns.', type: 'number', min: 1, max: 4, step: 1, uses: [ALL], advanced: true },
    ],
  },
  {
    id: 'shortcuts',
    title: 'Shortcuts',
    intro: 'Every action can have several keys. Games often use Ctrl, Shift and Alt themselves, so add a plain key such as F7 or Pause as an alternative. A key set here is taken from every other program while Maru runs.',
    fields: [
      { path: 'hotkeys.continuous', label: 'Live subtitles on or off', help: 'Watches the area you chose last.', type: 'keys', uses: [LIVE] },
      { path: 'hotkeys.continuous_area', label: 'Choose the subtitle area', help: 'Pick a new area and start there.', type: 'keys', uses: [LIVE] },
      { path: 'hotkeys.star', label: 'Star the line on screen', help: 'Marks the live line to make cards from later; the game keeps the keyboard.', type: 'keys', uses: [LIVE, CARDS] },
      { path: 'hotkeys.translate', label: 'Translate the selection', help: 'Selected text, or the text under the pointer.', type: 'keys', uses: [SNAP] },
      { path: 'hotkeys.region', label: 'Translate a region', help: 'Drag a rectangle and translate what is inside.', type: 'keys', uses: [SNAP] },
      { path: 'hotkeys.reading_again', label: 'Read the saved area again', help: 'Reads the area you saved for this game.', type: 'keys', uses: [SNAP] },
      { path: 'hotkeys.hover', label: 'Hover on or off', help: 'Rest the pointer on a word to see it explained.', type: 'keys', uses: [HOVER] },
      { path: 'hotkeys.scan', label: 'Label the whole screen', help: 'Game menus; press again to read the screen again.', type: 'keys', uses: [SCAN] },
      { path: 'hotkeys.speak', label: 'Translate and read aloud', help: 'Reads the selection in Korean; press again to stop.', type: 'keys', uses: [VOICE, SNAP] },
      { path: 'hotkeys.history', label: 'Open History', help: 'Recent lines, and the ones you starred.', type: 'keys', uses: [CARDS, LIVE] },
      { path: 'hotkeys.dismiss', label: 'Hide the panel and the labels', help: 'Clears what Maru is showing.', type: 'keys', uses: [SNAP, SCAN, HOVER] },
      { path: 'hotkeys.launcher', label: 'Floating controls on or off', help: 'The quick controls over a borderless game.', type: 'keys', uses: [ALL] },
      { path: 'hotkeys.wheel', label: 'Open the controls wheel', help: 'Opens at the pointer and takes the keyboard, for borderless games.', type: 'keys', uses: [ALL] },
    ],
  },
];

/**
 * Subheadings inside the longer sections, so each reads as a few short lists: who answers, how the screen is read,
 * how things look. Fields keep their place in the file; the page shows them in this order, any field not named here
 * last under "Other".
 */
export const GROUPS = {
  live: [
    ['Who translates', ['tasks.live.connection', 'tasks.live.model', ...SMALL_LIVE.map((m) => m.path), 'continuous.translation.device', 'continuous.translation.backup_model', 'continuous.context_lines']],
    ['Reading the subtitles', ['reading.live.detector', 'reading.live.reader', 'continuous.linger_ms']],
    ['YouTube', ['captions.enabled', 'captions.token']],
  ],
  captures: [
    ['Who translates', ['tasks.capture.connection', 'tasks.capture.while_live', 'learning.register']],
    ['Reading the screen', ['reading.capture.reader', 'reading.capture.reader_model', 'capture.use_accessibility', 'capture.use_clipboard_fallback', 'capture.use_ocr_fallback', 'capture.ocr_radius_px']],
    ['The panel', ['learning.gloss_mode', 'ui.font_size_px', 'ui.max_width_px', 'ui.theme', 'ui.placement', 'ui.show_romanization', 'ui.show_pronunciation', 'ui.show_grammar_formula']],
    ['Floating controls', ['ui.launcher']],
  ],
  cards: [
    ['Who chooses the meanings', ['tasks.card_meanings.connection', 'tasks.card_sentence.connection']],
    ['Anki', ['learning.anki.enabled', 'learning.anki.deck', 'learning.anki.sentence_deck', 'learning.anki.new_words_per_day', 'learning.anki.new_sentences_per_day', 'learning.anki.reverse', 'learning.anki.audio', 'learning.anki.sync_after_add', 'learning.anki.auto_tags', 'learning.anki.tags']],
    ['Words you look up', ['learning.track_lookups', 'learning.auto_save_after_lookups']],
  ],
  voice: [
    ['The voice', ['tts.enabled', 'tts.provider', 'tts.endpoint', 'tts.api_key', 'tts.model', 'tts.voice']],
    ['Reading and cards apart', ['tasks.voice_reading.endpoint', 'tasks.voice_reading.voice', 'tasks.voice_cards.voice']],
  ],
};
for (const section of SECTIONS) {
  const groups = GROUPS[section.id];
  if (!groups) continue;
  const at = new Map(groups.flatMap(([title, paths], g) => paths.map((path, i) => [path, { title, order: g * 100 + i }])));
  for (const f of section.fields) if (!f.advanced) f.group = at.get(f.path)?.title ?? 'Other';
  const rank = (f) => (f.advanced ? 1e6 : at.get(f.path)?.order ?? 1e5);
  section.fields = section.fields.map((f, i) => [f, i]).sort(([a, i], [b, j]) => rank(a) - rank(b) || i - j).map(([f]) => f);
}

/** The features a section is about: its rows say "Also used by" only for the others. */
export const OWN_FEATURES = { live: [LIVE], captures: [SNAP, SCAN, HOVER], cards: [CARDS, REVIEW], voice: [VOICE] };
export const otherUses = (field) => (field.uses ?? []).filter((u) => !(OWN_FEATURES[field.section] ?? []).includes(u));

/** The page's categories, in order: the flow map, the sections, and everything marked advanced. */
export const CATEGORIES = [{ id: 'home', title: 'Overview' }, ...SECTIONS.map((s) => ({ id: s.id, title: s.title })), { id: 'advanced', title: 'Advanced' }];

export const ALL_FIELDS = SECTIONS.flatMap((s) => s.fields.map((f) => ({ ...f, section: s.id })));
const TYPED = ['text', 'password', 'number', 'percent', 'tags', 'keys'];
/** Which of the three looks a field has: `choice`, `switch` or `typed`. */
export const controlKind = (field) => (field.type === 'toggle' ? 'switch' : TYPED.includes(field.type) ? 'typed' : 'choice');

/** One condition: `{path, value}`, `{path, not}` or `{path, in}`, optionally with `and`. */
function holds(when, value) {
  if (!when) return true;
  const now = value(when.path) ?? '';
  const ok = 'value' in when ? now === when.value : 'not' in when ? now !== when.not : when.in.includes(now);
  return ok && holds(when.and, value);
}

/** Does a field show up now? Some only matter for one choice of another setting. */
export function isVisible(field, draft, current, defaults) {
  return holds(field.when, (path) => shown(draft, current, defaults, path));
}

export const labelFor = (path) => ALL_FIELDS.find((f) => f.path === path)?.label ?? path;
export const sectionOf = (path) => ALL_FIELDS.find((f) => f.path === path)?.section ?? null;

/** Search by what a setting is called, what it does, the feature that uses it, or its section. */
export function matchesSetting(field, query) {
  const q = String(query ?? '').trim().toLowerCase();
  if (!q) return true;
  const section = SECTIONS.find((s) => s.id === field.section)?.title ?? '';
  return `${field.label} ${field.help ?? ''} ${(field.uses ?? []).join(' ')} ${section} ${field.path}`.toLowerCase().includes(q);
}

// ───────────── values ─────────────

export const valueAt = (obj, path) => path.split('.').reduce((o, k) => (o == null ? undefined : o[k]), obj);

/** Settings as the page edits them: numbers rounded, percent fields as whole percents. */
export function toEditor(field, raw) {
  if (field.type === 'percent') return Math.round((raw ?? 0) * 100);
  if (field.type === 'number') return typeof raw === 'number' ? Math.round(raw * 1e6) / 1e6 : raw;
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
