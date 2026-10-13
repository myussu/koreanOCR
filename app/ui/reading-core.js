// Saved-area presentation only; the engine owns window matching and capture decisions.
export function selectedArea(s) { return s?.areas?.find(a => a.id === s.selected) ?? null; }
export function readingText(s) {
  if (!selectedArea(s)) return 'Save or select a reading area';
  if (s.position_check) return 'Window size changed · check the saved area';
  const states = { ready:'Ready · manual reading', watching:'Watching for new text', empty:'Dialogue cleared · previous reading stays available', settling:'New text settling · previous reading below', capturing:'Reading the area…', queued:'New text waiting · previous reading below', reading:'Translating and explaining…', held:'On hold · current answer stays open', paused:'Paused · return to the game or check its position', error:'Reading failed · try Read again', choosing:'Choose the area in the game' };
  return states[s.phase] ?? 'Ready';
}
export function readingControls(s, busy = false) {
  const selected = !!selectedArea(s), blocked = busy || s?.phase === 'choosing';
  return { readDisabled:!selected||blocked||s?.phase==='capturing', autoDisabled:!selected||blocked, holdDisabled:!s?.automatic||blocked, createDisabled:!s?.can_create||blocked, automatic:!!s?.automatic, held:!!s?.held };
}
export function acceptsReadingStatus(current, request) { return current === request; }
export function readingBannerVisible(s) { return !!selectedArea(s) && (!!s.automatic || !!s.busy || !!s.pending || ['capturing','error'].includes(s.phase)); }
export function pickerArea(rect,origin,dpr=1) {
  if(!rect||![rect.x,rect.y,rect.w,rect.h,origin?.x,origin?.y,dpr].every(Number.isFinite)||dpr<=0||rect.w<=0||rect.h<=0)return null;
  return {x:(rect.x-origin.x)/dpr,y:(rect.y-origin.y)/dpr,w:rect.w/dpr,h:rect.h/dpr};
}
