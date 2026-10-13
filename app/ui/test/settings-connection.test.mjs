import {test} from 'node:test';
import assert from 'node:assert/strict';
import {connectionFields, connectionKey, canTestConnection, connectionResult, needsServerDetection, discoveryText, connectionPaths} from '../settings-connection-core.js';
import {shown} from '../settings-core.js';

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
