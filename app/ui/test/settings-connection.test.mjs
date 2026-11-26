import {test} from 'node:test';
import assert from 'node:assert/strict';
import {connectionFields, connectionKey, canTestConnection, connectionResult, needsServerDetection, discoveryText, connectionPaths} from '../settings-connection-core.js';
import {shown} from '../settings-core.js';
import {selectedServer,serverEdits,saveServer,azureParts,removeServer} from '../settings-servers-core.js';

test('a saved connection can be removed by name without touching the others', () => {
  const servers=[{name:'https://contoso.openai.azure.com/openai/deployments/gpt-4.1-mini',endpoint:'http://nas:11434',model_name:'gemma'},{name:'NAS',endpoint:'http://nas:11434',model_name:'gemma'}];
  assert.deepEqual(removeServer(servers,servers[0].name).map(s=>s.name),['NAS']);
  assert.equal(removeServer(servers,'missing').length,2);
});

test('a web address typed as the connection name is refused with what to do instead', () => {
  const nas={endpoint:'http://192.168.18.217:11434',modelName:'gemma4-12b-3060ti'};
  assert.throws(()=>saveServer([],'https://contoso.openai.azure.com/openai/deployments/gpt-4.1-mini',nas),/That is an address/);
  assert.equal(saveServer([],'NAS',nas)[0].name,'NAS');
});

test('a pasted Azure deployment URL is split into its address and deployment name', () => {
  assert.deepEqual(azureParts('https://contoso.openai.azure.com/openai/deployments/gpt-4.1-mini/chat/completions?api-version=2024-10-21'),{endpoint:'https://contoso.openai.azure.com',modelName:'gpt-4.1-mini'});
  assert.equal(azureParts('https://contoso.openai.azure.com'),null);
  assert.throws(()=>saveServer([],'Azure',{endpoint:'https://contoso.openai.azure.com/openai/deployments/gpt-4.1-mini',modelName:'x',provider:'azure',apiKey:'k'}),/use https:\/\/contoso\.openai\.azure\.com as the address and gpt-4\.1-mini/);
});

test('Gemini, like Azure, cannot be tested or saved without its API key', () => {
  const gemini={endpoint:'https://generativelanguage.googleapis.com/v1beta/openai',modelName:'gemini-2.5-flash',provider:'gemini',apiKey:null};
  assert(!canTestConnection(gemini));
  assert(canTestConnection({...gemini,apiKey:'k'}));
  assert.throws(()=>saveServer([],'Gemini',gemini),/Gemini API key/);
  assert.equal(saveServer([],'Gemini',{...gemini,apiKey:'k'})[0].provider,'gemini');
});

test('switching between azure and nas replaces credentials without changing the other translation lane',()=>{
  const nas={name:'NAS',endpoint:'http://nas:11434',model_name:'gemma',provider:'openai',api_key:null};
  const azure={name:'Azure',endpoint:'https://example.openai.azure.com/openai/v1/',model_name:'deployment',provider:'azure',api_key:'azure-secret'};
  const values={llm:{servers:[nas,azure],endpoint:nas.endpoint,model_name:nas.model_name},continuous:{translation:{snapshot_endpoint:'http://other',snapshot_model_name:'other'}}};
  let draft={...serverEdits('model',azure)};
  assert.equal(connectionFields('model',p=>shown(draft,values,{},p)).apiKey,'azure-secret');
  assert.equal(connectionFields('live',p=>shown(draft,values,{},p)).endpoint,'http://other');
  draft={...serverEdits('model',nas)};
  assert.equal(connectionFields('model',p=>shown(draft,values,{},p)).apiKey,undefined);
  assert.equal(selectedServer(values.llm.servers,connectionFields('model',p=>shown(draft,values,{},p))).name,'NAS');
});
test('registering many named connections does not select them and updating one preserves the rest',()=>{
  let servers=[];
  for(let n=0;n<50;n++)servers=saveServer(servers,`NAS ${n}`,{endpoint:`http://nas${n}:11434`,modelName:'gemma'});
  const before=servers;
  servers=saveServer(servers,'nas 2',{endpoint:'http://replacement:8080',modelName:'replacement'});
  assert.equal(servers.length,50);assert.equal(before[2].endpoint,'http://nas2:11434');assert.equal(servers[2].endpoint,'http://replacement:8080');assert.deepEqual(servers[3],before[3]);
});
test('a changed authentication key invalidates a previous successful check and azure requires one',()=>{
  const fields={endpoint:'https://example',modelName:'deployment',provider:'azure',apiKey:'secret',reasoningEffort:'none'};
  assert(!canTestConnection({...fields,apiKey:null}));assert(canTestConnection(fields));
  const state={key:connectionKey(fields),result:{kind:'ok',text:'Connected'}};
  assert.equal(connectionResult(state,{...fields,apiKey:'new-secret'}),null);
  assert.equal(connectionResult(state,{...fields,provider:'openai'}),null);
});
test('azure resource and canonical v1 addresses keep the same saved selection',()=>{
  const server={name:'Azure',endpoint:'https://example.openai.azure.com',model_name:'deployment',provider:'azure',api_key:'secret'};
  assert.equal(selectedServer([server],{endpoint:'https://example.openai.azure.com/openai/v1/',modelName:'deployment',provider:'azure',apiKey:'secret'}),server);
});
test('choosing one of two named aliases keeps that name while the connection values match',()=>{
  const servers=[{name:'NAS',endpoint:'http://nas:11434',model_name:'gemma'},{name:'TrueNAS',endpoint:'http://nas:11434',model_name:'gemma'}];
  assert.equal(selectedServer(servers,{endpoint:'http://nas:11434',modelName:'gemma'},'TrueNAS').name,'TrueNAS');
  assert.equal(selectedServer(servers,{endpoint:'http://elsewhere:8080',modelName:'gemma'},'TrueNAS'),null);
});

test('the main connection test uses unsaved endpoint and model fields without changing saved values',()=>{
  const values={llm:{endpoint:'http://old',model_name:'old',reasoning_effort:'none'}};
  const draft={'llm.endpoint':' http://new/v1 ', 'llm.model_name':'new'};
  const fields=connectionFields('model',p=>shown(draft,values,{},p));
  assert.deepEqual(fields,{endpoint:'http://new/v1',modelName:'new',reasoningEffort:'none'});
  assert.equal(values.llm.endpoint,'http://old');
});
test('the live snapshot server test keeps its own endpoint and model and shares the actual reasoning option',()=>{
  const values={'continuous.translation.snapshot_endpoint':'http://nas','continuous.translation.snapshot_model_name':'gemma','llm.endpoint':'http://other','llm.model_name':'other','llm.reasoning_effort':''};
  assert.deepEqual(connectionFields('live',p=>values[p]),{endpoint:'http://nas',modelName:'gemma',reasoningEffort:''});
});
test('a blank server or model cannot start a connection test',()=>{
  assert(!canTestConnection(connectionFields('model',()=>null)));
  assert(!canTestConnection({endpoint:'http://nas',modelName:''}));
  assert(canTestConnection({endpoint:'http://nas',modelName:'gemma'}));
});
test('a result from previous endpoint model or reasoning fields cannot confirm edited fields',()=>{
  const fields={endpoint:'http://nas',modelName:'gemma',reasoningEffort:'none'};
  const state={key:connectionKey(fields),result:{kind:'ok',text:'Connected'}};
  assert.equal(connectionResult(state,fields),state.result);
  for(const key of Object.keys(fields))assert.equal(connectionResult(state,{...fields,[key]:'changed'}),null);
});

test('bare hosts need detection while explicit ports and meaningful routes keep their destination',()=>{
  for(const address of ['192.168.18.217','nas.local','http://nas/'])assert(needsServerDetection(address));
  for(const address of ['nas:80','http://nas:11434','https://nas','http://nas/proxy/v1','[::1]:1234',''])assert(!needsServerDetection(address));
  assert.deepEqual(connectionPaths('live'),{endpoint:'continuous.translation.snapshot_endpoint',modelName:'continuous.translation.snapshot_model_name'});
});
test('discovery progress distinguishes metadata search cancellation and absence from inference success',()=>{
  assert.match(discoveryText({checked:12,total:762,done:false,servers:[]}),/12\/762/);
  assert.match(discoveryText({checked:12,total:762,cancelled:true,servers:[]}),/cancelled/);
  assert.match(discoveryText({checked:3,total:3,done:true,servers:[]}),/No model server/);
  assert.match(discoveryText({checked:3,total:3,done:true,servers:[{}]}),/Select a server and model/);
});
