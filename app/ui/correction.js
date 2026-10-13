import {invoke,listen,inTauri,mockCommands} from './bridge.js';
import {correctionForm} from './correction-core.js';
const text=document.querySelector('#text'),remember=document.querySelector('#remember'),apply=document.querySelector('#apply'),message=document.querySelector('#message'),forget=document.querySelector('#forget');
let view=null,busy=false;
if(!inTauri){
  const mode=new URLSearchParams(location.search).get('demo');
  const sample={target:{kind:mode==='snapshot'?'snapshot':'scan',id:7,item:2},observed:'ㅅ컵',text:'ㅅ컵',program:mode==='unknown'?'':'eden.exe',remembered:mode==='saved'};
  mockCommands({correction_view:()=>sample,correction_apply:async args=>{window.__lastFix=args;await new Promise(r=>setTimeout(r,500));if(mode==='stale')throw Error('That snapshot is no longer current. Open the correction from the current snapshot.');message.textContent='Correction applied (browser demo).';},correction_forget:()=>{},correction_cancel:()=>{message.textContent='Cancelled (browser demo).';}});
}
function sync(){const f=correctionForm(view,text.value,busy);apply.disabled=f.disabled;remember.disabled=busy||!f.canRemember;forget.disabled=busy;text.disabled=busy;return f;}
function feedback(text,kind=''){message.textContent=text;message.className=kind;}
function show(next){view=next;busy=false;text.value=next.text;remember.checked=next.remembered;forget.hidden=!next.remembered;message.textContent='';document.querySelector('#observed').textContent=next.observed;document.querySelector('#program').textContent=next.program?`Captured program: ${next.program}`:'Program unknown — this correction can apply once.';sync();text.focus();}
text.addEventListener('input',()=>{feedback(sync().error,'bad');});
document.querySelector('#form').onsubmit=async e=>{e.preventDefault();if(busy)return;const f=sync();if(f.disabled){feedback(f.error,'bad');return;}busy=true;sync();feedback('Applying correction…');try{await invoke('correction_apply',{target:view.target,text:f.text,remember:remember.checked&&f.canRemember});feedback(inTauri?'Correction applied.':'Correction applied (browser demo).','good');}catch(e){feedback(String(e),'bad');}finally{busy=false;sync();}};
forget.onclick=async()=>{if(busy)return;busy=true;sync();try{await invoke('correction_forget',{target:view.target});forget.hidden=true;remember.checked=false;feedback('Saved rule forgotten. The current text is unchanged.','good');}catch(e){feedback(String(e),'bad');}finally{busy=false;sync();}};
document.querySelector('#cancel').onclick=()=>invoke('correction_cancel').catch(e=>{message.textContent=String(e);});
document.addEventListener('keydown',e=>{if(e.key==='Escape')invoke('correction_cancel');});
listen('maru-correction',show);
invoke('correction_view').then(show).catch(e=>{message.textContent=String(e);sync();});
