// Status and actions for floating controls. No model-routing decisions live in the UI.
//
// The actions are grouped by what you are doing (live subtitles, a capture, the models, another window), and each
// says its shortcut, so the menu also teaches the keys. An action is [id, label, toggle field, shortcut name].
export const GROUPS = [
  { id: 'live', title: 'Live subtitles', task: 'live', actions: [['continuous', 'Live subtitles', 'live', 'continuous'], ['continuous_area', 'Choose the area', null, 'continuous_area'], ['star_line', 'Star this line', null, 'star']] },
  { id: 'capture', title: 'Capture', task: 'capture', actions: [['region', 'Snapshot a region', null, 'region'], ['translate', 'Translate selection', null, 'translate'], ['hover', 'Hover lookup', 'hover', 'hover'], ['scan', 'Label the screen', 'scan', 'scan'], ['scan_join_mode', 'Join words', 'scan_join_mode'], ['scan_refresh', 'Refresh labels'], ['speak', 'Read aloud', 'speech', 'speak']] },
  { id: 'models', title: 'Models and memory', actions: [['prepare_live', 'Prepare live session'], ['release_models', 'Release idle models']] },
  { id: 'open', title: 'Open', actions: [['history', 'History', null, 'history'], ['review', 'Review'], ['settings', 'Settings'], ['status', 'Status'], ['resources', 'Resources']] },
];
export const ACTIONS = GROUPS.flatMap((g) => g.actions);

/** A shortcut as a button shows it: every key, compact ("Ctrl+Alt+T · F6"); empty when none is set. */
export function shortcutText(keys) {
  return String(keys ?? '').split('|').map((k) => k.trim()).filter(Boolean).join(' · ');
}

const PLACE = { this_pc: 'this PC', network: 'another computer', hosted: 'hosted service' };
const STATE = { working: ['ok', 'working'], no_key: ['bad', 'no key'], quota: ['bad', 'quota exhausted'], unreachable: ['bad', 'unreachable'], failed: ['bad', 'not answering'], unknown: ['idle', 'not checked yet'], not_loaded: ['idle', 'loads when used'], off: ['idle', 'off'] };

/** Who a group's task asks now, from the `tasks` of the status: "NAS · gemma4-12b", where, and its state. */
export function routeLine(status, task) {
  const t = (status?.tasks ?? []).find((x) => x.task === task);
  if (!t) return null;
  if (t.state === 'off') return { text: t.message, where: '', kind: 'idle', state: 'off', title: t.message };
  const name = t.connection === 'local' ? 'This PC' : t.connection;
  const model = String(t.model ?? '').split(/[\\/]/).pop();
  const [kind, state] = STATE[t.state] ?? STATE.unknown;
  return { text: `${name} · ${model}`, where: PLACE[t.place] ?? '', kind, state, title: t.message };
}
export function statusText(s) {
  if (!s) return 'Connecting to Maru…';
  if (s.model_status?.startsWith('Error:')) return s.model_status;
  if (s.model_status === 'Loading') return 'Loading translation model…';
  if (s.active) return `Translating${s.queued ? ` · ${s.queued} waiting` : ''}`;
  return s.live ? 'Live subtitles on' : 'Live subtitles off';
}
export function moved(start, end) { return Math.hypot(end.x - start.x, end.y - start.y) > 5; }

export function liveDetails(s) {
  const setup=s?.live_setup;
  return {
    model: setup ? `Live translator: ${setup.model} · ${setup.device} · ${setup.model_status}` : `Selected: ${s?.model??'Unavailable'} · ${s?.model_status??'Unavailable'}`,
    ocr: setup ? `Live OCR: ${setup.ocr} · ${setup.ocr_status}${setup.idle_reader_loaded?' · unused vision reader still loaded':''}` : 'Live OCR: Unavailable',
    queue: `Queue: ${s?.active??0} translating · ${s?.queued??0} waiting`,
    preparation: setup?.preparation??'Not prepared',
    message: setup?.message??'',
  };
}
export function modelActions(s, busy=false) {
  const working=busy||!!s?.live_setup?.busy;
  return { prepareDisabled: !s||working||!!s.live||s.active>0||s.queued>0, releaseDisabled: !s||working };
}

export function scanActions(s, busy = false) {
  return { visible: !!s?.scan, disabled: busy || !s?.scan || s?.scan_id == null, joining: !!s?.scan && !!s?.scan_join_mode };
}
