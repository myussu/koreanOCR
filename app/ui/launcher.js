import { invoke, inTauri, mockCommands } from './bridge.js';
import { ACTIONS, statusText, moved, liveDetails, modelActions, scanActions } from './launcher-core.js';
import { makeVramBar } from './resources.js';
const button = document.querySelector('#maru'), menu = document.querySelector('#menu');
const vram = makeVramBar(); document.querySelector('#actions').before(vram);
let expanded = false, busy = false, drag = null, last = null;
if (!inTauri) {
  const state = { live: true, hover: false, scan: false, scan_id: null, scan_join_mode: false, speech: false, model: 'MiLMMT 4B Q6', model_status: 'Ready', queued: 0, active: 0, snapshot: 'Live model + offline meanings' };
  let nextScan = 0;
  state.live_setup={model:'HY-MT2 1.8B Q4',device:'GPU',model_status:'Ready',ocr:'Hybrid · CPU detector + PaddleOCR-VL',ocr_status:'Ready',preparation:'Not prepared',busy:false};
  const mode=new URLSearchParams(location.search).get('demo');
  if(mode==='queued'){state.active=1;state.queued=8;}
  if(mode==='cpu-ocr'){state.live_setup.ocr='CPU PP-OCR';state.live_setup.idle_reader_loaded=true;}
  mockCommands({ launcher_status: () => state, launcher_rects: () => {}, launcher_drag: () => {}, launcher_expand:()=>({up:false,left:false}), launcher_action: async ({ action }) => {
    const entry = ACTIONS.find(([id]) => id === action); if (entry?.[2]) state[entry[2]] = !state[entry[2]];
    if(action==='scan') { state.scan_id=state.scan?++nextScan:null;state.scan_join_mode=false; }
    if(action==='scan_refresh') { state.scan_id=++nextScan;state.scan_join_mode=false; }
    if(action==='prepare_live') {
      state.live_setup.busy=true;state.live_setup.preparation='Preparing translator…';state.live_setup.message=null;
      await new Promise(resolve=>setTimeout(resolve,1200));
      state.live_setup.busy=false;state.live_setup.preparation='Prepared';state.live_setup.message='Live models prepared. You can now turn live subtitles on.';
    }
    if(action==='release_models') {
      state.live_setup.preparation='Not prepared';state.live_setup.message=state.live?(state.live_setup.idle_reader_loaded?'Released idle OCR reader and speech; kept the live translator.':'Released idle speech; kept the live translator and OCR reader.'):'Released translator, OCR reader and speech.';
      state.live_setup.idle_reader_loaded=false;
    }
  } });
}
function report() {
  const rects = [button, ...(expanded ? [menu] : [])].map(el => { const r = el.getBoundingClientRect(); return { x:r.x, y:r.y, w:r.width, h:r.height }; });
  invoke('launcher_rects', { dpr: devicePixelRatio, rects }).catch(showError);
}
async function expand(value) {
  try {
    const layout=await invoke('launcher_expand',{expanded:value});
    expanded=value;menu.hidden=!value;button.setAttribute('aria-expanded',String(value));
    document.querySelector('#controls').classList.toggle('up',value&&layout.up);
    document.querySelector('#controls').classList.toggle('left',value&&layout.left);
    requestAnimationFrame(report);
    if (value) refreshResources();
  } catch(e) {showError(e);}
}
function showError(e) { document.querySelector('#error').textContent = String(e); }
async function action(id) {
  if (busy&&!['hide','quit'].includes(id)) return; busy=true; syncModelActions();
  document.querySelector('#error').textContent='';
  try { if (!['hide','quit','prepare_live','release_models'].includes(id)) await expand(false); await invoke('launcher_action', {action:id}); await refresh(); }
  catch(e) { expand(true); showError(e); } finally { busy=false; syncModelActions(); }
}
const actions = document.querySelector('#actions');
for(const [id,label,toggle] of ACTIONS) {
  const el=document.createElement('button'); el.textContent=label; el.dataset.action=id;
  if(toggle) el.setAttribute('aria-pressed','false');
  el.onclick=()=>action(id); actions.append(el);
}
async function refresh() {
  try {
    last=await invoke('launcher_status');
    document.querySelector('#state').textContent=statusText(last);
    const details=liveDetails(last);
    document.querySelector('#model').textContent=details.model;
    document.querySelector('#ocr').textContent=details.ocr;
    document.querySelector('#queue').textContent=details.queue;
    document.querySelector('#preparation').textContent=details.preparation;
    document.querySelector('#model-message').textContent=details.message;
    document.querySelector('#snapshot').textContent=`Snapshots: ${last.snapshot}`;
    if(last.shortcut)document.querySelector('#tip').textContent=`Drag 마 to move · ${last.shortcut} shows / hides`;
    document.querySelector('#dot').className=last.model_status.startsWith('Error:')||last.live_setup?.preparation==='Preparation failed'?'error':last.model_status==='Loading'||last.active||last.live_setup?.busy?'busy':last.live?'on':'';
    for(const [id,,toggle] of ACTIONS) if(toggle) actions.querySelector(`[data-action=${id}]`).setAttribute('aria-pressed',String(!!last[toggle]));
    syncModelActions();
    report();
  } catch(e) { showError(e); }
}
function syncModelActions() {
  const state=modelActions(last,busy);
  const prepare=actions.querySelector('[data-action=prepare_live]'), release=actions.querySelector('[data-action=release_models]');
  if(prepare) {
    prepare.disabled=state.prepareDisabled;
    prepare.title=last?.live?'Turn live off and let queued lines finish first.':'Warms the selected translator and OCR with fixed synthetic input. An external main translator receives a fixed Korean greeting.';
  }
  if(release) {
    release.disabled=state.releaseDisabled;
    release.title='Releases idle servers owned by Maru. Active requests and models needed by live capture stay loaded. External servers are unchanged.';
  }
  const scan=scanActions(last,busy);
  for(const id of ['scan_join_mode','scan_refresh']) {
    const control=actions.querySelector(`[data-action=${id}]`);
    control.hidden=!scan.visible;control.disabled=scan.disabled;
    control.title=id==='scan_join_mode'?'Select labels with left-click, then press Join. Click again to return to deep dive.':'Read and label the screen again.';
  }
}
button.addEventListener('pointerdown',async e=> {
  if(e.button!==0) return;
  drag={x:e.screenX,y:e.screenY,started:false}; button.setPointerCapture(e.pointerId);
  try { await invoke('launcher_drag',{phase:'start',dx:0,dy:0}); if(drag) drag.started=true; } catch(e) {drag=null;showError(e);}
});
let moveChain=Promise.resolve();
button.addEventListener('pointermove',e=>{
  if(!drag?.started||!moved(drag,{x:e.screenX,y:e.screenY})) return;
  const dx=e.screenX-drag.x,dy=e.screenY-drag.y;
  moveChain=moveChain.then(()=>invoke('launcher_drag',{phase:'move',dx,dy})).catch(showError);
});
async function endDrag(e) {
  const start=drag; if(!start)return; drag=null;
  const end={x:e.screenX,y:e.screenY};
  try { await moveChain; await invoke('launcher_drag',{phase:'end',dx:end.x-start.x,dy:end.y-start.y}); }
  catch(e){showError(e);}
  if(!moved(start,end)&&e.type!=='pointercancel')expand(!expanded);
}
button.addEventListener('pointerup',endDrag);button.addEventListener('pointercancel',endDrag);
button.addEventListener('click',e=>{if(e.detail===0)expand(!expanded);});
document.querySelector('#collapse').onclick=()=>expand(false);
document.querySelector('#hide').onclick=()=>action('hide');document.querySelector('#quit').onclick=()=>action('quit');
document.addEventListener('keydown',e=>{if(e.key==='Escape')expand(false);});
new ResizeObserver(report).observe(menu);
let readingResources=false;
async function refreshResources() {
  if(!expanded || document.hidden || readingResources)return;
  readingResources=true;
  try { vram.update(await invoke('resource_status')); } catch { vram.update(null); }
  finally { readingResources=false; report(); }
}
setInterval(refreshResources,1000);
setInterval(refresh,1000);refresh();
if(new URLSearchParams(location.search).has('demo'))expand(true);
