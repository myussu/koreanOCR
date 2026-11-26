import {connectionFields} from './settings-connection-core.js';
import {selectedServer,serverEdits,saveServer,removeServer} from './settings-servers-core.js';

// The picker sits above the address; saving sits below the test, after the connection has been filled in.

export function savedServerControls({section,value,edit}) {
  const root=document.createElement('div');root.className='saved-servers';
  const label=document.createElement('label');label.textContent='Saved connection';
  const select=document.createElement('select');select.setAttribute('aria-label','Saved server connection');label.append(select);
  const name=document.createElement('input');name.type='text';name.maxLength=64;name.placeholder='Short name: NAS, Azure…';name.setAttribute('aria-label','Connection name');
  const save=document.createElement('button');save.type='button';save.className='btn';save.textContent='Save as a connection';
  const error=document.createElement('span');error.className='res bad';error.setAttribute('role','alert');
  const remove=document.createElement('button');remove.type='button';remove.className='btn';remove.textContent='Remove';remove.title='Forget this saved connection (the settings below stay as they are)';label.append(remove);
  const help=document.createElement('p');help.className='help';help.textContent='Pick a saved connection to fill in its address, model, server type and key.';
  const saveHelp=document.createElement('p');saveHelp.className='help';saveHelp.textContent='Saves the address, model, server type and key above under this name, so you can switch back to it later. Saving an existing name updates it. Press Save at the bottom to apply; switching servers needs a restart.';
  const row=document.createElement('div');row.className='test-row';row.append(name,save,error);
  const saveBox=document.createElement('div');saveBox.className='saved-servers';saveBox.append(row,saveHelp);
  root.append(label,help);
  const fields=()=>connectionFields(section,value);
  let selectedName=null;
  select.onchange=()=>{
    const server=(value('llm.servers')??[]).find(server=>server.name===select.value);
    if(!server)return;
    selectedName=server.name;
    error.textContent='';
    for(const [path,next] of Object.entries(serverEdits(section,server)))edit(path,next);
    name.value=server.name;
  };
  remove.onclick=()=>{
    if(!select.value)return;
    edit('llm.servers',removeServer(value('llm.servers'),select.value));
    selectedName=null;name.value='';error.textContent='';
  };
  save.onclick=()=>{
    try{edit('llm.servers',saveServer(value('llm.servers'),name.value,fields()));error.textContent='';}
    catch(e){error.textContent=String(e);}
  };
  let namesKey='';
  const sync=()=>{
    const servers=value('llm.servers')??[],key=JSON.stringify(servers.map(server=>server.name));
    if(key!==namesKey||!select.children.length){namesKey=key;select.replaceChildren();const current=document.createElement('option');current.value='';current.textContent='Current connection / not saved';select.append(current);for(const server of servers){const option=document.createElement('option');option.value=server.name;option.textContent=server.name;select.append(option);}}
    const current=selectedServer(servers,fields(),selectedName);select.value=current?.name??'';
    save.disabled=!fields().endpoint||!fields().modelName;
    remove.disabled=!select.value;
  };
  sync();return {el:root,saveEl:saveBox,sync};
}
