import { modelActions, scanActions } from './launcher-core.js';
import { readingControls, readingText, selectedArea } from './reading-core.js';

const PAGES = [
  [
    ['region', 'Snapshot region', '▣', 'Choose an area and translate it with word explanations.'],
    ['reading_read', 'Read again', '↻', 'Read the selected saved area without drawing it again.'],
    ['reading_save', 'Save an area', '+', 'Choose and name a reading area for this game.'],
    ['reading_auto', 'Auto-read', '◉', 'Read the saved area when its text changes.'],
    ['reading_hold', 'Hold reading', 'Ⅱ', 'Keep the current reading open while you inspect it.'],
    ['scan', 'Label screen', '文', 'Toggle translated labels over the game screen.'],
    ['reading', 'Reading areas', '▤', 'Select, rename or reposition saved reading areas.'],
  ],
  [
    ['continuous', 'Live subtitles', '▶', 'Toggle the live subtitle translator.'],
    ['continuous_area', 'Subtitle area', '▣', 'Choose the area used by live subtitles.'],
    ['scan_join_mode', 'Join words', '⇄', 'Select screen labels to join into a sentence.'],
    ['scan_refresh', 'Refresh labels', '↻', 'Read and label the screen again.'],
    ['translate', 'Translate selection', 'A', 'Translate selected text in the current application.'],
    ['hover', 'Hover lookup', '◎', 'Toggle dictionary lookup under the pointer.'],
    ['speak', 'Read aloud', '♪', 'Translate the current selection and read it aloud.'],
  ],
  [
    ['settings', 'Settings', '⚙', 'Open Maru settings.'],
    ['history', 'History', '◷', 'Browse previous translations.'],
    ['review', 'Review words', '✓', 'Review your saved Korean words.'],
    ['resources', 'Resources', '▥', 'Inspect model resources and local diagnostic recording.'],
    ['prepare_live', 'Prepare live', '▶', 'Warm the selected live translator and OCR before playing.'],
    ['release_models', 'Release models', '↓', 'Unload idle models owned by Maru.'],
    ['status', 'App status', 'i', 'Inspect available services and setup.'],
  ],
];
export const WHEEL_PAGES = ['Reading', 'More tools', 'Maru'];

export function wheelActions(page, status, busy = false) {
  const at = Number.isInteger(page) && page >= 0 && page < PAGES.length ? page : 0;
  const reading = readingControls(status?.reading, busy), scan = scanActions(status, busy), models = modelActions(status, busy);
  return [...PAGES[at].map(([id, label, icon, detail]) => {
    let active = false, disabled = busy, reason = busy ? 'Another action is in progress.' : '';
    if (['region', 'translate', 'speak', 'continuous', 'continuous_area', 'scan', 'hover'].includes(id) && !status) {
      disabled = true; reason = 'Waiting for Maru status.';
    }
    if (id === 'reading_read' || id === 'reading_auto') {
      disabled = id === 'reading_read' ? reading.readDisabled : reading.autoDisabled;
      if (disabled && !busy) reason = status?.reading?.phase === 'choosing' ? 'Finish choosing the area first.' : 'Save or select a reading area first.';
    }
    if (id === 'reading_save') {
      disabled = reading.createDisabled;
      if (disabled && !busy) reason = status?.reading?.phase === 'choosing' ? 'Finish choosing the area first.' : 'Return to the game before saving an area.';
    }
    if (id === 'reading_auto') active = reading.automatic;
    if (id === 'reading_hold') {
      disabled = reading.holdDisabled; active = reading.held;
      if (reading.held) { label = 'Resume reading'; icon = '▶'; detail = 'Resume watching the saved area for new text.'; }
      if (disabled && !busy) reason = status?.reading?.phase === 'choosing' ? 'Finish choosing the area first.' : 'Turn Auto-read on first.';
    }
    if (id === 'scan') active = !!status?.scan;
    if (id === 'continuous') active = !!status?.live;
    if (id === 'hover') active = !!status?.hover;
    if (id === 'speak') active = !!status?.speech;
    if (id === 'scan_join_mode' || id === 'scan_refresh') {
      disabled = scan.disabled;
      if (id === 'scan_join_mode') active = scan.joining;
      if (disabled && !busy) reason = 'Turn Label screen on first.';
    }
    if (id === 'prepare_live' || id === 'release_models') {
      disabled = id === 'prepare_live' ? models.prepareDisabled : models.releaseDisabled;
      if (disabled && !busy) reason = !status ? 'Waiting for Maru status.' : status.live_setup?.busy ? 'Model preparation is in progress.' : 'Turn Live subtitles off and let its queued work finish first.';
    }
    return { id, label, icon, detail, active, disabled, reason, toggle: ['reading_auto', 'reading_hold', 'scan', 'continuous', 'hover', 'speak', 'scan_join_mode'].includes(id) };
  }), { id:'next_page', label:at === 2 ? 'Back to reading' : 'More controls', icon:'›', detail:`Show ${WHEEL_PAGES[(at + 1) % PAGES.length].toLowerCase()}.`, active:false, disabled:false, reason:'', toggle:false }];
}

export function nextWheelSelection(actions, current, step = 1) {
  if (!actions.length) return null;
  let at = Number.isInteger(current) && current >= 0 && current < actions.length ? current : step < 0 ? 0 : actions.length - 1;
  for (let n = 0; n < actions.length; n++) {
    at = (at + (step < 0 ? -1 : 1) + actions.length) % actions.length;
    if (!actions[at].disabled) return at;
  }
  return null;
}

export function wheelKey(key, actions, selected) {
  if (key === 'Escape') return { kind:'close' };
  if (['PageDown', ']'].includes(key)) return { kind:'page', step:1 };
  if (['PageUp', '['].includes(key)) return { kind:'page', step:-1 };
  if (['ArrowRight', 'ArrowDown', 'ArrowLeft', 'ArrowUp'].includes(key)) {
    const index = nextWheelSelection(actions, selected, ['ArrowLeft', 'ArrowUp'].includes(key) ? -1 : 1);
    return index == null ? null : { kind:'select', index };
  }
  const index = /^[1-8]$/.test(key) ? Number(key) - 1 : ['Enter', ' '].includes(key) ? selected : null;
  if (index == null || !actions[index]) return null;
  return { kind:actions[index].disabled ? 'unavailable' : 'activate', index };
}

export function wheelReadingSummary(status) {
  const area = selectedArea(status?.reading);
  return area ? { title:area.name, detail:readingText(status.reading) } : { title:'No saved area selected', detail:'Choose Save an area to create one.' };
}
