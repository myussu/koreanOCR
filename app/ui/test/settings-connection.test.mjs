import {test} from 'node:test';
import assert from 'node:assert/strict';
import {needsServerDetection, discoveryText, connectionPaths} from '../settings-connection-core.js';

test('bare hosts need to be looked for while explicit ports and meaningful routes keep their destination',()=>{
  for(const address of ['192.168.18.217','nas.local','http://nas/'])assert(needsServerDetection(address));
  for(const address of ['nas:80','http://nas:11434','https://nas','http://nas/proxy/v1','[::1]:1234',''])assert(!needsServerDetection(address));
  assert.deepEqual(connectionPaths(),{endpoint:'llm.endpoint',modelName:'llm.model_name'});
});
test('search progress tells a search in progress, a cancelled one, nothing found and something found apart',()=>{
  assert.match(discoveryText({checked:12,total:762,done:false,servers:[]}),/12\/762/);
  assert.match(discoveryText({checked:12,total:762,cancelled:true,servers:[]}),/cancelled/);
  assert.match(discoveryText({checked:3,total:3,done:true,servers:[]}),/No model server/);
  assert.match(discoveryText({checked:3,total:3,done:true,servers:[{}]}),/Choose the server and its model/);
});
