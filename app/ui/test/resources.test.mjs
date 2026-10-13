import {test} from 'node:test';
import assert from 'node:assert/strict';
import {vramState,bytesText,percentText,processCells} from '../resources-core.js';
const gpu = (used,total=100) => ({name:'GPU',dedicated_used_bytes:used,dedicated_total_bytes:total});
const state = gpus => ({enabled:true,sample:{at_ms:1000,gpus}});
test('quick VRAM viewing marks high and critical occupancy and includes all applications',()=>{
  assert.equal(vramState(state([gpu(79)]),1000).level,'normal');
  assert.equal(vramState(state([gpu(80)]),1000).level,'high');
  const critical=vramState(state([gpu(95)]),1000);
  assert.equal(critical.level,'critical');assert.equal(critical.percent,95);assert.match(critical.detail,/all apps/);
  assert.equal(vramState(state([gpu(101)]),1000).percent,100);
});
test('multiple GPUs show the greatest memory pressure rather than combining different capacities',()=>{
  const s=vramState(state([gpu(10,16),{...gpu(7.8,8),name:'Second GPU'}]),1000);
  assert.equal(s.percent,98);assert.match(s.detail,/Second GPU/);
});
test('paused stale unsupported and malformed samples never appear as zero VRAM usage',()=>{
  assert.equal(vramState(null,1000).label,'VRAM unavailable');
  assert.equal(vramState({enabled:false},1000).label,'VRAM monitoring paused');
  for(const s of [null,{enabled:false},state([]),state([gpu(null)]),state([gpu(-1)]),state([gpu(1,0)]),state([gpu(NaN)])])assert.equal(vramState(s,1000).percent,null);
  assert.equal(vramState(state([gpu(90)]),5000).percent,null);
  assert.equal(vramState(state([gpu(0)]),1000).percent,0);
});
test('unloaded processes show unavailable readings while actual measured zero remains zero',()=>{
  assert(processCells({}).every(v=>v==='Unavailable'));
  assert.equal(percentText(0),'0.0%');assert.equal(bytesText(0),'0 MiB');
  assert.equal(bytesText(1024**3),'1.00 GiB');assert.equal(bytesText(null),'Unavailable');
});
