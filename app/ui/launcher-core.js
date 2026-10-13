// Status and actions for floating controls. No model-routing decisions live in the UI.
export const ACTIONS = [
  ['continuous', 'Live subtitles', 'live'], ['continuous_area', 'Choose subtitle area'],
  ['prepare_live', 'Prepare live session'], ['release_models', 'Release idle models'],
  ['region', 'Snapshot a region'], ['translate', 'Translate selection'],
  ['hover', 'Hover lookup', 'hover'], ['scan', 'Label screen', 'scan'],
  ['scan_join_mode', 'Join words', 'scan_join_mode'], ['scan_refresh', 'Refresh labels'],
  ['speak', 'Translate and read aloud', 'speech'],
  ['history', 'History'], ['review', 'Review'], ['status', 'Status'], ['settings', 'Settings'],
  ['resources', 'Resources / recording'],
];
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
