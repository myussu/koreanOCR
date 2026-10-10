// The Settings tab of the hub window. The rules live in settings-core.js; this file draws them.
//
// Edits are collected as a draft (only what differs from the file) and written with one Save. Typing
// never re-renders the page (that would drop the cursor): each field updates itself.

import { invoke } from './bridge.js';
import { SECTIONS, isVisible, appliesLive, labelFor, toEditor, fromEditor, withEdit, withReset, shown, isDefault, valueAt, splitKeys, joinKeys, comboFromEvent, keyRisk } from './settings-core.js';

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
  document.head.append(
    h('style', { id: 'settings-style' }, `
    .set-sec { margin: 22px 0 6px; }
    .set-sec h3 { margin: 0 0 4px; font-size: 16px; }
    .set-sec .intro { color: var(--muted); font-size: 12.5px; margin: 0 0 8px; line-height: 1.45; }
    .set-row { display: grid; grid-template-columns: 1fr minmax(210px, 300px); gap: 4px 18px; align-items: center; padding: 9px 2px; border-bottom: 1px solid var(--line); }
    .set-row.keysrow { grid-template-columns: minmax(170px, 1fr) minmax(300px, 440px); }
    .set-row .name { font-size: 14px; }
    .set-row .name .tag { margin-left: 6px; font-size: 10.5px; padding: 0 6px; border-radius: 999px; background: var(--chip); color: var(--muted); }
    .set-row .name .tag.restart { color: var(--warn); }
    .set-row .help { color: var(--muted); font-size: 12px; margin-top: 1px; line-height: 1.35; }
    .set-row.changed .name::before { content: "● "; color: var(--accent); }
    .set-ctl { display: flex; gap: 8px; align-items: center; justify-content: flex-end; min-width: 0; }
    .set-ctl input[type=text], .set-ctl input[type=number], .set-ctl select { flex: 1; min-width: 0; font: inherit; font-size: 13px; padding: 5px 8px; border-radius: 8px; border: 1px solid var(--line); background: var(--bg); color: var(--fg); }
    .set-ctl input[type=number] { max-width: 110px; flex: none; }
    .set-ctl input[type=range] { flex: 1; accent-color: var(--accent); }
    .set-ctl .val { min-width: 62px; text-align: right; font-size: 12.5px; color: var(--muted); font-variant-numeric: tabular-nums; }
    .set-ctl .unit { color: var(--muted); font-size: 12px; white-space: nowrap; }
    .reset { all: unset; cursor: pointer; color: var(--muted); padding: 0 4px; font-size: 14px; visibility: hidden; }
    .reset.on { visibility: visible; }
    .reset:hover { color: var(--accent); }
    .sw { position: relative; width: 38px; height: 22px; flex: none; }
    .sw input { position: absolute; inset: 0; opacity: 0; margin: 0; cursor: pointer; }
    .sw i { position: absolute; inset: 0; border-radius: 999px; background: var(--line); transition: background .15s; pointer-events: none; }
    .sw i::after { content: ""; position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: transform .15s; }
    .sw input:checked + i { background: var(--accent); }
    .sw input:checked + i::after { transform: translateX(16px); }
    .sw input:focus-visible + i { outline: 2px solid var(--accent); outline-offset: 2px; }
    .keys { display: flex; gap: 6px; align-items: flex-start; justify-content: flex-end; }
    .keys .list { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; justify-content: flex-end; }
    .keys .k { display: inline-flex; align-items: center; gap: 4px; background: var(--chip); border-radius: 7px; padding: 2px 4px 2px 8px; font-size: 12.5px; font-family: ui-monospace, Consolas, monospace; }
    .keys .k button { all: unset; cursor: pointer; color: var(--muted); padding: 0 4px; border-radius: 5px; }
    .keys .k button:hover { color: var(--bad); }
    .keys .k.warn { outline: 1px solid var(--warn); }
    .keys .add { font-size: 12px; }
    .keys .add.listening { background: var(--accent); color: #fff; border-color: var(--accent); }
    .keys-note { grid-column: 1 / -1; font-size: 11.5px; color: var(--warn); text-align: right; }
    .set-row[hidden] { display: none; }
    .test-row { display: flex; gap: 10px; align-items: center; margin: 10px 0 2px; flex-wrap: wrap; }
    .test-row .res { font-size: 12.5px; color: var(--muted); }
    .test-row .res.bad { color: var(--bad); }
    .adv-toggle { all: unset; cursor: pointer; color: var(--muted); font-size: 12.5px; padding: 6px 2px; }
    .adv-toggle:hover { color: var(--accent); }
    .banner { margin: 14px 0 0; padding: 10px 14px; border-radius: 10px; font-size: 13px; }
    .banner.restart { background: rgba(217, 119, 6, 0.12); border: 1px solid var(--warn); display: flex; gap: 12px; align-items: center; }
    .banner.restart div { flex: 1; }
    .banner.ok { background: rgba(21, 128, 61, 0.10); color: var(--good); }
    .banner.bad { background: rgba(180, 35, 24, 0.10); color: var(--bad); }
    .savebar { position: sticky; bottom: 0; margin: 20px -22px -40px; padding: 11px 22px; background: var(--bg); border-top: 1px solid var(--line); display: flex; gap: 10px; align-items: center; z-index: 5; }
    .savebar .msg { flex: 1; font-size: 13px; color: var(--muted); }
    .savebar .msg.bad { color: var(--bad); }
    .comp td:nth-child(2) { width: 46%; }
    .cmd { font-family: ui-monospace, Consolas, monospace; font-size: 12px; background: var(--chip); border-radius: 6px; padding: 2px 7px; }
    .file-row { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; margin-top: 6px; font-size: 12.5px; color: var(--muted); }
  `),
  );
};

const btn = (text, onclick, cls = '') => h('button', { class: `btn ${cls}`, onclick, text });

export async function settingsView() {
  ensureStyles();
  const root = h('div');
  let data = await invoke('get_settings');
  const status = await invoke('status').catch(() => null);
  let draft = {};
  let message = null; // { kind: 'ok' | 'bad', text }
  let capturing = null; // { path, stop }
  const updaters = [];
  const open = new Set(); // sections with Advanced expanded

  const refreshAll = () => updaters.forEach((u) => u());

  // ───────────── controls ─────────────

  function control(field) {
    const value = () => shown(draft, data.values, data.defaults, field.path);
    const edit = (v) => {
      draft = withEdit(draft, data.values, field.path, v);
      message = null;
      refreshAll();
    };
    const reset = h('button', { class: 'reset', title: 'Back to the default', text: '↺', onclick: () => ((draft = withReset(draft, data.values, data.defaults, field.path)), (message = null), rebuild(field)) });
    const sync = () => reset.classList.toggle('on', !isDefault(draft, data.values, data.defaults, field.path));
    let input;

    if (field.type === 'toggle') {
      input = h('input', { type: 'checkbox', checked: value(), onchange: (e) => edit(e.target.checked) });
      return { el: h('div', { class: 'set-ctl' }, h('label', { class: 'sw' }, input, h('i')), reset), sync, rebuild: () => (input.checked = value()) };
    }
    if (field.type === 'select') {
      input = h('select', { onchange: (e) => edit(e.target.value) }, field.options.map((o) => h('option', { value: o.value, text: o.label })));
      const set = () => (input.value = value());
      set();
      return { el: h('div', { class: 'set-ctl' }, input, reset), sync, rebuild: set };
    }
    if (field.type === 'range') {
      const val = h('span', { class: 'val' });
      input = h('input', { type: 'range', min: field.min, max: field.max, step: field.step, oninput: (e) => edit(Number(e.target.value)) });
      const set = () => {
        input.value = toEditor(field, value());
        val.textContent = `${toEditor(field, value())}${field.unit ? ` ${field.unit}` : ''}`;
      };
      set();
      return { el: h('div', { class: 'set-ctl' }, input, val, reset), sync: () => (sync(), set()), rebuild: set };
    }
    if (field.type === 'number' || field.type === 'percent') {
      input = h('input', {
        type: 'number', min: field.min, max: field.max, step: field.step,
        oninput: (e) => {
          const n = e.target.valueAsNumber;
          if (!Number.isNaN(n)) edit(fromEditor(field, n));
        },
      });
      const set = () => (input.value = toEditor(field, value()));
      set();
      const unit = field.type === 'percent' ? '%' : field.unit;
      return { el: h('div', { class: 'set-ctl' }, input, unit && h('span', { class: 'unit', text: unit }), reset), sync, rebuild: set };
    }
    if (field.type === 'text' || field.type === 'password' || field.type === 'tags') {
      input = h('input', { type: field.type === 'password' ? 'password' : 'text', autocomplete: 'off', placeholder: field.placeholder ?? '', spellcheck: 'false', oninput: (e) => edit(fromEditor(field, e.target.value)) });
      const set = () => (input.value = toEditor(field, value()));
      set();
      return { el: h('div', { class: 'set-ctl' }, input, reset), sync, rebuild: set };
    }
    return null;
  }

  // ───────────── shortcuts ─────────────

  function keysControl(field) {
    const box = h('div', { class: 'keys' });
    const note = h('div', { class: 'keys-note' });
    const list = () => splitKeys(shown(draft, data.values, data.defaults, field.path));
    const save = (next) => {
      draft = withEdit(draft, data.values, field.path, joinKeys(next));
      message = null;
      refreshAll();
    };
    const reset = h('button', { class: 'reset', title: 'Back to the default', text: '↺', onclick: () => ((draft = withReset(draft, data.values, data.defaults, field.path)), (message = null), refreshAll()) });
    const draw = () => {
      const keys = list();
      const risks = keys.map(keyRisk);
      const listening = capturing?.path === field.path;
      box.replaceChildren(
        h(
          'div',
          { class: 'list' },
          ...keys.map((k, i) =>
            h('span', { class: `k${risks[i] ? ' warn' : ''}` }, k, h('button', { title: 'Remove', text: '×', onclick: () => save(keys.filter((_, j) => j !== i)) })),
          ),
          h('button', {
            class: `btn add${listening ? ' listening' : ''}`,
            text: listening ? 'Press the keys…  (Esc to cancel)' : keys.length ? '+ Add another key' : '+ Set key',
            onclick: () => (listening ? stopCapture() : startCapture(field, keys, save)),
          }),
        ),
        reset,
      );
      reset.classList.toggle('on', !isDefault(draft, data.values, data.defaults, field.path));
      note.textContent = listening && capturing.problem ? capturing.problem : risks.find(Boolean) ?? '';
    };
    return { el: [box, note], sync: draw, rebuild: draw };
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
      if (combo === 'unknown') {
        capturing.problem = 'Maru has no name for that key. Try another.';
        return refreshAll();
      }
      if (!keys.includes(combo)) save([...keys, combo]);
      stopCapture();
    };
    addEventListener('keydown', onKey, true);
    capturing = { path: field.path, stop: () => removeEventListener('keydown', onKey, true), problem: '' };
    refreshAll();
  }

  // ───────────── rows ─────────────

  function row(field) {
    const ctl = field.type === 'keys' ? keysControl(field) : control(field);
    const live = appliesLive(field.path);
    const holder = h('div', { class: `set-row${field.type === 'keys' ? ' keysrow' : ''}` });
    const name = h('div', {}, h('div', { class: 'name' }, field.label, !live && h('span', { class: 'tag restart', title: 'Takes effect after a restart', text: 'restart' })), field.help && h('div', { class: 'help', text: field.help }));
    const build = () => {
      holder.classList.toggle('changed', field.path in draft);
      holder.replaceChildren(...(field.type === 'keys' ? [name, ...[].concat(ctl.el)] : [name, ctl.el]));
    };
    build();
    updaters.push(() => {
      holder.classList.toggle('changed', field.path in draft);
      holder.hidden = !isVisible(field, draft, data.values, data.defaults);
      ctl.sync();
    });
    holder.dataset.path = field.path;
    holder.rebuildField = () => ctl.rebuild();
    return holder;
  }

  const rows = new Map();
  function rebuild(field) {
    rows.get(field.path)?.rebuildField();
    refreshAll();
  }

  // ───────────── page ─────────────

  const banner = h('div');
  const bar = h('div', { class: 'savebar' });
  const msg = h('div', { class: 'msg' });
  const saveBtn = btn('Save', () => save(), 'primary');
  const discardBtn = btn('Discard', () => ((draft = {}), (message = null), stopCapture(), rebuildAll()));
  bar.append(msg, discardBtn, saveBtn);

  function drawBanner() {
    const pending = data.pending_restart;
    banner.replaceChildren();
    if (pending.length) {
      const names = [...new Set(pending.map(labelFor))];
      banner.append(
        h('div', { class: 'banner restart' }, h('div', {}, h('b', { text: 'Restart to apply: ' }), names.slice(0, 6).join(', '), names.length > 6 ? ` and ${names.length - 6} more` : '', '. The language model loads again afterwards.'), btn('Restart Maru', () => invoke('restart_app'), 'primary')),
      );
    }
    if (data.lite_from_env) banner.append(h('div', { class: 'banner restart' }, 'MARU_LITE is set in the environment, so game mode is on whatever this page says.'));
  }

  function drawBar() {
    const n = Object.keys(draft).length;
    msg.className = `msg${message?.kind === 'bad' ? ' bad' : ''}`;
    msg.textContent = message ? message.text : n ? `${n} unsaved change${n === 1 ? '' : 's'}` : 'All changes are saved';
    saveBtn.disabled = n === 0;
    discardBtn.disabled = n === 0;
    saveBtn.style.opacity = discardBtn.style.opacity = n === 0 ? 0.5 : 1;
  }
  updaters.push(drawBar);

  async function save() {
    try {
      const r = await invoke('save_settings', { changes: draft });
      const live = r.changed.filter(appliesLive).length;
      data = await invoke('get_settings');
      draft = {};
      const warn = r.warnings?.length ? ` ${r.warnings.join(' ')}` : '';
      message = { kind: r.warnings?.length ? 'bad' : 'ok', text: (r.changed.length ? `Saved. ${live ? 'Shortcuts and panel changes are active now.' : ''}` : 'Nothing changed.') + warn };
      rebuildAll();
    } catch (e) {
      message = { kind: 'bad', text: String(e) };
      refreshAll();
    }
  }

  const fieldRows = (section, advanced) => section.fields.filter((f) => !!f.advanced === advanced).map((f) => {
    const r = row(f);
    rows.set(f.path, r);
    return r;
  });

  function rebuildAll() {
    updaters.length = 0;
    rows.clear();
    drawBanner();
    const sections = SECTIONS.map((s) => {
      const adv = s.fields.some((f) => f.advanced);
      const advBox = h('div');
      const draw = () => {
        advBox.replaceChildren();
        if (open.has(s.id)) advBox.append(...fieldRows(s, true));
        toggle.textContent = open.has(s.id) ? 'Hide advanced ▴' : 'Advanced ▾';
      };
      const toggle = h('button', { class: 'adv-toggle', onclick: () => (open.has(s.id) ? open.delete(s.id) : open.add(s.id), draw(), refreshAll()) });
      const body = fieldRows(s, false);
      if (adv) draw();
      return h('section', { class: 'set-sec' }, h('h3', { text: s.title }), s.intro && h('p', { class: 'intro', text: s.intro }), body, adv && toggle, adv && advBox, s.action === 'tts-test' && ttsTest(), s.action === 'anki' && ankiActions());
    });
    updaters.push(drawBar);
    root.replaceChildren(banner, h('h2', { text: 'Settings', style: 'margin:0' }), ...sections, components(), fileRow(), bar);
    refreshAll();
  }

  // ───────────── speech test ─────────────

  function ttsTest() {
    const res = h('span', { class: 'res' });
    const button = btn('Test voice', async () => {
      button.disabled = true;
      res.className = 'res';
      res.textContent = 'Speaking…';
      try {
        const clip = await invoke('tts_test', { text: null });
        const audio = new Audio(`data:${clip.mime};base64,${clip.data}`);
        await audio.play();
        res.textContent = `${clip.seconds ? `${clip.seconds.toFixed(1)} s of speech` : 'Speech'} ${clip.cached ? 'from the cache' : `made in ${(clip.took_ms / 1000).toFixed(1)} s`}`;
      } catch (e) {
        res.className = 'res bad';
        res.textContent = String(e);
      } finally {
        button.disabled = false;
      }
    });
    const sync = () => {
      const dirty = Object.keys(draft).some((p) => p.startsWith('tts.'));
      button.disabled = dirty;
      button.title = dirty ? 'Save first: the test uses the saved settings' : '';
    };
    updaters.push(sync);
    return h('div', { class: 'test-row' }, button, res, h('span', { class: 'res', text: 'Uses the saved settings.' }));
  }

  // ───────────── Anki ─────────────

  function ankiActions() {
    const res = h('span', { class: 'res' });
    const run = (what, command, ok) =>
      btn(what, async (e) => {
        const button = e.target;
        button.disabled = true;
        res.className = 'res';
        res.textContent = 'Asking Anki…';
        try {
          const r = await invoke(command);
          res.textContent = ok(r);
        } catch (err) {
          res.className = 'res bad';
          res.textContent = String(err);
        } finally {
          button.disabled = false;
        }
      });
    const check = run('Check Anki', 'anki_check', (r) => r);
    const sync = run('Sync now', 'anki_sync', () => 'Asked Anki to sync with AnkiWeb. Anki shows its own progress.');
    const sync_ = () => {
      const dirty = Object.keys(draft).some((p) => p.startsWith('learning.anki.'));
      check.disabled = sync.disabled = dirty;
      check.title = sync.title = dirty ? 'Save first, then restart: these use the settings Maru started with' : '';
    };
    updaters.push(sync_);
    return h('div', { class: 'test-row' }, check, sync, res);
  }

  // ───────────── components and the file ─────────────

  function components() {
    if (!status) return h('div');
    const row = (what, ok, detail, command) =>
      h('tr', {}, h('td', { text: what }), h('td', {}, ok ? h('span', { class: 'flag on', text: detail }) : [h('span', { class: 'flag off', text: detail }), ' ', command && h('span', { class: 'cmd', text: command }), ' ', command && btn('Copy', (e) => copy(command, e.target))]));
    return h(
      'section',
      { class: 'set-sec' },
      h('h3', { text: 'Models and downloads' }),
      h('p', { class: 'intro', text: 'What is installed. For anything missing, run the command in a terminal; then restart Maru.' }),
      h(
        'table',
        { class: 'comp' },
        row('Translation model', status.llm === 'ready', status.llm === 'ready' ? 'ready' : status.llm, 'maru setup llm'),
        row('Korean analyser', status.analyser === 'kiwi', status.analyser === 'kiwi' ? 'Kiwi' : `${status.analyser} (basic)`, 'maru setup kiwi'),
        row('Dictionary', status.dictionary, status.dictionary ? 'installed' : 'not installed', 'maru dict download'),
        row('Text reader (PP-OCR)', status.ocr, status.ocr ? 'ready' : 'not installed', 'maru setup ocr'),
        row('Speech', status.tts === 'ready', status.tts === 'off' ? 'off' : status.tts === 'ready' ? (status.tts_source === 'local' ? 'ready (this PC)' : 'ready (server)') : status.tts, 'maru setup tts'),
        row('OCR reading model', status.ocr_reader === 'ready', status.ocr_reader === 'ready' ? 'ready' : status.ocr_reader === 'off' ? 'not in use' : status.ocr_reader, 'maru setup ocr-vl'),
      ),
    );
  }

  async function copy(text, el) {
    try {
      await navigator.clipboard.writeText(text);
      el.textContent = 'Copied';
      setTimeout(() => (el.textContent = 'Copy'), 1200);
    } catch {
      /* clipboard unavailable: the command is on screen */
    }
  }

  function fileRow() {
    return h('div', { class: 'file-row' }, h('span', {}, 'Settings file: ', h('span', { class: 'cmd', text: data.path })), btn('Show in folder', () => invoke('open_config_file').catch((e) => ((message = { kind: 'bad', text: String(e) }), refreshAll()))), h('span', { text: 'Comments you add there are kept when you save here.' }));
  }

  rebuildAll();
  return [root];
}
