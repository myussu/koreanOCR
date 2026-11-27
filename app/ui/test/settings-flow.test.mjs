import test from 'node:test';
import assert from 'node:assert/strict';
import { LOCAL, PLACES, placeOf, localModelName, connections, routeChoices, stateDot, flowSteps, readRows, usedBy, connectionProblem, saveConnection, removeConnection, pendingSummary, azureParts, needsKey } from '../settings-flow-core.js';

const servers = [
  { name: 'NAS', endpoint: 'http://192.168.18.217:11434', model_name: 'gemma4-12b', provider: 'openai', api_key: null },
  { name: 'Claude', endpoint: 'https://api.anthropic.com', model_name: 'claude-sonnet-5-5', provider: 'openai', api_key: 'k' },
  { name: 'Gemini', endpoint: 'https://generativelanguage.googleapis.com/v1beta/openai', model_name: 'gemini-flash', provider: 'gemini', api_key: 'k' },
];
const values = { llm: { hf_repo: 'unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL', model_path: null, servers }, learning: { anki: { enabled: true, deck: 'Korean::Words', sentence_deck: 'Korean::Sentences' } }, tts: { enabled: true, provider: 'local', voice: 'sohee' } };
const tasks = { 'tasks.live.connection': 'NAS', 'tasks.capture.connection': 'Gemini', 'tasks.capture.while_live': '', 'tasks.card_meanings.connection': 'local', 'tasks.card_sentence.connection': '' };
const value = (path) => tasks[path];

test('a connection is placed on this PC, another computer or a hosted service by its address, as the engine does', () => {
  assert.equal(placeOf('http://127.0.0.1:18080'), 'this_pc');
  assert.equal(placeOf('http://localhost:1234/v1'), 'this_pc');
  for (const lan of ['http://192.168.18.217:11434', 'http://10.0.0.5', 'http://172.20.1.1:8080', 'http://truenas:11434', 'http://nas.local:11434']) assert.equal(placeOf(lan), 'network', lan);
  assert.equal(placeOf('https://api.anthropic.com'), 'hosted');
  assert.equal(placeOf('http://172.200.1.1'), 'hosted');
  assert.equal(placeOf('http://192.168.1.2', 'azure'), 'hosted');
  assert.deepEqual(Object.keys(PLACES), ['this_pc', 'network', 'hosted']);
});

test("the choices of a task are this PC's model and every saved connection, each saying its model and where it runs", () => {
  const all = connections(values);
  assert.deepEqual(all.map((c) => [c.name, c.place]), [[LOCAL, 'this_pc'], ['NAS', 'network'], ['Claude', 'hosted'], ['Gemini', 'hosted']]);
  assert.equal(localModelName(values), 'gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL');
  assert.equal(localModelName({ llm: { model_path: 'D:\\models\\mine.gguf' } }), 'mine.gguf');
  const cards = routeChoices({ type: 'route', path: 'tasks.live.connection' }, values, 'NAS');
  assert.deepEqual(cards.map((c) => c.value), [LOCAL, 'NAS', 'Claude', 'Gemini']);
  assert.equal(cards[1].detail, 'gemma4-12b · another computer');
  assert.equal(cards[0].label, 'This PC');
  // No model on this PC: its card says so instead of hiding.
  const none = routeChoices({ type: 'route', path: 'tasks.live.connection' }, { llm: { servers } }, 'NAS');
  assert.ok(none[0].missing && none[0].detail.startsWith('no model set up'));
});

test('an optional connection starts with a card that means none, and a connection that is gone stays visible', () => {
  const sentence = routeChoices({ type: 'route_optional', path: 'tasks.card_sentence.connection', none: 'Keep the translation the line has' }, values, '');
  assert.deepEqual(sentence.map((c) => c.value), ['', LOCAL, 'NAS', 'Claude', 'Gemini']);
  assert.equal(sentence[0].label, 'Keep the translation the line has');
  const whileLive = routeChoices({ type: 'route_optional', path: 'tasks.capture.while_live', none: 'The same connection' }, values, '');
  assert.ok(!whileLive.some((c) => c.value === LOCAL), 'another connection while live subtitles run is never this PC');
  const gone = routeChoices({ type: 'route', path: 'tasks.live.connection' }, values, 'Removed');
  assert.deepEqual(gone.at(-1), { value: 'Removed', label: 'Removed', detail: 'This connection is no longer saved', missing: true });
});

test('a status dot is green only when it worked, red when something must be done, hollow when nothing is known', () => {
  assert.deepEqual(stateDot('working'), { kind: 'ok', label: 'working' });
  assert.deepEqual(['no_key', 'quota', 'unreachable', 'failed'].map((s) => stateDot(s)), [{ kind: 'bad', label: 'no key' }, { kind: 'bad', label: 'quota exhausted' }, { kind: 'bad', label: 'unreachable' }, { kind: 'bad', label: 'not answering properly' }]);
  for (const s of ['unknown', 'not_loaded', 'off', 'something new']) assert.equal(stateDot(s).kind, 'idle', s);
});

test('the overview walks a line through four steps and says what each uses now, where it runs and how it is doing', () => {
  const status = { ocr: true, ocr_reader: 'loads on demand', analyser: 'kiwi', dictionary: true, learner_dictionary: true, tts: 'loads on demand' };
  const statuses = [
    { task: 'live', connection: 'NAS', model: 'gemma4-12b', place: 'network', state: 'working', message: 'NAS answered.' },
    { task: 'capture', connection: 'Gemini', model: 'gemini-flash', place: 'hosted', state: 'quota', message: 'Gemini has no quota left.' },
    { task: 'card_meanings', connection: 'local', model: 'gemma-4-12B', place: 'this_pc', state: 'not_loaded', message: "This PC's model loads when it is first used." },
    { task: 'card_sentence', connection: '', model: '', place: null, state: 'off', message: 'Cards keep the translation the line already has.' },
  ];
  const steps = flowSteps(status, statuses, values);
  assert.deepEqual(steps.map((s) => s.title), ['Read the screen', 'Translate', 'Words and meanings', 'Cards']);
  const row = (step, what) => steps.find((s) => s.id === step).rows.find((r) => r.what === what);
  assert.deepEqual([row('translate', 'Live subtitles').uses, row('translate', 'Live subtitles').where, row('translate', 'Live subtitles').dot.kind], ['NAS · gemma4-12b', 'another computer', 'ok']);
  const captures = row('translate', 'Captures');
  assert.deepEqual([captures.uses, captures.where, captures.dot.label, captures.note, captures.open], ['Gemini · gemini-flash', 'hosted service', 'quota exhausted', 'Gemini has no quota left.', 'captures']);
  assert.deepEqual([row('words', 'Word splitter').uses, row('words', 'Dictionary').uses], ['Kiwi', 'Wiktionary and 한국어기초사전']);
  const meaning = row('words', 'Meaning in the line');
  assert.deepEqual([meaning.uses, meaning.where, meaning.dot.kind, meaning.note, meaning.open], ['This PC · gemma-4-12B', 'this PC', 'idle', '', 'cards']);
  assert.equal(row('cards', 'Anki decks').uses, 'Korean::Words · Korean::Sentences');
  assert.equal(row('cards', 'Sentence translation').uses, 'Cards keep the translation the line already has.');
  assert.deepEqual([row('cards', 'Voice').uses, row('cards', 'Voice').where, row('cards', 'Voice').open], ['sohee on this PC', 'this PC', 'voice']);
  assert.ok(steps.every((s) => s.rows.every((r) => r.open && r.dot)), 'every line opens a section and has a dot');
});

test('the overview survives knowing nothing yet, and says what is off or missing', () => {
  const steps = flowSteps(null, [], { learning: { anki: { enabled: false } }, tts: { enabled: false } });
  const all = steps.flatMap((s) => s.rows);
  assert.equal(all.find((r) => r.what === 'Live subtitles').uses, 'not known yet');
  assert.equal(all.find((r) => r.what === 'Anki decks').uses, 'Anki is switched off');
  assert.equal(all.find((r) => r.what === 'Voice').uses, 'off');
  assert.equal(all.find((r) => r.what === 'Word splitter').uses, 'basic rules (Kiwi is not installed)');
  assert.equal(all.find((r) => r.what === 'Text reader').dot.kind, 'bad');
});

test('a connection says which tasks use it, and cannot be removed while one does', () => {
  assert.deepEqual(usedBy('nas', value), ['Live subtitles']);
  assert.deepEqual(usedBy('local', value), ['Card meanings']);
  assert.deepEqual(usedBy('Claude', value), []);
  assert.throws(() => removeConnection(servers, 'Gemini', value), /Gemini is used by Captures\. Choose another connection there first\./);
  assert.deepEqual(removeConnection(servers, 'Claude', value)['llm.servers'].map((s) => s.name), ['NAS', 'Gemini']);
});

test('saving a connection adds it or replaces the one edited, and a rename follows into the tasks that named it', () => {
  const added = saveConnection(servers, { name: ' Azure ', endpoint: 'https://contoso.openai.azure.com', model_name: 'gpt-mini', provider: 'azure', api_key: 'k' }, null, value);
  assert.deepEqual(Object.keys(added), ['llm.servers']);
  assert.deepEqual(added['llm.servers'].at(-1), { name: 'Azure', endpoint: 'https://contoso.openai.azure.com', model_name: 'gpt-mini', provider: 'azure', api_key: 'k' });
  const renamed = saveConnection(servers, { ...servers[0], name: 'Home server', model_name: 'gemma-big' }, 'NAS', value);
  assert.deepEqual(renamed['llm.servers'].map((s) => s.name), ['Home server', 'Claude', 'Gemini'], 'in place, not appended');
  assert.equal(renamed['llm.servers'][0].model_name, 'gemma-big');
  assert.equal(renamed['tasks.live.connection'], 'Home server');
  assert.ok(!('tasks.capture.connection' in renamed), 'tasks that named another connection are untouched');
  const same = saveConnection(servers, { ...servers[1], api_key: '' }, 'Claude', value);
  assert.equal(same['llm.servers'][1].api_key, null, 'an emptied key is removed');
});

test('a connection that cannot work is refused with what to do, before anything is saved', () => {
  const ok = { name: 'NAS2', endpoint: 'http://nas:11434', model_name: 'gemma', provider: 'openai', api_key: '' };
  assert.equal(connectionProblem(servers, ok), null);
  assert.match(connectionProblem(servers, { ...ok, name: '' }), /short name/);
  assert.match(connectionProblem(servers, { ...ok, name: 'local' }), /this PC's model/);
  assert.match(connectionProblem(servers, { ...ok, name: 'https://nas' }), /That is an address/);
  assert.match(connectionProblem(servers, { ...ok, name: 'nas' }), /already a connection called nas/);
  assert.equal(connectionProblem(servers, { ...ok, name: 'NAS' }, 'NAS'), null, 'keeping its own name while editing');
  assert.match(connectionProblem(servers, { ...ok, endpoint: '192.168.1.20' }), /Find the server/);
  assert.match(connectionProblem(servers, { ...ok, model_name: ' ' }), /model/);
  assert.match(connectionProblem(servers, { ...ok, provider: 'gemini' }), /Gemini refuses requests without one/);
  assert.match(connectionProblem(servers, { ...ok, provider: 'azure', api_key: 'k', endpoint: 'https://contoso.openai.azure.com/openai/deployments/gpt-4.1-mini/chat/completions' }), /https:\/\/contoso\.openai\.azure\.com as the address and gpt-4\.1-mini as the model/);
  assert.throws(() => saveConnection(servers, { ...ok, name: '' }, null, value), /short name/);
  assert.deepEqual(azureParts('https://contoso.openai.azure.com/openai/deployments/my%20dep/x'), { endpoint: 'https://contoso.openai.azure.com', modelName: 'my dep' });
  assert.ok(needsKey('azure') && needsKey('gemini') && needsKey('anthropic') && !needsKey('openai'));
  assert.match(connectionProblem(servers, { ...ok, provider: 'anthropic', endpoint: 'https://api.anthropic.com' }), /Anthropic refuses requests without one/);
});

test('the restart banner names what is waiting once each, and only a few of them', () => {
  const label = (p) => ({ 'a.x': 'Alpha', 'a.y': 'Alpha' }[p] ?? p);
  assert.equal(pendingSummary(['a.x', 'a.y', 'b'], label), 'Alpha, b');
  assert.equal(pendingSummary(['1', '2', '3', '4', '5', '6', '7', '8'], label), '1, 2, 3, 4, 5, 6 and 2 more');
  assert.equal(pendingSummary([], label), '');
});

test('the read step says per task what reads and where it runs, and that the reading model loads with the first capture', () => {
  const reading = [
    { task: 'live', what: 'Live subtitles', uses: 'Line finder: Small and quick · letters: Standard', reader: null, place: 'This PC · processor' },
    { task: 'capture', what: 'Captures', uses: 'Line finder: Standard · letters: Standard', reader: 'PaddleOCR-VL 1.6', place: 'This PC · processor and graphics card' },
    { task: 'scan', what: 'Screen scan', uses: 'Line finder: Standard · letters: Standard', reader: null, place: 'This PC · processor' },
  ];
  const rows = readRows({ ocr: true, ocr_reader: 'loads on demand', reading });
  assert.deepEqual(rows.map((r) => r.what), ['Live subtitles', 'Captures', 'Screen scan']);
  assert.deepEqual([rows[0].uses, rows[0].where, rows[0].dot.kind, rows[0].open], ['Line finder: Small and quick · letters: Standard · no reading model', 'This PC · processor', 'ok', 'live']);
  assert.deepEqual([rows[1].uses, rows[1].dot.kind, rows[1].open], ['Line finder: Standard · letters: Standard · checked by PaddleOCR-VL 1.6, which loads with the first capture', 'idle', 'captures']);
  assert.equal(rows[1].note, undefined, 'waiting to load is not shown as a problem');
  const loaded = readRows({ ocr: true, ocr_reader: 'ready', reading })[1];
  assert.deepEqual([loaded.dot.kind, loaded.uses], ['ok', 'Line finder: Standard · letters: Standard · checked by PaddleOCR-VL 1.6']);
  for (const r of rows) assert.ok(!/pp-ocr|onnx|cpu_ocr|vision_reader/i.test(`${r.what} ${r.uses} ${r.where}`), `no internal names: ${r.uses}`);
  assert.deepEqual(readRows({ ocr: false, reading }).map((r) => r.uses), ['not installed'], 'no text reader at all');
  assert.deepEqual(readRows({ ocr: true, ocr_reader: 'off' }).map((r) => r.what), ['Text reader', 'Reading model'], 'an older shell without the field');
});

test('the Test under a connection field checks the picked card, saved or not, without saving or restarting', async () => {
  const { routeTestPlan, needsRestart } = await import('../settings-flow-core.js');
  const servers = [{ name: 'NAS', endpoint: 'http://192.168.1.20:11434', model_name: 'gemma4-12b', provider: 'openai', api_key: null }];
  const page = { 'llm.servers': servers, 'tasks.live.model': '' };
  const value = (path) => page[path];
  const live = { path: 'tasks.live.connection' };
  assert.deepEqual(routeTestPlan(live, 'local', value), { kind: 'local', label: 'This PC' });
  assert.deepEqual(routeTestPlan(live, 'nas', value), { kind: 'server', label: 'NAS', server: servers[0] });
  assert.equal(routeTestPlan(live, 'Claude', value).kind, 'none');
  assert.equal(routeTestPlan({ path: 'tasks.card_sentence.connection', none: 'Keep the translation the line has' }, '', value).message, 'Keep the translation the line has: nothing to test.');
  page['tasks.live.model'] = 'lfm';
  assert.equal(routeTestPlan(live, 'local', value).kind, 'later', 'a translation-only model loads only with live subtitles');
  assert.equal(routeTestPlan({ path: 'tasks.capture.connection' }, 'local', value).kind, 'local', 'only live subtitles have translation-only models');

  const appliesLive = (p) => p.startsWith('tasks.card_') || p === 'llm.servers';
  assert.equal(needsRestart({ 'tasks.card_meanings.connection': 'NAS' }, appliesLive), false);
  assert.equal(needsRestart({ 'tasks.capture.connection': 'NAS', 'llm.servers': [] }, appliesLive), true);
  assert.equal(needsRestart({}, appliesLive), false);
});
