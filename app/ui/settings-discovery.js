// Metadata search controls. Configuration writes remain the Settings page's Save action.
import {invoke} from './bridge.js';
import {connectionPaths, discoveryText, needsServerDetection} from './settings-connection-core.js';

const el=(tag,cls='',text='')=>{const node=document.createElement(tag);node.className=cls;node.textContent=text;return node;};
const button=(text,action)=>{const node=el('button','btn',text);node.type='button';node.addEventListener('click',action);return node;};
const option=(value,text)=>{const node=el('option','',text);node.value=value;return node;};

export function discoveryControls({section,state,fields,edit,refresh,disposed}) {
  const paths=connectionPaths(section);
  const root=el('div','server-discovery');
  const message=el('div','discovery-message');message.setAttribute('role','status');message.setAttribute('aria-live','polite');
  const servers=el('select');servers.setAttribute('aria-label','Discovered model servers');
  const models=el('select');models.setAttribute('aria-label','Models on selected server');
  const choices=el('div','discovery-choices');choices.append(servers,models);
  const detect=button('Detect server',()=>run('host',fields().endpoint).catch(()=>{}));
  const find=button('Find LAN servers',async()=>{
    state.showLan=true;state.error=null;state.loadingNetworks=true;refresh();
    try {state.networks=await invoke('discovery_networks');}
    catch(e){state.error=String(e);}
    finally{state.loadingNetworks=false;refresh();}
  });
  const cancel=button('Cancel search',()=>stop());
  const actions=el('div','test-row');actions.append(detect,find,cancel);
  const lan=el('div','discovery-lan');
  const networks=el('select');networks.setAttribute('aria-label','Local network to search');
  networks.addEventListener('change',()=>{state.network=networks.value;refresh();});
  const search=button('Search this network',()=>run('lan',networks.value).catch(()=>{}));
  lan.append(el('div','help','Choose a local network. Searches only ports 11434, 1234 and 8080; larger networks are limited to the shown /24 portion.'),networks,search);
  root.append(el('div','help','Enter an IP, hostname or full URL above. Detect server finds common ports and lists models without requesting a translation. Custom ports and routes are kept.'),actions,lan,message,choices);

  function selectServer(endpoint) {
    state.key=endpoint;state.selected=endpoint;
    edit(paths.endpoint,endpoint);
  }
  servers.addEventListener('change',()=>selectServer(servers.value));
  models.addEventListener('change',()=>{if(models.value)edit(paths.modelName,models.value);});

  function stop() {
    state.cancelRequested=true;
    if(state.jobId!=null)invoke('discovery_cancel',{jobId:state.jobId}).catch(()=>{});
  }
  state.stop=stop;

  async function run(kind,target) {
    if(state.busy || !target || disposed())return [];
    state.busy=true;state.cancelRequested=false;state.jobId=null;state.key=fields().endpoint;
    state.kind=kind;state.progress=null;state.error=null;state.servers=[];state.selected=null;refresh();
    try {
      const jobId=await invoke('discovery_start',{kind,target});state.jobId=jobId;
      if(state.cancelRequested || disposed())await invoke('discovery_cancel',{jobId});
      for(;;) {
        const progress=await invoke('discovery_status',{jobId});
        state.progress=progress;state.servers=progress.servers;refresh();
        if(progress.done)break;
        if(disposed() || state.cancelRequested)await invoke('discovery_cancel',{jobId});
        await new Promise(ok=>setTimeout(ok,180));
      }
      state.busy=false;
      if(state.cancelRequested || disposed() || state.progress.cancelled){if(kind==='host' || disposed())state.servers=[];return [];}
      if(kind==='host' && state.key===fields().endpoint && state.servers.length===1)selectServer(state.servers[0].endpoint);
      return state.servers;
    } catch(e){state.error=String(e);throw e;}
    finally{state.busy=false;state.jobId=null;refresh();}
  }

  async function resolveForTest() {
    if(!needsServerDetection(fields().endpoint))return;
    const found=await run('host',fields().endpoint);
    if(found.length!==1)throw Error(found.length ? 'Multiple servers found. Select one below, then test again.' : 'No server selected. Use Detect server or enter the complete URL.');
    if(found[0].models.length && !found[0].models.includes(fields().modelName))throw Error('Server found. Choose its translation model below, then test again.');
  }

  function sync() {
    const current=fields();
    if(state.key!=null && state.key!==current.endpoint){
      if(state.busy && state.kind==='host')stop();
      if(!state.busy){state.servers=[];state.progress=null;state.error=null;state.key=current.endpoint;state.selected=null;}
    }
    detect.disabled=state.busy || !current.endpoint;find.disabled=state.busy || state.loadingNetworks;
    cancel.hidden=!state.busy;cancel.disabled=state.cancelRequested;
    lan.hidden=!state.showLan;
    if(JSON.stringify(state.networks)!==networks.dataset.items){
      networks.replaceChildren(option('','Choose a network…'),...(state.networks??[]).map(n=>option(n.cidr,n.label)));
      networks.dataset.items=JSON.stringify(state.networks);networks.value=state.network??'';
    }
    search.disabled=state.busy || state.loadingNetworks || !networks.value;
    const list=state.servers??[];
    choices.hidden=!list.length;
    const signature=JSON.stringify(list);
    if(servers.dataset.items!==signature){servers.replaceChildren(option('','Choose a server…'),...list.map(s=>option(s.endpoint,`${s.endpoint} · ${s.api}`)));servers.dataset.items=signature;}
    servers.value=state.selected??'';
    const selected=list.find(s=>s.endpoint===state.selected);
    const modelKey=JSON.stringify(selected?.models??[]);
    if(models.dataset.items!==modelKey){models.replaceChildren(option('','Choose a model…'),...(selected?.models??[]).map(m=>option(m,m)));models.dataset.items=modelKey;}
    models.disabled=!selected || !selected.models.length;
    models.value=selected?.models.includes(current.modelName) ? current.modelName : '';
    message.classList.toggle('bad',!!state.error);
    message.textContent=state.error??(state.loadingNetworks ? 'Reading local network interfaces…' : state.showLan && state.networks?.length===0 ? 'No private IPv4 network found. You can still detect a server by address.' : discoveryText(state.progress));
    if(selected && !selected.models.length)message.textContent+=' No models were listed; enter a model name manually or load one on the server.';
    if(state.progress?.done && !list.length && !state.progress.cancelled && state.progress.errors?.length)message.textContent+=` ${state.progress.errors.join(' ')}`;
  }
  return {el:root,sync,resolveForTest};
}
