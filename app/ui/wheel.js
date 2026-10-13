import { invoke, listen, inTauri, mockCommands } from './bridge.js';
import { wheelActions, nextWheelSelection, wheelKey, wheelReadingSummary, WHEEL_PAGES } from './wheel-core.js';
import { makeVramBar } from './resources.js';
import { installReadingDemo } from './reading-demo.js';

const actions = document.querySelector('#actions'), error = document.querySelector('#error');
const vram = makeVramBar(); document.querySelector('#vram').append(vram);
let page = 0, selected = 0, status = null, busy = false, refreshing = null, opened = true, generation = 0;
let entries = [], buttons = [];

if (!inTauri) {
  const reading = installReadingDemo();
  const demo = new URLSearchParams(location.search).get('demo');
  status = { reading, live:false, hover:false, scan:demo === 'labels', scan_id:demo === 'labels' ? 5 : null, scan_join_mode:false, speech:false, model_status:'Ready', queued:0, active:0, live_setup:{busy:false} };
  if (demo === 'busy') {status.active=1;status.queued=2;status.live=true;}
  if (demo === 'empty' || demo === 'no-game') {reading.areas=[];reading.selected=null;reading.can_create=demo!=='no-game';}
  if (demo === 'held') {reading.automatic=true;reading.held=true;reading.phase='held';}
  if (demo === 'watching') {reading.automatic=true;reading.phase='watching';}
  mockCommands({
    wheel_status:()=>structuredClone(status),
    wheel_ready:()=>{},
    wheel_close:()=>{opened=false;document.querySelector('#wheel').dataset.closed='true';document.querySelector('#selected').textContent='Returned to game';},
    wheel_action:async({action})=>{
      document.querySelector('#wheel').dataset.lastAction=action;
      if (demo === 'error' && action === 'reading_read') throw new Error('The game window changed size. Check or reposition the saved area.');
      const toggles = {continuous:'live',hover:'hover',scan:'scan',speak:'speech',scan_join_mode:'scan_join_mode'};
      if (toggles[action]) status[toggles[action]]=!status[toggles[action]];
      if (action === 'scan') {status.scan_id=status.scan?5:null;status.scan_join_mode=false;}
      if (action === 'reading_auto') {reading.automatic=!reading.automatic;reading.held=false;reading.phase=reading.automatic?'watching':'ready';}
      if (action === 'reading_hold') {reading.held=!reading.held;reading.phase=reading.held?'held':'watching';}
      if (action === 'reading_read' || action === 'prepare_live') await new Promise(r=>setTimeout(r,250));
      if (demo !== 'stay') {opened=false;document.querySelector('#wheel').dataset.closed='true';}
    },
  });
}

function showError(message) { error.textContent=String(message);error.hidden=false; }
function clearError() { error.textContent='';error.hidden=true; }
function describe() {
  const entry=entries[selected];
  document.querySelector('#selected').textContent=busy?'Working…':entry?.label??'Choose an action';
  document.querySelector('#selected').title=entry?.detail??'';
  const reading=wheelReadingSummary(status);
  document.querySelector('#area').textContent=reading.title;
  document.querySelector('#area').title=reading.detail;
}
function choose(index, focus = true) {
  if (!entries[index] || entries[index].disabled) return;
  selected=index;
  buttons.forEach((button,n)=>{button.classList.toggle('selected',n===selected);button.tabIndex=n===selected?0:-1;});
  describe();
  if (focus) buttons[selected]?.focus({preventScroll:true});
}
function draw(rebuild = false) {
  entries=wheelActions(page,status,busy);
  if (rebuild || !buttons.length) {
    buttons=entries.map((entry,index)=>{
      const button=document.createElement('button');button.type='button';button.className='action';button.dataset.index=index;button.dataset.action=entry.id;
      const number=document.createElement('span');number.className='number';number.textContent=index+1;number.setAttribute('aria-hidden','true');
      const icon=document.createElement('span');icon.className='icon';icon.setAttribute('aria-hidden','true');
      const label=document.createElement('span');label.className='label';
      const dot=document.createElement('i');dot.className='active-dot';dot.setAttribute('aria-hidden','true');
      button.append(number,icon,label,dot);
      button.addEventListener('pointerenter',()=>{if(opened&&!busy&&!entries[index]?.disabled){clearError();choose(index,false);}});
      button.addEventListener('focus',()=>choose(index,false));
      button.addEventListener('click',()=>activate(index));
      return button;
    });
    actions.replaceChildren(...buttons);
  }
  entries.forEach((entry,index)=>{
    const button=buttons[index];button.disabled=entry.disabled;
    button.querySelector('.icon').textContent=entry.icon;button.querySelector('.label').textContent=entry.label;
    button.querySelector('.active-dot').hidden=!entry.active;
    button.title=entry.disabled?entry.reason:entry.detail;
    button.setAttribute('aria-label',`${index+1}. ${entry.label}${entry.disabled?`. ${entry.reason}`:''}`);
    if(entry.toggle)button.setAttribute('aria-pressed',String(entry.active));else button.removeAttribute('aria-pressed');
  });
  document.querySelector('#page').textContent=`${WHEEL_PAGES[page]} · ${page+1}/${WHEEL_PAGES.length}`;
  actions.setAttribute('aria-label',`${WHEEL_PAGES[page]} actions`);
  if(busy) {describe();return;}
  const valid=entries[selected]&&!entries[selected].disabled?selected:nextWheelSelection(entries,-1);
  if(valid!=null)choose(valid,false);else describe();
}
function changePage(step = 1) {
  page=(page+step+WHEEL_PAGES.length)%WHEEL_PAGES.length;selected=0;clearError();draw(true);choose(nextWheelSelection(entries,-1)??0);
}
async function close() {
  const current=generation;
  try {await invoke('wheel_close');if(current===generation)opened=false;} catch(e) {if(current===generation)showError(e);}
}
async function activate(index) {
  if (busy || !opened) return;
  const entry=entries[index];if(!entry)return;
  if(entry.disabled) {showError(entry.reason);return;}
  if(entry.id==='next_page') {changePage();return;}
  choose(index,false);
  const current=generation;
  busy=true;clearError();draw();
  try {await invoke('wheel_action',{action:entry.id});if(current===generation)opened=false;}
  catch(e) {if(current===generation){showError(e);await refresh();}}
  finally {if(current===generation){busy=false;draw();if(opened)choose(selected);}}
}
async function refresh() {
  if(refreshing===generation||!opened||document.hidden)return;
  const current=generation;refreshing=current;
  try {
    const next=await invoke('wheel_status');if(!opened||current!==generation)return;status=next;draw();
    try {const counters=await invoke('resource_status');if(opened&&current===generation)vram.update(counters);} catch {if(current===generation)vram.update(null);}
  } catch(e) {if(current===generation)showError(e);}
  finally {if(refreshing===current)refreshing=null;}
}
document.addEventListener('keydown',e=>{
  if(e.altKey||e.ctrlKey||e.metaKey)return;
  if(e.target===document.querySelector('#close')&&['Enter',' '].includes(e.key))return;
  const decision=wheelKey(e.key,entries,selected);if(!decision)return;
  e.preventDefault();
  if(decision.kind==='close')close();
  if(busy)return;
  if(decision.kind==='page')changePage(decision.step);
  if(decision.kind==='select'){clearError();choose(decision.index);}
  if(decision.kind==='activate')activate(decision.index);
  if(decision.kind==='unavailable')showError(entries[decision.index].reason);
});
document.querySelector('#close').onclick=close;
error.onclick=clearError;
await listen('maru-wheel-show',()=>{generation++;opened=true;busy=false;page=0;selected=0;clearError();draw(true);refresh().then(()=>choose(nextWheelSelection(entries,-1)??0));});
document.addEventListener('visibilitychange',()=>{if(!document.hidden&&opened)refresh();});
draw(true);refresh().then(()=>choose(nextWheelSelection(entries,-1)??0));
invoke('wheel_ready').catch(showError);
setInterval(refresh,1000);
