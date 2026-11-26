import {test} from 'node:test';
import assert from 'node:assert/strict';
import {modelEdits} from '../settings-live-core.js';
import {withEdit} from '../settings-core.js';
import {ACTIONS,statusText,moved} from '../launcher-core.js';
test('choosing a live model on this PC stages its available file without overwriting a custom path',()=>{
  const files=[{model:'milmmt',path:'D:/approved.gguf'}];
  const edits=modelEdits('milmmt',{},files);
  assert.equal(edits['continuous.translation.milmmt_path'],'D:/approved.gguf');
  assert.deepEqual(modelEdits('milmmt',{'continuous.translation.milmmt_path':'E:/mine.gguf'},files),{'tasks.live.connection':'local','tasks.live.model':'milmmt'});
  const current={tasks:{live:{connection:'NAS',model:''}},continuous:{translation:{milmmt_path:null}}};
  let draft={};for(const [k,v]of Object.entries(edits))draft=withEdit(draft,current,k,v);
  assert.equal(current.tasks.live.connection,'NAS','nothing is applied until Save');
  assert.deepEqual([draft['tasks.live.connection'],draft['tasks.live.model']],['local','milmmt']);
});
test('choosing the full model clears the model name and unknown models stage nothing',()=>{
  assert.deepEqual(modelEdits('main',{},[]),{'tasks.live.connection':'local','tasks.live.model':''});
  assert.deepEqual(modelEdits('unknown',{},[]),{});
  assert.deepEqual(modelEdits('hy_mt2',{},[]),{'tasks.live.connection':'local','tasks.live.model':'hy_mt2'});
});
test('small model choices keep separate files and only stage the requested one',()=>{
  const files=[{model:'hy_mt2_small',path:'D:/small.gguf'},{model:'lfm',path:'D:/lfm.gguf'}];
  assert.deepEqual(modelEdits('hy_mt2_small',{},files),{'tasks.live.connection':'local','tasks.live.model':'hy_mt2_small','continuous.translation.hy_mt2_small_path':'D:/small.gguf'});
  assert.deepEqual(modelEdits('lfm',{},files),{'tasks.live.connection':'local','tasks.live.model':'lfm','continuous.translation.lfm_path':'D:/lfm.gguf'});
  assert.deepEqual(modelEdits('lfm',{'continuous.translation.lfm_path':'E:/custom.gguf'},files),{'tasks.live.connection':'local','tasks.live.model':'lfm'});
});
test('floating controls expose tray actions and give errors and loading priority over live status',()=>{
  for(const id of ['translate','region','hover','continuous','continuous_area','scan','speak','review','status','settings'])assert(ACTIONS.some(a=>a[0]===id));
  assert.equal(statusText({live:true,model_status:'Loading'}),'Loading translation model…');
  assert.equal(statusText({live:true,model_status:'Error: model missing',active:1}),'Error: model missing');
  assert.equal(statusText({live:false,active:1,queued:2}),'Translating · 2 waiting');
  assert.equal(statusText({live:false,active:0}),'Live subtitles off');
  assert(!moved({x:0,y:0},{x:3,y:2}));assert(moved({x:0,y:0},{x:6,y:0}));
});
