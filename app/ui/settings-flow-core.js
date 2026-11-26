// The settings page's decisions that are not about one field: where a connection runs, the cards a task can
// choose from, the flow map on the overview, the status dot, and editing the list of connections. Pure functions.

export const LOCAL = 'local';

/** Where a connection runs, as the page says it. Keys match `maru_engine::routes::Place`. */
export const PLACES = { this_pc: 'this PC', network: 'another computer', hosted: 'hosted service' };

/** Mirrors `routes::place_of`: where the server at `endpoint` is, from its address alone. */
export function placeOf(endpoint, provider = 'openai') {
  if (provider && provider !== 'openai') return 'hosted';
  const rest = String(endpoint ?? '').split('://')[1] ?? String(endpoint ?? '');
  const host = rest.split(/[/:]/)[0].toLowerCase();
  const parts = host.split('.');
  const ip = parts.length === 4 && parts.every((p) => /^\d{1,3}$/.test(p)) ? parts.map(Number) : null;
  if (host === 'localhost' || (ip && ip[0] === 127)) return 'this_pc';
  if (ip) return ip[0] === 10 || (ip[0] === 172 && ip[1] >= 16 && ip[1] < 32) || (ip[0] === 192 && ip[1] === 168) || (ip[0] === 169 && ip[1] === 254) ? 'network' : 'hosted';
  return !host.includes('.') || /\.(local|lan|home)$/.test(host) ? 'network' : 'hosted';
}

/** The name of this PC's model, short enough for a card. */
export function localModelName(values) {
  const file = values?.llm?.model_path;
  if (file) return String(file).split(/[\\/]/).pop();
  const repo = values?.llm?.hf_repo;
  return repo ? String(repo).split('/').pop() : null;
}

/** Everything a task can be given: this PC's model first, then the saved connections. */
export function connections(values) {
  const model = localModelName(values);
  const local = { name: LOCAL, title: 'This PC', model: model ?? 'no model set up', place: 'this_pc', ready: !!model };
  const saved = (values?.llm?.servers ?? []).map((s) => ({ name: s.name, title: s.name, model: s.model_name, place: placeOf(s.endpoint, s.provider), ready: true }));
  return [local, ...saved];
}

/**
 * The cards of a `route` field (every connection) or a `route_optional` one (the saved connections, after a
 * card that means "none"). A value naming a connection that no longer exists gets its own card, so it is seen.
 */
export function routeChoices(field, values, current) {
  const all = connections(values);
  const card = (c) => ({ value: c.name, label: c.title, detail: `${c.model} · ${PLACES[c.place]}`, place: c.place, missing: !c.ready });
  const choices = field.type === 'route_optional' ? [{ value: '', label: field.none, detail: '' }, ...all.filter((c) => c.name !== LOCAL || field.path !== 'tasks.capture.while_live').map(card)] : all.map(card);
  const known = choices.some((c) => String(c.value).toLowerCase() === String(current ?? '').toLowerCase());
  if (!known && current) choices.push({ value: current, label: current, detail: 'This connection is no longer saved', missing: true });
  return choices;
}

/** The colour and words of a status dot. States are `maru_engine::routes::State`. */
export function stateDot(state) {
  const table = {
    working: ['ok', 'working'],
    no_key: ['bad', 'no key'],
    quota: ['bad', 'quota exhausted'],
    unreachable: ['bad', 'unreachable'],
    failed: ['bad', 'not answering properly'],
    unknown: ['idle', 'not checked yet'],
    not_loaded: ['idle', 'loads when used'],
    off: ['idle', 'off'],
  };
  const [kind, label] = table[state] ?? table.unknown;
  return { kind, label };
}

const taskOf = (statuses, task) => (statuses ?? []).find((s) => s.task === task);

/** One line of a flow step: what it is, what it uses now, where that runs, and its dot. */
function taskRow(what, status, open) {
  if (!status) return { what, uses: 'not known yet', where: '', dot: stateDot('unknown'), open };
  if (status.state === 'off') return { what, uses: status.message, where: '', dot: stateDot('off'), open };
  const uses = status.connection === LOCAL ? `This PC · ${status.model}` : `${status.connection} · ${status.model}`;
  return { what, uses, where: PLACES[status.place] ?? '', dot: stateDot(status.state), note: status.state === 'working' || status.state === 'unknown' || status.state === 'not_loaded' ? '' : status.message, open };
}

/** A part of Maru as a state: installed or not (`true`/`false`), switched off (`null`), or a model's status word. */
function partState(s) {
  if (s === true || s === 'ready') return 'working';
  if (s === false || String(s).startsWith('failed')) return 'failed';
  if (s == null || s === 'off' || s === 'not configured') return 'off';
  return 'not_loaded'; // "loads on demand", "loading"
}
const part = (what, state, uses, open, where = PLACES.this_pc) => ({ what, uses, where: partState(state) === 'off' ? '' : where, dot: stateDot(partState(state)), open });

/**
 * The overview: the four steps a line goes through, each with what it uses now. `status` is the `status`
 * command's answer, `tasks` the `task_status` one, `values` the settings.
 */
export function flowSteps(status, tasks, values) {
  const s = status ?? {};
  const anki = values?.learning?.anki ?? {};
  const decks = anki.sentence_deck && anki.sentence_deck !== anki.deck ? `${anki.deck} · ${anki.sentence_deck}` : anki.deck;
  const voice = values?.tts?.enabled ? (values.tts.provider === 'endpoint' ? ['a voice server', PLACES.network] : [`${values.tts.voice || 'the voice'} on this PC`, PLACES.this_pc]) : null;
  return [
    {
      id: 'read',
      title: 'Read the screen',
      about: 'Selected text is read exactly; pixels are read by the text reader.',
      rows: [
        part('Text reader', !!s.ocr, s.ocr ? 'PP-OCR' : 'not installed', 'captures'),
        part('Reading model', s.ocr_reader ?? null, partState(s.ocr_reader ?? null) === 'off' ? 'not in use' : 'PaddleOCR-VL', 'captures'),
      ],
    },
    {
      id: 'translate',
      title: 'Translate',
      about: 'Live lines and captures each ask the connection you chose for them.',
      rows: [taskRow('Live subtitles', taskOf(tasks, 'live'), 'live'), taskRow('Captures', taskOf(tasks, 'capture'), 'captures')],
    },
    {
      id: 'words',
      title: 'Words and meanings',
      about: 'Kiwi splits a line into words; the dictionary lists what each can mean; a model picks the meaning that fits.',
      rows: [
        part('Word splitter', s.analyser === 'kiwi', s.analyser === 'kiwi' ? 'Kiwi' : 'basic rules (Kiwi is not installed)', 'advanced'),
        part('Dictionary', !!s.dictionary, s.dictionary ? (s.learner_dictionary ? 'Wiktionary and 한국어기초사전' : 'Wiktionary') : 'not installed', 'advanced'),
        taskRow('Meaning in the line', taskOf(tasks, 'card_meanings'), 'cards'),
      ],
    },
    {
      id: 'cards',
      title: 'Cards',
      about: 'The words you pick and the sentence become Anki cards, with sound.',
      rows: [
        part('Anki decks', anki.enabled ? true : null, anki.enabled ? decks : 'Anki is switched off', 'cards', 'Anki on this PC'),
        taskRow('Sentence translation', taskOf(tasks, 'card_sentence'), 'cards'),
        part('Voice', voice ? (s.tts && s.tts !== 'off' ? s.tts : 'loads on demand') : null, voice ? voice[0] : 'off', 'voice', voice ? voice[1] : ''),
      ],
    },
  ];
}

// ───────────── the list of connections ─────────────

const TASK_PATHS = [
  ['Live subtitles', 'tasks.live.connection'],
  ['Captures', 'tasks.capture.connection'],
  ['Captures while live subtitles are on', 'tasks.capture.while_live'],
  ['Card meanings', 'tasks.card_meanings.connection'],
  ['Card sentence translation', 'tasks.card_sentence.connection'],
];
const sameName = (a, b) => String(a ?? '').trim().toLowerCase() === String(b ?? '').trim().toLowerCase();

/** The tasks that use the connection called `name`; `value(path)` reads a setting as the page shows it. */
export function usedBy(name, value) {
  return TASK_PATHS.filter(([, path]) => sameName(value(path), name)).map(([label]) => label);
}

/** Server types that refuse requests without an API key. */
export const needsKey = (provider) => provider === 'azure' || provider === 'gemini';

/** A pasted Azure deployment URL, split into the resource address and the deployment name. */
export function azureParts(endpoint) {
  const match = /^(https:\/\/[^/]+)\/openai\/deployments\/([^/?#]+)/.exec(String(endpoint ?? '').trim());
  return match ? { endpoint: match[1], modelName: decodeURIComponent(match[2]) } : null;
}

/** What is wrong with a connection as typed, in words that say what to do; `null` when it can be saved. */
export function connectionProblem(servers, edited, original = null) {
  const name = String(edited.name ?? '').trim();
  if (!name) return 'Give this connection a short name, such as NAS or Claude.';
  if (sameName(name, LOCAL)) return '"local" is the name of this PC\'s model. Choose another name.';
  if (/^https?:\/\/|\.(com|net|org|io|ai)\b/i.test(name)) return 'That is an address. Put it in the address field and give the connection a short name.';
  if ((servers ?? []).some((s) => sameName(s.name, name) && !sameName(s.name, original))) return `There is already a connection called ${name}.`;
  const azure = edited.provider === 'azure' && azureParts(edited.endpoint);
  if (azure) return `For Azure, use ${azure.endpoint} as the address and ${azure.modelName} as the model.`;
  if (!/^https?:\/\//.test(String(edited.endpoint ?? '').trim())) return 'Enter the full address, starting with http:// or https://. For a bare IP or name, press Find the server.';
  if (!String(edited.model_name ?? '').trim()) return 'Enter the model the server should use.';
  if (needsKey(edited.provider) && !edited.api_key) return `Enter the API key: ${edited.provider === 'azure' ? 'Azure' : 'Gemini'} refuses requests without one.`;
  return null;
}

/**
 * The edits for saving a connection (new, or replacing `original`): the new list and, when it was renamed, the
 * tasks that named it. Throws with a sentence for the person when it cannot be saved.
 */
export function saveConnection(servers, edited, original, value) {
  const problem = connectionProblem(servers, edited, original);
  if (problem) throw Error(problem);
  const next = { name: edited.name.trim(), endpoint: edited.endpoint.trim(), model_name: edited.model_name.trim(), provider: edited.provider ?? 'openai', api_key: edited.api_key || null };
  const list = [...(servers ?? [])];
  const at = original == null ? -1 : list.findIndex((s) => sameName(s.name, original));
  if (at < 0) list.push(next);
  else list[at] = next;
  const edits = { 'llm.servers': list };
  if (original != null && !sameName(original, next.name)) for (const [, path] of TASK_PATHS) if (sameName(value(path), original)) edits[path] = next.name;
  return edits;
}

/** The list without `name`. Throws, naming the tasks, while a task still uses it. */
export function removeConnection(servers, name, value) {
  const users = usedBy(name, value);
  if (users.length) throw Error(`${name} is used by ${users.join(' and ')}. Choose another connection there first.`);
  return { 'llm.servers': (servers ?? []).filter((s) => !sameName(s.name, name)) };
}

/** The restart banner's sentence: the names of what is waiting, a few of them. */
export function pendingSummary(paths, labelFor, limit = 6) {
  const names = [...new Set((paths ?? []).map(labelFor))];
  return names.length > limit ? `${names.slice(0, limit).join(', ')} and ${names.length - limit} more` : names.join(', ');
}
