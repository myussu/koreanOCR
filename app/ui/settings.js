// The Settings tab of the hub window. The rules live in settings-core.js; this file draws them.
//
// Edits are collected as a draft (only what differs from the file) and written with one Save. Typing
// never re-renders the page (that would drop the cursor): each field updates itself.

import { invoke } from './bridge.js';
import { ankiSettingsPending } from './anki.js';
import { LIVE_MODELS, modelEdits, matchesSetting } from './settings-live-core.js';
import { connectionFields, connectionKey, canTestConnection, connectionResult, connectionPaths, needsServerDetection } from './settings-connection-core.js';
import { discoveryControls } from './settings-discovery.js';
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
  document.head.append(h('link', { id:'settings-style', rel:'stylesheet', href:'settings.css' }));
};

const btn = (text, onclick, cls = '') => h('button', { class: `btn ${cls}`, onclick, text });

export async function settingsView() {
  ensureStyles();
  const root = h('div', {class:'settings-page'});
  let data = await invoke('get_settings');
  const status = await invoke('status').catch(() => null);
  let draft = {};
  let message = null; // { kind: 'ok' | 'bad', text }
  let capturing = null; // { path, stop }
  const updaters = [];
  const open = new Set(); // sections with Advanced expanded
  const connectionChecks = new Map();
  const discoveryStates = new Map();
  let disposed=false;
  root.dispose=()=>{disposed=true;stopCapture();for(const state of discoveryStates.values())if(state.busy)state.stop?.();};
  let category = 'live', query = '', saving = false;
  const sidebar = h('aside', {class:'settings-sidebar', 'aria-label':'Settings categories'});
  const content = h('div');
  const search = h('input', {class:'settings-search', type:'search', placeholder:'Search settings…', 'aria-label':'Search settings', oninput:e=>{query=e.target.value; rebuildAll();}});
  root.canLeave = () => {
    if (!Object.keys(draft).length && !saving) { stopCapture(); return true; }
    message = {kind:'bad',text:'Save or discard your changes before leaving Settings.'}; refreshAll(); return false;
  };

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
      for(const input of holder.querySelectorAll('input,select')) input.setAttribute('aria-label',field.label);
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
        h('div', { class: 'banner restart' }, h('div', {}, h('b', { text: 'Restart to apply: ' }), names.slice(0, 6).join(', '), names.length > 6 ? ` and ${names.length - 6} more` : '', '.'), btn('Restart Maru', () => {
          if(Object.keys(draft).length){message={kind:'bad',text:'Save or discard the unsaved changes before restarting.'};refreshAll();return;}
          invoke('restart_app').catch(e=>{message={kind:'bad',text:String(e)};refreshAll();});
        }, 'primary')),
      );
    }
    if (data.lite_from_env) banner.append(h('div', { class: 'banner restart' }, 'MARU_LITE is set in the environment, so game mode is on whatever this page says.'));
  }

  function drawBar() {
    const n = Object.keys(draft).length;
    msg.className = `msg${message?.kind === 'bad' ? ' bad' : ''}`;
    msg.textContent = message ? message.text : n ? `${n} unsaved change${n === 1 ? '' : 's'}` : 'All changes are saved';
    saveBtn.disabled = n === 0 || saving;
    discardBtn.disabled = n === 0 || saving;
    saveBtn.style.opacity = discardBtn.style.opacity = n === 0 ? 0.5 : 1;
  }
  updaters.push(drawBar);

  async function save() {
    if(saving)return;
    saving=true; root.inert=true; message={kind:'ok',text:'Saving…'};refreshAll();
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
    } finally {
      saving=false;root.inert=false;refreshAll();
    }
  }

  const fieldRows = (section, advanced) => section.fields.filter((f) => f.path !== 'continuous.translation.model' && matchesSetting(f,query) && (!!query || !!f.advanced === advanced)).flatMap((f) => {
    const r = row(f);
    rows.set(f.path, r);
    return ['llm.model_name','continuous.translation.snapshot_model_name'].includes(f.path) ? [r, serverTest(section.id)] : [r];
  });

  function rebuildAll() {
    stopCapture();
    updaters.length = 0;
    rows.clear();
    drawBanner();
    sidebar.replaceChildren(...[...SECTIONS, {id:'components',title:'Installed resources',fields:[]}].map(s=>btn(s.title,()=>{category=s.id;query='';search.value='';rebuildAll();})));
    [...sidebar.children].forEach((b,i)=>b.setAttribute('aria-current',String(!query && category===[...SECTIONS,{id:'components'}][i].id)));
    const wanted = SECTIONS.filter(s=>query ? s.fields.some(f=>matchesSetting(f,query)) : s.id===category);
    const sections = wanted.map((s) => {
      const adv = s.fields.some((f) => f.advanced);
      const advBox = h('div');
      const draw = () => {
        advBox.replaceChildren();
        if (open.has(s.id)) advBox.append(...fieldRows(s, true));
        toggle.textContent = open.has(s.id) ? 'Hide advanced ▴' : 'Advanced ▾';
      };
      const toggle = h('button', { class: 'adv-toggle', onclick: () => (open.has(s.id) ? open.delete(s.id) : open.add(s.id), draw(), refreshAll()) });
      const body = fieldRows(s, false);
      if (adv && !query) draw();
      return h('section', { class: 'set-sec' }, h('h3', { text: s.title }), s.intro && h('p', { class: 'intro', text: s.intro }), s.id==='live' && liveCards(), body, adv && !query && toggle, adv && !query && advBox, s.action === 'tts-test' && ttsTest(), s.action === 'anki' && ankiActions());
    });
    updaters.push(drawBar);
    content.replaceChildren(...[...sections, !query && category==='components' && components(), !query && category==='components' && fileRow(), query && !sections.length && h('p',{class:'settings-empty',text:'No settings match this search.'})].filter(el=>el!==false && el!==''));
    if(!root.children.length) root.append(h('header',{class:'settings-head'},h('div',{},h('h2',{text:'Settings'}),h('p',{text:'Make Maru fit the way you read and play.'})),search),banner,h('div',{class:'settings-grid'},sidebar,content),bar);
    refreshAll();
  }

  function liveCards() {
    const selected=shown(draft,data.values,data.defaults,'continuous.translation.model');
    const values=Object.fromEntries(LIVE_MODELS.filter(m=>m.path).map(m=>[m.path,shown(draft,data.values,data.defaults,m.path)]));
    const cards=LIVE_MODELS.map(m=>h('button',{class:'live-card','aria-pressed':String(selected===m.id),onclick:()=>{
      for(const [path,v] of Object.entries(modelEdits(m.id,values,data.live_models))) draft=withEdit(draft,data.values,path,v);
      message=null;rebuildAll();
    }},h('span',{class:'model-tag',text:m.tag}),h('strong',{text:m.name}),h('small',{text:m.note}),m.path && h('small',{text:values[m.path]||data.live_models?.find(f=>f.model===m.id)?.path?'Local file available':'Choose a local GGUF file below'})));
    return h('div',{},h('div',{class:'live-cards','aria-label':'Live translation models'},cards),h('p',{class:'live-note',text:'The lightweight choices leave more GPU memory for a game, with more translation errors. Their tested GGUF files use smaller runtime buffers. Choose GPU or CPU below, save and restart. Only one owned local translation model is kept loaded. OCR and speech have separate memory costs.'}));
  }

  // ───────────── speech test ─────────────

  function serverTest(section) {
    const fields = () => connectionFields(section, p => shown(draft,data.values,data.defaults,p));
    const edit=(path,value)=>{draft=withEdit(draft,data.values,path,value);message=null;rows.get(path)?.rebuildField();refreshAll();};
    if(!discoveryStates.has(section))discoveryStates.set(section,{});
    const discoveryState=discoveryStates.get(section);
    const discovery=discoveryControls({section,state:discoveryState,fields,edit,refresh:refreshAll,disposed:()=>disposed});
    updaters.push(discovery.sync);
    const result = h('span', {class:'res', role:'status', 'aria-live':'polite'});
    const button = btn('Test connection', async () => {
      let args = fields();
      if (!canTestConnection(args) || connectionChecks.get(section)?.busy || discoveryState.busy) return;
      const state = {key:connectionKey(args), busy:true, phase:needsServerDetection(args.endpoint) ? 'discovery' : 'inference', result:null};
      connectionChecks.set(section,state);
      refreshAll();
      try {
        await discovery.resolveForTest();
        const current=fields();
        if(disposed || args.modelName!==current.modelName || args.reasoningEffort!==current.reasoningEffort)throw Error('Settings changed during discovery. Test the current values again.');
        args=current;state.key=connectionKey(args);state.phase='inference';refreshAll();
        const reply = await invoke('test_model_connection', args);
        if(reply.endpoint && state.key===connectionKey(fields()) && !disposed){
          const paths=connectionPaths(section);state.key=connectionKey({...args,endpoint:reply.endpoint});edit(paths.endpoint,reply.endpoint);
        }
        state.result = {kind:'ok', text:`Connected · model replied in ${(reply.took_ms/1000).toFixed(1)} s: ${reply.reply}`};
      } catch (e) {
        if(state.phase==='discovery' && discoveryState.selected===fields().endpoint)state.key=connectionKey(fields());
        state.result = {kind:'bad', text:`Connection failed: ${String(e)}`};
      } finally {
        state.busy=false;
        refreshAll();
      }
    });
    const sync = () => {
      const args=fields(), state=connectionChecks.get(section);
      // Editing invalidates the old result, even if the fields are changed back later.
      if(state && state.key!==connectionKey(args)) state.result=null;
      const reply=connectionResult(state,args);
      button.disabled=!!state?.busy || !!discoveryState.busy || !canTestConnection(args);
      button.textContent=state?.busy ? 'Testing…' : 'Test connection';
      result.className=`res${reply?.kind==='bad' ? ' bad' : ''}`;
      result.textContent=state?.busy ? state.phase==='discovery' ? 'Detecting the model server…' : 'Waiting for the server model (up to 60 s)…' : reply?.text ?? '';
    };
    updaters.push(sync);
    return h('div',{class:'server-test', 'data-connection':section},
      discovery.el,
      h('div',{class:'help',text:'Tests the address and model entered above before saving. Sends only “안녕하세요.” with a translation instruction; the server may load its model.'}),
      h('div',{class:'test-row'},button,result));
  }

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
    const progress = run('Refresh progress', 'anki_refresh', (n) => `Read ${n} words from Anki; local learning progress is updated.`);
    const sync_ = () => {
      const dirty = ankiSettingsPending(draft,data.pending_restart);
      check.disabled = sync.disabled = progress.disabled = dirty;
      check.title = sync.title = progress.title = dirty ? 'Save first, then restart: these use the settings Maru started with' : '';
    };
    updaters.push(sync_);
    return h('div', { class: 'test-row' }, check, sync, progress, res);
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
