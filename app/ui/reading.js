import { invoke } from './bridge.js';
import { selectedArea, readingText, readingControls, readingBannerVisible } from './reading-core.js';
import { installReadingDemo } from './reading-demo.js';

function element(tag, text, className='') { const el=document.createElement(tag);if(text!=null)el.textContent=text;if(className)el.className=className;return el; }
function button(text, action) { const el=element('button',text);el.type='button';el.onclick=action;return el; }

export function readingQuickControls({ beforeCapture=async()=>{}, manage, changed=()=>{}, banner=false }={}) {
  const root=element('section',null,`reading-controls${banner?' reading-banner':''}`);
  root.setAttribute('aria-label','Saved reading area');
  const title=element('strong','Saved reading area'), state=element('p',null,'reading-state'), message=element('p',null,'reading-message');
  state.setAttribute('role','status');message.setAttribute('role','status');
  const select=element('select');select.setAttribute('aria-label','Saved reading area');select.hidden=banner;
  let last=null,busy=false;
  const actions=element('div',null,'reading-buttons');
  const run=async(command,args={},capture=false)=>{
    if(busy)return;busy=true;paint();
    try {if(capture)await beforeCapture();await invoke(command,args);last=await invoke('reading_status');}
    catch(e){message.textContent=String(e);message.dataset.error='true';}
    finally{busy=false;paint();changed();}
  };
  const read=button('Read again',()=>run('reading_read',{},true));
  const auto=button('Auto-read',()=>run('reading_auto',{enabled:!last?.automatic}));
  const hold=button('Hold',()=>run('reading_hold',{held:!last?.held}));
  auto.setAttribute('aria-pressed','false');hold.setAttribute('aria-pressed','false');
  actions.append(read,auto,hold);
  if(manage)actions.append(button('Manage areas',manage));
  root.append(title,select,state,actions,message);
  select.onchange=()=>run('reading_select',{areaId:Number(select.value)});
  function paint() {
    const area=selectedArea(last), controls=readingControls(last,busy);
    root.dataset.phase=last?.phase??'ready';
    root.hidden=banner&&!readingBannerVisible(last);
    title.textContent=area?`${last.game??area.game} · ${area.name}`:'Saved reading areas';
    state.textContent=readingText(last);
    const options=(last?.areas??[]).filter(a=>a.exe===last?.exe);
    const key=JSON.stringify(options.map(a=>[a.id,a.name]));
    if(select.dataset.key!==key){select.replaceChildren(...options.map(a=>{const o=element('option',a.name);o.value=a.id;return o;}));select.dataset.key=key;}
    select.hidden=banner||options.length<2;select.value=area?.id??'';select.disabled=busy;
    read.disabled=controls.readDisabled;auto.disabled=controls.autoDisabled;hold.disabled=controls.holdDisabled;hold.hidden=!last?.automatic;
    auto.setAttribute('aria-pressed',String(controls.automatic));hold.setAttribute('aria-pressed',String(controls.held));hold.textContent=controls.held?'Resume':'Hold';
    if(!message.dataset.error)message.textContent=last?.message??'';
  }
  root.update=s=>{last=s;delete message.dataset.error;paint();changed();};
  root.refresh=async()=>{try{root.update(await invoke('reading_status'));}catch{root.hidden=banner;}};
  paint();return root;
}

export async function readingView() {
  installReadingDemo();
  const root=element('div',null,'reading-manager');
  root.append(element('h2','Saved reading areas'),element('p','Save a dialogue box, item description, or journal once. Read the whole area again with full translation, word meanings, and deep dive.', 'help'));
  const quick=readingQuickControls(), feedback=element('p',null,'err');feedback.setAttribute('role','alert');
  const create=element('form',null,'reading-create'),label=element('label');label.append(element('span','New area name'));
  const input=element('input');input.value='Dialogue';input.maxLength=64;input.required=true;input.setAttribute('aria-label','New area name');label.append(input);
  const save=button('Choose area in game',()=>{});save.type='submit';create.append(label,save);
  root.append(quick,element('p','Start from the floating controls while your game is visible. Areas follow window movement. After a size or resolution change, check the scaled area or reposition it. Auto-read starts off every time Maru opens.', 'help'),create,feedback);
  const list=element('div');root.append(list);
  let last=null,disposed=false,polling=false,busy=false,listKey='';
  async function run(command,args) {
    if(busy)return;busy=true;save.disabled=true;feedback.textContent='';
    try {await invoke(command,args);if(list.contains(document.activeElement))document.activeElement.blur();listKey='';await refresh();}catch(e){feedback.textContent=String(e);}finally{busy=false;save.disabled=!last?.can_create;}
  }
  create.onsubmit=e=>{e.preventDefault();run('reading_create',{name:input.value,areaId:null});};
  function row(area) {
    const el=element('article',null,`reading-area${area.id===last.selected?' active':''}`),header=element('header');
    header.append(element('strong',area.name),element('span',area.id===last.selected?'Selected':''));
    el.append(header,element('p',`${area.game} · ${area.exe} · saved at ${Math.round(area.reference_width)} × ${Math.round(area.reference_height)}`,'meta'));
    const controls=element('div',null,'reading-buttons');
    const use=button('Select',()=>run('reading_select',{areaId:area.id}));use.disabled=area.id===last.selected;controls.append(use);
    controls.append(button('Reposition',()=>run('reading_create',{name:area.name,areaId:area.id})),button('Rename',()=>{
      const edit=element('input');edit.value=area.name;edit.maxLength=64;edit.setAttribute('aria-label','Area name');
      const saveName=()=>run('reading_rename',{areaId:area.id,name:edit.value});
      edit.onkeydown=e=>{if(e.key==='Enter'){e.preventDefault();saveName();}if(e.key==='Escape'){e.preventDefault();listKey='';drawList(true);}};
      controls.replaceChildren(edit,button('Save name',saveName),button('Cancel',()=>{listKey='';drawList(true);}));edit.focus();edit.select();
    }),button('Remove',()=>{
      const remove=button('Remove area',()=>run('reading_remove',{areaId:area.id}));remove.className='danger';
      controls.replaceChildren(element('span',`Remove “${area.name}”?`),remove,button('Cancel',()=>{listKey='';drawList(true);}));
    }));
    if(area.id===last.selected&&last.position_check)controls.append(button('Check scaled area',()=>run('reading_create',{name:area.name,areaId:area.id})));
    el.append(controls);return el;
  }
  function drawList(force=false) {
    const key=JSON.stringify([last.areas,last.selected,last.position_check]);
    if(!force&&(key===listKey||list.contains(document.activeElement)))return;
    listKey=key;
    list.replaceChildren(...(last.areas.length?last.areas.map(row):[element('p','No areas saved yet. Open your game, use the floating menu to save an area, and name it here.','reading-empty')]));
  }
  async function refresh() {
    if(disposed||polling)return;polling=true;
    try {const s=await invoke('reading_status');if(disposed)return;last=s;quick.update(s);save.disabled=busy||!s.can_create;drawList();}catch(e){feedback.textContent=String(e);}finally{polling=false;}
  }
  await refresh();const timer=setInterval(()=>{if(!document.hidden)refresh();},1000);
  root.dispose=()=>{disposed=true;clearInterval(timer);};
  return [root];
}

export function readingPanel(changed) {
  if(new URLSearchParams(location.search).get('demo')?.startsWith('reading-'))installReadingDemo();
  const banner=readingQuickControls({banner:true,changed});
  setInterval(()=>{if(!document.hidden)banner.refresh();},1000);banner.refresh();return banner;
}
