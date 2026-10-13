import { invoke, listen, inTauri, mockCommands } from './bridge.js';
import { readingProgress, readingProgressView } from './reading-progress-core.js';

const root=document.querySelector('#progress');
let progress=null,revision=0;
if(!inTauri) {
  const phase=new URLSearchParams(location.search).get('demo')??'capturing';
  const messages={capturing:'Recognizing the Korean text in this area.',queued:'Your reading has been captured and is waiting for the translator.',reading:'Word meanings and the full translation will appear in the reading panel.',done:'Open the reading panel for the translation and individual word explanations.',error:'The translation server did not respond. Try Read again; your previous reading remains available.'};
  mockCommands({reading_progress_ready:()=>{},reading_progress_status:()=>({id:1,area:'Hollow Knight · Dialogue',phase,message:messages[phase]??''})});
}
function update(next) {
  revision++;
  const accepted=readingProgress(progress,next);if(accepted===progress)return;progress=accepted;
  const view=readingProgressView(progress);root.hidden=!view.visible;root.dataset.phase=view.phase;root.dataset.busy=String(view.busy);root.setAttribute('aria-busy',String(view.busy));
  document.querySelector('#area').textContent=view.title;document.querySelector('#area').title=view.title;
  document.querySelector('#phase').textContent=view.label;document.querySelector('#message').textContent=view.message;document.querySelector('#message').title=view.message;
  document.querySelector('#symbol').textContent=view.busy?'':view.phase==='error'?'!':'✓';
}
async function refresh() {
  const request=revision;
  try{const next=await invoke('reading_progress_status');if(request===revision)update(next);}catch{}
}
await listen('maru-reading-progress',update);
invoke('reading_progress_ready').catch(()=>{});
refresh();
document.addEventListener('visibilitychange',()=>{if(!document.hidden)refresh();});
