// The floating controls: the 마 button over a game and its menu. The actions are grouped (live subtitles, capture,
// the saved game area, models and memory, other windows); each button says its shortcut and each group who
// translates for it. The rules are in launcher-core.js.
import { invoke, inTauri, mockCommands } from './bridge.js';
import { GROUPS, ACTIONS, statusText, moved, liveDetails, modelActions, scanActions, shortcutText, routeLine } from './launcher-core.js';
import { makeVramBar } from './resources.js';
import { vramState } from './resources-core.js';
import { readingQuickControls } from './reading.js';
import { installReadingDemo } from './reading-demo.js';

const button = document.querySelector('#maru'), menu = document.querySelector('#menu'), groups = document.querySelector('#groups');
let expanded = false, busy = false, drag = null, last = null;

function el(tag, props = {}, ...kids) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === 'class') node.className = v;
    else if (k === 'text') node.textContent = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) node.append(kid);
  return node;
}

// ───────────── building the menu once; refresh() only updates it ─────────────

const actionButtons = new Map();
function actionButton([id, label, toggle]) {
  const b = el('button', { type: 'button', 'data-action': id, 'aria-pressed': toggle ? 'false' : null, onclick: () => action(id) }, el('span', { class: 'label', text: label }), el('kbd', { class: 'keys' }));
  actionButtons.set(id, b);
  return b;
}
const routeLines = new Map();
function section(group, ...extra) {
  const route = group.task ? el('div', { class: 'route' }) : null;
  if (route) routeLines.set(group.task, route);
  return el('section', { class: `group ${group.id}`, 'aria-label': group.title }, el('h3', { text: group.title }), el('div', { class: 'actions' }, group.actions.map(actionButton)), route, extra);
}

const byId = Object.fromEntries(GROUPS.map((g) => [g.id, g]));
const vram = makeVramBar();
const details = { model: el('small'), ocr: el('small'), queue: el('small'), preparation: el('p', { class: 'preparation', role: 'status' }), message: el('small', { class: 'model-message', role: 'status' }) };
const areas = readingQuickControls({ beforeCapture: () => expand(false), manage: () => action('reading'), changed: () => report() });
const saveArea = el('button', { type: 'button', text: 'Save a new area', onclick: async () => {
  try {
    let name = 'Dialogue', n = 2;
    while (last?.reading?.areas.some((a) => a.exe === last.reading.exe && a.name.toLowerCase() === name.toLowerCase())) name = `Dialogue ${n++}`;
    await expand(false);
    await invoke('reading_create', { name, areaId: null });
  } catch (e) { await expand(true); showError(e); }
} });
areas.append(saveArea);

// Models and memory folds away: it is for when something is slow or the graphics card is full.
const models = el('details', { class: 'group models' }, el('summary', {}, el('span', { text: byId.models.title }), el('span', { class: 'vram-short' })),
  vram, details.model, details.ocr, details.queue, details.preparation, details.message, el('div', { class: 'actions' }, byId.models.actions.map(actionButton)));
// The saved game area folds too; whether it is open is remembered on this PC (a game without areas can fold it away).
const areaBox = el('details', { class: 'group area', open: remembered('maru.launcher.area', true) || null }, el('summary', {}, el('span', { text: 'Saved game area' })), areas);
areaBox.addEventListener('toggle', () => { remember('maru.launcher.area', areaBox.open); report(); });
groups.append(
  section(byId.live),
  section(byId.capture),
  areaBox,
  models,
  el('section', { class: 'group open', 'aria-label': byId.open.title }, el('div', { class: 'actions' }, byId.open.actions.map(actionButton))),
);

function remembered(key, otherwise) {
  try { const v = localStorage.getItem(key); return v == null ? otherwise : v === '1'; } catch { return otherwise; }
}
function remember(key, on) {
  try { localStorage.setItem(key, on ? '1' : '0'); } catch { /* the menu still works, it just forgets */ }
}

// ───────────── demo data for a plain browser ─────────────

if (!inTauri) {
  const reading = installReadingDemo();
  const state = { live: true, hover: false, scan: false, scan_id: null, scan_join_mode: false, speech: false, model: 'MiLMMT 4B Q6', model_status: 'Ready', queued: 0, active: 0, snapshot: 'Live model + offline meanings' };
  state.reading = reading;
  let nextScan = 0;
  state.live_setup = { model: 'HY-MT2 1.8B Q4', device: 'GPU', model_status: 'Ready', ocr: 'Hybrid · CPU detector + PaddleOCR-VL', ocr_status: 'Ready', preparation: 'Not prepared', busy: false };
  state.hotkeys = { translate: 'Ctrl+Alt+T | F6', region: 'Ctrl+Alt+R', hover: 'Ctrl+Alt+H | F8', continuous: 'Ctrl+Alt+C', continuous_area: 'Ctrl+Alt+Shift+C', scan: 'Ctrl+Alt+S', speak: 'Ctrl+Alt+P', history: 'Ctrl+Alt+L', dismiss: 'Ctrl+Alt+Escape', launcher: 'Ctrl+Alt+M', reading_again: '', wheel: 'Ctrl+Alt+Q', star: 'Ctrl+Alt+B' };
  state.tasks = [
    { task: 'live', connection: 'NAS', model: 'gemma4-12b', place: 'network', state: 'working', message: 'NAS answered.' },
    { task: 'capture', connection: 'Gemini', model: 'gemini-flash', place: 'hosted', state: 'quota', message: 'Gemini has no quota left.' },
  ];
  const mode = new URLSearchParams(location.search).get('demo');
  if (mode === 'queued') { state.active = 1; state.queued = 8; }
  if (mode === 'cpu-ocr') { state.live_setup.ocr = 'CPU PP-OCR'; state.live_setup.idle_reader_loaded = true; }
  mockCommands({ launcher_status: () => state, launcher_rects: () => {}, launcher_drag: () => {}, launcher_expand: () => ({ up: false, left: false }), resource_status: () => null, launcher_action: async ({ action }) => {
    const entry = ACTIONS.find(([id]) => id === action); if (entry?.[2]) state[entry[2]] = !state[entry[2]];
    if (action === 'scan') { state.scan_id = state.scan ? ++nextScan : null; state.scan_join_mode = false; }
    if (action === 'scan_refresh') { state.scan_id = ++nextScan; state.scan_join_mode = false; }
    if (action === 'prepare_live') {
      state.live_setup.busy = true; state.live_setup.preparation = 'Preparing translator…'; state.live_setup.message = null;
      await new Promise((resolve) => setTimeout(resolve, 1200));
      state.live_setup.busy = false; state.live_setup.preparation = 'Prepared'; state.live_setup.message = 'Live models prepared. You can now turn live subtitles on.';
    }
    if (action === 'release_models') {
      state.live_setup.preparation = 'Not prepared'; state.live_setup.message = state.live ? (state.live_setup.idle_reader_loaded ? 'Released idle OCR reader and speech; kept the live translator.' : 'Released idle speech; kept the live translator and OCR reader.') : 'Released translator, OCR reader and speech.';
      state.live_setup.idle_reader_loaded = false;
    }
  } });
}

// ───────────── the window ─────────────

function report() {
  const rects = [button, ...(expanded ? [menu] : [])].map((node) => { const r = node.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; });
  invoke('launcher_rects', { dpr: devicePixelRatio, rects }).catch(showError);
}
async function expand(value) {
  try {
    const layout = await invoke('launcher_expand', { expanded: value });
    expanded = value; menu.hidden = !value; button.setAttribute('aria-expanded', String(value));
    document.querySelector('#controls').classList.toggle('up', value && layout.up);
    document.querySelector('#controls').classList.toggle('left', value && layout.left);
    requestAnimationFrame(report);
    if (value) refreshResources();
  } catch (e) { showError(e); }
}
function showError(e) { document.querySelector('#error').textContent = String(e); }
async function action(id) {
  if (busy && !['hide', 'quit'].includes(id)) return; busy = true; syncModelActions();
  document.querySelector('#error').textContent = '';
  try { if (!['hide', 'quit', 'prepare_live', 'release_models'].includes(id)) await expand(false); await invoke('launcher_action', { action: id }); await refresh(); }
  catch (e) { expand(true); showError(e); } finally { busy = false; syncModelActions(); }
}

async function refresh() {
  try {
    last = await invoke('launcher_status');
    areas.update(last.reading); saveArea.disabled = busy || !last.reading?.can_create;
    document.querySelector('#state').textContent = statusText(last) + (last.active || last.queued ? '' : last.hover ? ' · hover on' : '');
    const d = liveDetails(last);
    details.model.textContent = d.model; details.ocr.textContent = d.ocr; details.queue.textContent = d.queue;
    details.preparation.textContent = d.preparation; details.message.textContent = d.message;
    for (const [id, , toggle, key] of ACTIONS) {
      const b = actionButtons.get(id);
      if (toggle) b.setAttribute('aria-pressed', String(!!last[toggle]));
      const keys = shortcutText(last.hotkeys?.[key]);
      b.querySelector('.keys').textContent = keys;
      b.querySelector('.keys').hidden = !keys;
      b.title = keys ? `${b.querySelector('.label').textContent} (${keys})` : '';
    }
    for (const [task, line] of routeLines) {
      const r = routeLine(last, task);
      line.hidden = !r;
      if (r) line.replaceChildren(...[el('span', { class: `dot ${r.kind}` }), el('span', { class: 'who', text: `Translates with ${r.text}` }), r.where && el('span', { class: 'where', text: r.where }), r.kind !== 'ok' && el('span', { class: `state ${r.kind}`, text: r.state })].filter(Boolean));
      if (r) line.title = r.title;
    }
    if (last.shortcut) document.querySelector('#tip').textContent = `Drag 마 to move · ${last.shortcut} shows / hides · Esc closes`;
    document.querySelector('#dot').className = last.model_status.startsWith('Error:') || last.live_setup?.preparation === 'Preparation failed' ? 'error' : last.model_status === 'Loading' || last.active || last.live_setup?.busy ? 'busy' : last.live ? 'on' : '';
    syncModelActions();
    report();
  } catch (e) { showError(e); }
}
function syncModelActions() {
  const state = modelActions(last, busy);
  const prepare = actionButtons.get('prepare_live'), release = actionButtons.get('release_models');
  prepare.disabled = state.prepareDisabled;
  prepare.title = last?.live ? 'Turn live off and let queued lines finish first.' : 'Warms the selected translator and OCR with fixed synthetic input. An external main translator receives a fixed Korean greeting.';
  release.disabled = state.releaseDisabled;
  release.title = 'Releases idle servers owned by Maru. Active requests and models needed by live capture stay loaded. External servers are unchanged.';
  const scan = scanActions(last, busy);
  for (const id of ['scan_join_mode', 'scan_refresh']) {
    const control = actionButtons.get(id);
    control.hidden = !scan.visible; control.disabled = scan.disabled;
    control.title = id === 'scan_join_mode' ? 'Select labels with left-click, then press Join. Click again to return to deep dive.' : 'Read and label the screen again.';
  }
}

// ───────────── dragging the button ─────────────

button.addEventListener('pointerdown', async (e) => {
  if (e.button !== 0) return;
  drag = { x: e.screenX, y: e.screenY, started: false }; button.setPointerCapture(e.pointerId);
  try { await invoke('launcher_drag', { phase: 'start', dx: 0, dy: 0 }); if (drag) drag.started = true; } catch (err) { drag = null; showError(err); }
});
let moveChain = Promise.resolve();
button.addEventListener('pointermove', (e) => {
  if (!drag?.started || !moved(drag, { x: e.screenX, y: e.screenY })) return;
  const dx = e.screenX - drag.x, dy = e.screenY - drag.y;
  moveChain = moveChain.then(() => invoke('launcher_drag', { phase: 'move', dx, dy })).catch(showError);
});
async function endDrag(e) {
  const start = drag; if (!start) return; drag = null;
  const end = { x: e.screenX, y: e.screenY };
  try { await moveChain; await invoke('launcher_drag', { phase: 'end', dx: end.x - start.x, dy: end.y - start.y }); }
  catch (err) { showError(err); }
  if (!moved(start, end) && e.type !== 'pointercancel') expand(!expanded);
}
button.addEventListener('pointerup', endDrag); button.addEventListener('pointercancel', endDrag);
button.addEventListener('click', (e) => { if (e.detail === 0) expand(!expanded); });
document.querySelector('#collapse').onclick = () => expand(false);
document.querySelector('#hide').onclick = () => action('hide'); document.querySelector('#quit').onclick = () => action('quit');
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') expand(false); });
new ResizeObserver(report).observe(menu);
models.addEventListener('toggle', () => { report(); if (models.open) refreshResources(); });

let readingResources = false;
async function refreshResources() {
  if (!expanded || document.hidden || readingResources) return;
  readingResources = true;
  try {
    const r = await invoke('resource_status');
    vram.update(r);
    const state = vramState(r);
    models.querySelector('.vram-short').textContent = state.percent == null ? '' : state.label;
  } catch { vram.update(null); }
  finally { readingResources = false; report(); }
}
setInterval(refreshResources, 1000);
setInterval(refresh, 1000); refresh();
if (new URLSearchParams(location.search).has('demo')) expand(true);

