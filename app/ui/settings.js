// The Settings tab of the hub window. The rules live in settings-core.js and settings-flow-core.js; this file draws them.
//
// The overview is a map of what a line goes through (read the screen → translate → words and meanings → cards),
// each step showing what it uses now, where that runs and whether it works. The sections follow the same flow.
// Edits are collected as a draft (only what differs from the file) and written with one Save. Typing never
// re-renders the page (that would drop the cursor): each field updates itself.

import { invoke } from './bridge.js';
import { ankiSettingsPending, soundsText } from './anki.js';
import { modelEdits } from './settings-live-core.js';
import { discoveryControls } from './settings-discovery.js';
import { SECTIONS, CATEGORIES, ALL_FIELDS, controlKind, isVisible, appliesLive, labelFor, matchesSetting, toEditor, fromEditor, withEdit, withReset, shown, isDefault, splitKeys, joinKeys, comboFromEvent, keyRisk } from './settings-core.js';
import { LOCAL, PLACES, connections, routeChoices, stateDot, flowSteps, usedBy, connectionProblem, saveConnection, removeConnection, pendingSummary, needsKey } from './settings-flow-core.js';

function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'text') el.textContent = v;
    else if (k === 'value') el.value = v;
    else if (k === 'checked') el.checked = !!v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) if (kid != null && kid !== false) el.append(kid);
  return el;
}

const ensureStyles = () => {
  if (document.getElementById('settings-style')) return;
  document.head.append(h('link', { id: 'settings-style', rel: 'stylesheet', href: 'settings.css' }));
};

const btn = (text, onclick, cls = '') => h('button', { class: `btn ${cls}`, type: 'button', onclick, text });
const dot = (state) => h('span', { class: `dot ${state.kind}`, title: state.label, role: 'img', 'aria-label': state.label });
const TASK_OF_SECTION = { live: 'Live subtitles', capture: 'Captures', card_meanings: 'Word meanings', card_sentence: 'Sentence translation' };

export async function settingsView() {
  ensureStyles();
  const root = h('div', { class: 'settings-page' });
  let data = await invoke('get_settings');
  let status = await invoke('status').catch(() => null);
  let tasks = await invoke('task_status').catch(() => []);
  let draft = {};
  let message = null; // { kind: 'ok' | 'bad', text }
  let capturing = null; // { path, stop }
  let editing = null; // the connection being edited: { original, fields, check, discovery }
  const updaters = [];
  let disposed = false;
  root.dispose = () => { disposed = true; stopCapture(); editing?.discovery?.stop?.(); };
  // A link can open one section, a search, or a connection's editor: hub.html?settings=connections&connection=NAS#settings
  const asked = new URLSearchParams(location.search);
  const start = asked.get('settings');
  let category = CATEGORIES.some((c) => c.id === start) ? start : 'home', query = asked.get('q') ?? '', saving = false;
  const wanted = asked.get('connection');
  if (wanted) {
    const saved = (data.values.llm?.servers ?? []).find((x) => x.name === wanted);
    editing = saved ? { original: saved.name, fields: { ...saved, api_key: saved.api_key ?? '' }, problem: '' } : { original: null, fields: { name: '', endpoint: '', model_name: '', provider: 'openai', api_key: '' }, problem: '' };
    category = 'connections';
  }
  const sidebar = h('nav', { class: 'settings-sidebar', 'aria-label': 'Settings sections' });
  const content = h('div', { class: 'settings-content' });
  const search = h('input', { class: 'settings-search', type: 'search', value: asked.get('q') ?? '', placeholder: 'Search by setting or feature…', 'aria-label': 'Search settings', oninput: (e) => { query = e.target.value; rebuildAll(); } });
  root.canLeave = () => {
    if (!Object.keys(draft).length && !saving) { stopCapture(); return true; }
    message = { kind: 'bad', text: 'Save or discard your changes before leaving Settings.' }; refreshAll(); return false;
  };

  const refreshAll = () => updaters.forEach((u) => u());
  const value = (path) => shown(draft, data.values, data.defaults, path);
  const live = () => ({ ...data.values, llm: { ...data.values.llm, servers: value('llm.servers') ?? [] } });
  const stage = (edits) => { for (const [path, v] of Object.entries(edits)) draft = withEdit(draft, data.values, path, v); message = null; };
  const go = (id) => { category = id; query = ''; search.value = ''; editing = null; rebuildAll(); root.scrollIntoView?.({ block: 'start' }); };

  // ───────────── the three kinds of control ─────────────

  /** Cards to pick one of: a visible list of every possibility, the chosen one outlined. */
  function choiceControl(field) {
    const box = h('div', { class: 'choices', role: 'radiogroup', 'aria-label': field.label });
    const options = () => (field.type === 'route' || field.type === 'route_optional' ? routeChoices(field, live(), value(field.path)) : field.options);
    const pick = (v) => {
      // Choosing a translation-only model also fills in the file Maru found for it.
      if (field.path === 'tasks.live.model') stage(modelEdits(v === '' ? 'main' : v, Object.fromEntries(ALL_FIELDS.map((f) => [f.path, value(f.path)])), data.live_models));
      else stage({ [field.path]: v });
      rebuildAll();
    };
    const draw = () => {
      const now = value(field.path) ?? '';
      box.replaceChildren(...options().map((o) => h('button', { class: `choice${o.missing ? ' missing' : ''}`, type: 'button', role: 'radio', 'aria-checked': String(String(o.value ?? '').toLowerCase() === String(now).toLowerCase()), onclick: () => pick(o.value) },
        h('strong', { text: o.label }), o.detail && h('small', { text: o.detail }))));
    };
    draw();
    return { el: box, sync: draw, rebuild: draw };
  }

  /** A switch: on or off. */
  function switchControl(field) {
    const input = h('input', { type: 'checkbox', role: 'switch', checked: value(field.path), 'aria-label': field.label, onchange: (e) => { stage({ [field.path]: e.target.checked }); refreshAll(); } });
    const word = h('span', { class: 'sw-word' });
    const set = () => { input.checked = !!value(field.path); word.textContent = input.checked ? 'On' : 'Off'; };
    set();
    return { el: h('label', { class: 'sw-wrap' }, h('span', { class: 'sw' }, input, h('i')), word), sync: set, rebuild: set };
  }

  /** A box to type in: text, a number with its unit, a secret, a list of tags. */
  function typedControl(field) {
    const edit = (v) => { stage({ [field.path]: v }); refreshAll(); };
    const numeric = field.type === 'number' || field.type === 'percent';
    const input = numeric
      ? h('input', { type: 'number', min: field.min, max: field.max, step: field.step, 'aria-label': field.label, oninput: (e) => { const n = e.target.valueAsNumber; if (!Number.isNaN(n)) edit(fromEditor(field, n)); } })
      : h('input', { type: field.type === 'password' ? 'password' : 'text', autocomplete: 'off', placeholder: field.placeholder ?? '', spellcheck: 'false', 'aria-label': field.label, oninput: (e) => edit(fromEditor(field, e.target.value)) });
    const set = () => (input.value = toEditor(field, value(field.path)));
    set();
    const unit = field.type === 'percent' ? '%' : field.unit;
    return { el: h('div', { class: `typed${numeric ? ' number' : ''}` }, input, unit && h('span', { class: 'unit', text: unit })), sync: () => {}, rebuild: set };
  }

  function keysControl(field) {
    const box = h('div', { class: 'keys' });
    const note = h('div', { class: 'keys-note' });
    const list = () => splitKeys(value(field.path));
    const save = (next) => { stage({ [field.path]: joinKeys(next) }); refreshAll(); };
    const draw = () => {
      const keys = list();
      const risks = keys.map(keyRisk);
      const listening = capturing?.path === field.path;
      box.replaceChildren(h('div', { class: 'list' },
        ...keys.map((k, i) => h('span', { class: `k${risks[i] ? ' warn' : ''}` }, k, h('button', { title: 'Remove', type: 'button', text: '×', onclick: () => save(keys.filter((_, j) => j !== i)) }))),
        h('button', { class: `btn add${listening ? ' listening' : ''}`, type: 'button', text: listening ? 'Press the keys…  (Esc to cancel)' : keys.length ? '+ Add another key' : '+ Set key', onclick: () => (listening ? stopCapture() : startCapture(field, keys, save)) })));
      note.textContent = listening && capturing.problem ? capturing.problem : risks.find(Boolean) ?? '';
    };
    return { el: h('div', { class: 'keys-wrap' }, box, note), sync: draw, rebuild: draw };
  }

  function stopCapture() {
    capturing?.stop();
    capturing = null;
    refreshAll();
  }

  function startCapture(field, keys, save) {
    capturing?.stop();
    const onKey = (e) => {
      e.preventDefault();
      e.stopPropagation();
      const combo = comboFromEvent(e);
      if (combo === null) return;
      if (combo === 'cancel') return stopCapture();
      if (combo === 'unknown') { capturing.problem = 'Maru has no name for that key. Try another.'; return refreshAll(); }
      if (!keys.includes(combo)) save([...keys, combo]);
      stopCapture();
    };
    addEventListener('keydown', onKey, true);
    capturing = { path: field.path, stop: () => removeEventListener('keydown', onKey, true), problem: '' };
    refreshAll();
  }

  // ───────────── a setting: its name, one sentence, who uses it, its control ─────────────

  const rows = new Map();
  function row(field) {
    const kind = controlKind(field);
    const ctl = field.type === 'keys' ? keysControl(field) : kind === 'switch' ? switchControl(field) : kind === 'choice' ? choiceControl(field) : typedControl(field);
    const reset = h('button', { class: 'reset', type: 'button', title: 'Back to the default', text: '↺', onclick: () => { draft = withReset(draft, data.values, data.defaults, field.path); message = null; ctl.rebuild(); refreshAll(); } });
    const uses = field.uses?.length && h('div', { class: 'uses' }, 'Used by ', field.uses.map((u) => h('span', { class: 'use', text: u })));
    const holder = h('div', { class: `set-row kind-${kind}${field.type === 'keys' ? ' keysrow' : ''}`, 'data-path': field.path },
      h('div', { class: 'what' }, h('div', { class: 'name' }, field.label, reset), field.help && h('div', { class: 'help', text: field.help }), uses),
      ctl.el);
    updaters.push(() => {
      holder.classList.toggle('changed', field.path in draft);
      holder.hidden = !isVisible(field, draft, data.values, data.defaults);
      reset.classList.toggle('on', !isDefault(draft, data.values, data.defaults, field.path));
      ctl.sync();
    });
    rows.set(field.path, { rebuild: ctl.rebuild });
    return holder;
  }

  // ───────────── what a task uses now ─────────────

  /** "In use now": the connection the running app asks for this task, its dot, and a Test button. */
  function taskLine(task) {
    const box = h('div', { class: 'task-now' });
    let busy = false;
    const draw = () => {
      const s = tasks.find((t) => t.task === task);
      if (!s) return box.replaceChildren();
      const state = stateDot(busy ? 'unknown' : s.state);
      const who = s.state === 'off' ? s.message : `${s.connection === LOCAL ? 'This PC' : s.connection} · ${s.model}`;
      const then = s.fallbacks?.length ? ` If it cannot answer: ${s.fallbacks.join(', then ')}.` : '';
      const changed = Object.keys(draft).some((p) => p.startsWith(`tasks.${task}.`));
      box.replaceChildren(...[
        h('div', { class: 'task-who' }, h('span', { class: 'task-label', text: `${TASK_OF_SECTION[task]} now` }), dot(state), h('b', { text: who }), s.place && h('span', { class: 'place', text: PLACES[s.place] }), h('span', { class: `task-state ${state.kind}`, text: busy ? 'testing…' : state.label })),
        h('div', { class: 'task-msg', text: `${busy || s.state === 'off' ? '' : s.message}${then}${changed ? ' Your change here takes effect after Save' + (appliesLive(`tasks.${task}.connection`) ? '.' : ' and a restart.') : ''}` }),
        s.state !== 'off' && btn(busy ? 'Testing…' : 'Test', async () => {
          if (busy) return;
          busy = true; draw();
          try {
            const result = await invoke('task_test', { task });
            tasks = tasks.map((t) => (t.task === task ? result : t));
          } catch (e) { message = { kind: 'bad', text: String(e) }; }
          busy = false; refreshAll();
        })].filter(Boolean));
    };
    draw();
    updaters.push(draw);
    return box;
  }

  // ───────────── the overview: a map of the flow ─────────────

  function homeView() {
    const steps = flowSteps(status, tasks, data.values);
    return h('section', { class: 'flow' },
      h('p', { class: 'intro', text: 'What a line goes through, and what each step uses right now. Click a line to change it.' }),
      h('ol', { class: 'flow-steps' }, steps.map((step, n) => h('li', { class: 'flow-step' },
        h('div', { class: 'flow-head' }, h('span', { class: 'flow-n', text: String(n + 1) }), h('h3', { text: step.title })),
        h('p', { class: 'flow-about', text: step.about }),
        step.rows.map((r) => h('button', { class: 'flow-row', type: 'button', onclick: () => go(r.open) },
          dot(r.dot), h('span', { class: 'flow-what', text: r.what }), h('span', { class: 'flow-uses', text: r.uses }), r.where && h('span', { class: 'place', text: r.where }),
          r.note && h('span', { class: 'flow-note', text: r.note })))))),
      h('p', { class: 'flow-legend' }, dot(stateDot('working')), ' working ', dot(stateDot('quota')), ' needs attention ', dot(stateDot('unknown')), ' not checked yet, or off'));
  }

  // ───────────── connections: the one place they are edited ─────────────

  function connectionsView() {
    const servers = () => value('llm.servers') ?? [];
    const list = h('div', { class: 'conn-list' });
    const draw = () => {
      const cards = connections(live()).map((c) => {
        const saved = servers().find((s) => s.name === c.name);
        const users = usedBy(c.name, value);
        const check = checks.get(c.name);
        return h('div', { class: 'conn' },
          h('div', { class: 'conn-head' }, h('b', { text: c.title }), h('span', { class: 'place', text: PLACES[c.place] }), check && dot(stateDot(check.state)), check && h('span', { class: `task-state ${stateDot(check.state).kind}`, text: check.message })),
          h('div', { class: 'conn-detail', text: saved ? `${c.model} · ${saved.endpoint}${needsKey(saved.provider) || saved.api_key ? saved.api_key ? ' · key saved' : ' · no key' : ''}` : `${c.model}${c.ready ? '' : ' — set one below'}` }),
          h('div', { class: 'uses' }, users.length ? ['Used by ', users.map((u) => h('span', { class: 'use', text: u }))] : 'Not used by any task'),
          saved && h('div', { class: 'conn-actions' },
            btn('Edit', () => { editing = { original: saved.name, fields: { ...saved, api_key: saved.api_key ?? '' }, problem: '' }; rebuildAll(); }),
            btn(check?.busy ? 'Testing…' : 'Test', () => test(saved)),
            btn('Remove', () => { try { stage(removeConnection(servers(), saved.name, value)); rebuildAll(); } catch (e) { message = { kind: 'bad', text: e.message }; refreshAll(); } })));
      });
      list.replaceChildren(...cards);
    };
    async function test(server) {
      if (checks.get(server.name)?.busy) return;
      checks.set(server.name, { busy: true, state: 'unknown', message: 'testing…' });
      draw();
      const r = await invoke('connection_check', { name: server.name, endpoint: server.endpoint, modelName: server.model_name, reasoningEffort: value('llm.reasoning_effort'), provider: server.provider ?? 'openai', apiKey: server.api_key || null }).catch((e) => ({ state: 'failed', message: String(e) }));
      checks.set(server.name, { busy: false, state: r.state === 'input' ? 'failed' : r.state, message: r.message });
      draw();
    }
    draw();
    updaters.push(draw);
    return h('section', { class: 'set-sec' }, list, editing ? connectionEditor() : btn('+ Add a connection', () => { editing = { original: null, fields: { name: '', endpoint: '', model_name: '', provider: 'openai', api_key: '' }, problem: '' }; rebuildAll(); }, 'primary'));
  }
  const checks = new Map();

  function connectionEditor() {
    const f = editing.fields;
    const problem = h('div', { class: 'res bad', role: 'alert' });
    const result = h('div', { class: 'res', role: 'status', 'aria-live': 'polite' });
    const input = (key, label, help, props = {}) => {
      const el = h('input', { type: props.type ?? 'text', value: f[key] ?? '', placeholder: props.placeholder ?? '', autocomplete: 'off', spellcheck: 'false', 'aria-label': label, oninput: (e) => { f[key] = e.target.value; problem.textContent = ''; result.textContent = ''; discovery.sync(); } });
      inputs[key] = el;
      return h('label', { class: 'conn-field' }, h('span', { class: 'name', text: label }), h('span', { class: 'help', text: help }), h('div', { class: 'typed' }, el));
    };
    const inputs = {};
    const types = [['openai', 'A server of yours', 'Ollama, LM Studio, llama.cpp, a NAS, any OpenAI-style address'], ['anthropic', 'Anthropic (Claude)', 'Its own API at https://api.anthropic.com, with the prompt cache'], ['azure', 'Azure OpenAI', 'The address is your resource; the model is your deployment'], ['gemini', 'Google Gemini', 'Needs a key from Google AI Studio']];
    const typeBox = h('div', { class: 'choices', role: 'radiogroup', 'aria-label': 'Kind of server' });
    const drawTypes = () => typeBox.replaceChildren(...types.map(([v, label, detail]) => h('button', { class: 'choice', type: 'button', role: 'radio', 'aria-checked': String((f.provider ?? 'openai') === v), onclick: () => {
      f.provider = v;
      if (v === 'gemini' && !f.endpoint) { f.endpoint = 'https://generativelanguage.googleapis.com/v1beta/openai'; inputs.endpoint.value = f.endpoint; }
      if (v === 'anthropic' && !f.endpoint) { f.endpoint = 'https://api.anthropic.com'; inputs.endpoint.value = f.endpoint; }
      drawTypes();
    } }, h('strong', { text: label }), h('small', { text: detail }))));
    drawTypes();
    // Finding a server by a bare address reuses the discovery controls; they edit this form, not the file.
    const state = editing.discovery ?? (editing.discovery = {});
    const discovery = discoveryControls({
      section: 'model', state,
      fields: () => ({ endpoint: String(f.endpoint ?? '').trim(), modelName: String(f.model_name ?? '').trim() }),
      edit: (path, v) => { const key = path === 'llm.endpoint' ? 'endpoint' : 'model_name'; f[key] = v; inputs[key].value = v; },
      refresh: () => discovery.sync(), disposed: () => disposed,
    });
    discovery.sync();
    const done = () => {
      try {
        stage(saveConnection(value('llm.servers') ?? [], f, editing.original, value));
        editing = null; rebuildAll();
      } catch (e) { problem.textContent = e.message; }
    };
    const test = btn('Test', async () => {
      const wrong = connectionProblem([], { ...f, name: f.name || 'server' }, null);
      if (wrong) return void (problem.textContent = wrong);
      test.disabled = true; result.className = 'res'; result.textContent = 'Asking the server (up to a minute while it loads its model)…';
      const r = await invoke('connection_check', { name: f.name, endpoint: f.endpoint, modelName: f.model_name, reasoningEffort: value('llm.reasoning_effort'), provider: f.provider ?? 'openai', apiKey: f.api_key || null }).catch((e) => ({ state: 'failed', message: String(e) }));
      test.disabled = false; result.className = `res${r.state === 'working' ? ' good' : ' bad'}`; result.textContent = r.message;
      if (r.endpoint && r.endpoint !== f.endpoint) { f.endpoint = r.endpoint; inputs.endpoint.value = r.endpoint; }
    });
    return h('div', { class: 'conn-edit' },
      h('h4', { text: editing.original ? `Edit ${editing.original}` : 'A new connection' }),
      input('name', 'Name', 'What the tasks call it: NAS, Claude, Azure…', { placeholder: 'NAS' }),
      h('div', { class: 'conn-field' }, h('span', { class: 'name', text: 'Kind of server' }), h('span', { class: 'help', text: 'Decides how Maru signs in and where the model name goes.' }), typeBox),
      input('endpoint', 'Address', 'The full address, or a bare IP or name to look for with Find the server.', { placeholder: 'http://192.168.1.20:11434' }),
      discovery.el,
      input('model_name', 'Model', 'The model that server should use, exactly as it names it.', { placeholder: 'gemma4-12b' }),
      input('api_key', 'API key', 'Leave empty for a server without one. It is stored in config.toml on this PC.', { type: 'password' }),
      h('div', { class: 'test-row' }, btn(editing.original ? 'Keep these changes' : 'Add it', done, 'primary'), test, btn('Cancel', () => { editing = null; rebuildAll(); }), result),
      problem,
      h('p', { class: 'help', text: 'Nothing is written until you press Save at the bottom of the page.' }));
  }

  // ───────────── actions that belong to a section ─────────────

  function ttsTest() {
    const res = h('span', { class: 'res' });
    const button = btn('Test the voice', async () => {
      button.disabled = true; res.className = 'res'; res.textContent = 'Speaking…';
      try {
        const clip = await invoke('tts_test', { text: null });
        await new Audio(`data:${clip.mime};base64,${clip.data}`).play();
        res.textContent = `${clip.seconds ? `${clip.seconds.toFixed(1)} s of speech` : 'Speech'} ${clip.cached ? 'from the cache' : `made in ${(clip.took_ms / 1000).toFixed(1)} s`}`;
      } catch (e) { res.className = 'res bad'; res.textContent = String(e); } finally { button.disabled = false; }
    });
    updaters.push(() => {
      const dirty = Object.keys(draft).some((p) => p.startsWith('tts.'));
      button.disabled = dirty;
      button.title = dirty ? 'Save first: the test uses the saved settings' : '';
    });
    return h('div', { class: 'test-row' }, button, res);
  }

  function ankiActions() {
    const res = h('span', { class: 'res' });
    const run = (what, command, ok, waiting = 'Asking Anki…') => btn(what, async (e) => {
      const button = e.target;
      button.disabled = true; res.className = 'res'; res.textContent = waiting;
      try { res.textContent = ok(await invoke(command)); } catch (err) { res.className = 'res bad'; res.textContent = String(err); } finally { button.disabled = false; }
    });
    const all = [
      run('Check Anki', 'anki_check', (r) => r),
      run('Sync now', 'anki_sync', () => 'Asked Anki to sync with AnkiWeb. Anki shows its own progress.'),
      run('Refresh my progress', 'anki_refresh', (n) => `Read ${n} words from Anki.`),
      run('Add missing sounds', 'anki_fill_sounds', soundsText, 'Making sounds… the voice can take a minute to start.'),
    ];
    updaters.push(() => {
      const dirty = ankiSettingsPending(draft, data.pending_restart);
      for (const b of all) { b.disabled = dirty; b.title = dirty ? 'Save first, then restart: these use the settings Maru started with' : ''; }
    });
    return h('div', { class: 'test-row' }, all, res);
  }

  function installed() {
    if (!status) return null;
    const line = (what, ok, detail, command) => h('tr', {}, h('td', { text: what }), h('td', {}, dot(stateDot(ok ? 'working' : 'failed')), ' ', detail, !ok && command && [' ', h('span', { class: 'cmd', text: command }), ' ', btn('Copy', (e) => copy(command, e.target))]));
    return h('section', { class: 'set-sec' }, h('h3', { text: 'Installed on this PC' }), h('p', { class: 'intro', text: 'For anything missing, run the command in a terminal, then restart Maru.' }),
      h('table', { class: 'comp' },
        line("This PC's model", status.llm === 'ready' || status.llm === 'loads on demand', status.llm, 'maru setup llm'),
        line('Word splitter (Kiwi)', status.analyser === 'kiwi', status.analyser === 'kiwi' ? 'installed' : 'basic rules only', 'maru setup kiwi'),
        line('Dictionary', status.dictionary, status.dictionary ? 'installed' : 'not installed', 'maru dict download'),
        line('Text reader (PP-OCR)', status.ocr, status.ocr ? 'installed' : 'not installed', 'maru setup ocr'),
        line('Reading model', status.ocr_reader !== 'off' && !String(status.ocr_reader).startsWith('failed'), status.ocr_reader === 'off' ? 'not in use' : status.ocr_reader, 'maru setup ocr-vl'),
        line('Voice', status.tts !== 'off' && !String(status.tts).startsWith('failed'), status.tts, 'maru setup tts')),
      h('div', { class: 'file-row' }, h('span', {}, 'Settings file: ', h('span', { class: 'cmd', text: data.path })), btn('Show in folder', () => invoke('open_config_file').catch((e) => { message = { kind: 'bad', text: String(e) }; refreshAll(); })), h('span', { text: 'Comments you add there are kept when you save here.' })));
  }

  async function copy(text, el) {
    try { await navigator.clipboard.writeText(text); el.textContent = 'Copied'; setTimeout(() => (el.textContent = 'Copy'), 1200); } catch { /* the command is on screen */ }
  }

  // ───────────── page ─────────────

  const banner = h('div');
  const bar = h('div', { class: 'savebar' });
  const msg = h('div', { class: 'msg' });
  const saveBtn = btn('Save', () => save(), 'primary');
  const discardBtn = btn('Discard', () => { draft = {}; message = null; editing = null; stopCapture(); rebuildAll(); });
  bar.append(msg, discardBtn, saveBtn);

  /** One banner for everything that waits for a restart, instead of a mark on every row. */
  function drawBanner() {
    const pending = data.pending_restart;
    banner.replaceChildren();
    if (pending.length) banner.append(h('div', { class: 'banner restart' }, h('div', {}, h('b', { text: 'Restart to apply: ' }), pendingSummary(pending, labelFor), '.'), btn('Restart Maru', () => {
      if (Object.keys(draft).length) { message = { kind: 'bad', text: 'Save or discard the unsaved changes before restarting.' }; refreshAll(); return; }
      invoke('restart_app').catch((e) => { message = { kind: 'bad', text: String(e) }; refreshAll(); });
    }, 'primary')));
    if (data.lite_from_env) banner.append(h('div', { class: 'banner restart' }, 'MARU_LITE is set in the environment, so game mode is on whatever this page says.'));
  }

  function drawBar() {
    const n = Object.keys(draft).length;
    msg.className = `msg${message?.kind === 'bad' ? ' bad' : ''}`;
    msg.textContent = message ? message.text : n ? `${n} unsaved change${n === 1 ? '' : 's'}` : 'All changes are saved';
    saveBtn.disabled = discardBtn.disabled = n === 0 || saving;
  }

  async function save() {
    if (saving) return;
    saving = true; root.inert = true; message = { kind: 'ok', text: 'Saving…' }; refreshAll();
    try {
      const r = await invoke('save_settings', { changes: draft });
      const waits = r.changed.filter((p) => !appliesLive(p)).length;
      data = await invoke('get_settings');
      tasks = await invoke('task_status').catch(() => tasks);
      draft = {};
      const warn = r.warnings?.length ? ` ${r.warnings.join(' ')}` : '';
      message = { kind: r.warnings?.length ? 'bad' : 'ok', text: (!r.changed.length ? 'Nothing changed.' : waits ? 'Saved. Some changes wait for a restart (see the top of the page).' : 'Saved. Everything is in effect now.') + warn };
      rebuildAll();
    } catch (e) {
      message = { kind: 'bad', text: String(e) };
      refreshAll();
    } finally {
      saving = false; root.inert = false; refreshAll();
    }
  }

  const fieldsOf = (section, advanced) => section.fields.map((f) => ({ ...f, section: section.id })).filter((f) => !!f.advanced === advanced);

  function sectionView(section) {
    return h('section', { class: 'set-sec' },
      h('h3', {}, section.title, section.subtitle && h('small', { text: ` ${section.subtitle}` })),
      section.intro && h('p', { class: 'intro', text: section.intro }),
      (section.tasks ?? []).map(taskLine),
      section.custom === 'connections' && [connectionsView(), h('h4', { text: "This PC's model" })],
      fieldsOf(section, false).map(row),
      section.action === 'tts-test' && ttsTest(),
      section.action === 'anki' && ankiActions(),
      section.fields.some((f) => f.advanced) && h('p', { class: 'more' }, 'Less common settings for this are in ', h('a', { href: '#', text: 'Advanced', onclick: (e) => { e.preventDefault(); go('advanced'); } }), '.'));
  }

  function advancedView() {
    return [
      h('section', { class: 'set-sec' }, h('h3', { text: 'Advanced' }), h('p', { class: 'intro', text: 'Settings that rarely need changing, grouped by the section they belong to.' }),
        SECTIONS.filter((s) => s.fields.some((f) => f.advanced)).map((s) => [h('h4', { text: s.title }), fieldsOf(s, true).map(row)])),
      installed(),
    ];
  }

  function searchView() {
    const hits = SECTIONS.map((s) => [s, s.fields.map((f) => ({ ...f, section: s.id })).filter((f) => matchesSetting(f, query))]).filter(([, fields]) => fields.length);
    if (!hits.length) return h('p', { class: 'settings-empty', text: 'No setting or feature matches that. Try a feature name such as "scan", "cards" or "live".' });
    return hits.map(([s, fields]) => h('section', { class: 'set-sec' }, h('h3', {}, s.title, h('small', { text: ` ${fields.length} match${fields.length === 1 ? '' : 'es'}` })), fields.map(row)));
  }

  function rebuildAll() {
    stopCapture();
    updaters.length = 0;
    rows.clear();
    drawBanner();
    sidebar.replaceChildren(...CATEGORIES.map((c) => h('button', { type: 'button', text: c.title, 'aria-current': String(!query && category === c.id), onclick: () => go(c.id) })));
    const section = SECTIONS.find((s) => s.id === category);
    const view = query ? searchView() : category === 'home' ? homeView() : category === 'advanced' ? advancedView() : sectionView(section);
    content.replaceChildren(...[].concat(view).flat(Infinity).filter(Boolean));
    updaters.push(drawBar);
    if (!root.children.length) root.append(h('header', { class: 'settings-head' }, h('div', {}, h('h2', { text: 'Settings' }), h('p', { text: 'What Maru does with a line, step by step, and what each step uses.' })), search), banner, h('div', { class: 'settings-grid' }, sidebar, content), bar);
    refreshAll();
  }

  rebuildAll();
  return [root];
}
