import {test} from 'node:test';
import assert from 'node:assert/strict';
import {modelEdits,matchesSetting} from '../settings-live-core.js';
import {withEdit} from '../settings-core.js';
import {ACTIONS,statusText,moved} from '../launcher-core.js';
test('choosing a live model stages its available file without overwriting a custom path',()=>{
  const files=[{model:'milmmt',path:'D:/approved.gguf'}];
  const edits=modelEdits('milmmt',{},files);
  assert.equal(edits['continuous.translation.milmmt_path'],'D:/approved.gguf');
  assert.deepEqual(modelEdits('milmmt',{'continuous.translation.milmmt_path':'E:/mine.gguf'},files),{'continuous.translation.model':'milmmt'});
  const current={continuous:{translation:{model:'main',milmmt_path:null}}};
  let draft={};for(const [k,v]of Object.entries(edits))draft=withEdit(draft,current,k,v);
  assert.equal(current.continuous.translation.model,'main');
  assert.equal(draft['continuous.translation.model'],'milmmt');
});
test('choosing the main model keeps both test file paths and rejects unknown model IDs',()=>{
  assert.deepEqual(modelEdits('main',{},[]),{'continuous.translation.model':'main'});
  assert.deepEqual(modelEdits('unknown',{},[]),{});
  assert.deepEqual(modelEdits('hy_mt2',{},[]),{'continuous.translation.model':'hy_mt2'});
});
test('lightweight model choices keep separate files and only stage the requested option',()=>{
  const files=[{model:'hy_mt2_small',path:'D:/small.gguf'},{model:'lfm',path:'D:/lfm.gguf'}];
  assert.deepEqual(modelEdits('hy_mt2_small',{},files),{'continuous.translation.model':'hy_mt2_small','continuous.translation.hy_mt2_small_path':'D:/small.gguf'});
  assert.deepEqual(modelEdits('lfm',{},files),{'continuous.translation.model':'lfm','continuous.translation.lfm_path':'D:/lfm.gguf'});
  assert.deepEqual(modelEdits('lfm',{'continuous.translation.lfm_path':'E:/custom.gguf'},files),{'continuous.translation.model':'lfm'});
});
test('search finds help and config keys as well as setting labels',()=>{
  const f={label:'Snapshot server',help:'Sends text to your NAS',path:'continuous.translation.snapshot_endpoint'};
  assert(matchesSetting(f,'NAS'));assert(matchesSetting(f,'snapshot_endpoint'));assert(!matchesSetting(f,'anki'));
});
test('floating controls expose tray actions and give errors and loading priority over live status',()=>{
  for(const id of ['translate','region','hover','continuous','continuous_area','scan','speak','review','status','settings'])assert(ACTIONS.some(a=>a[0]===id));
  assert.equal(statusText({live:true,model_status:'Loading'}),'Loading translation model…');
  assert.equal(statusText({live:true,model_status:'Error: model missing',active:1}),'Error: model missing');
  assert.equal(statusText({live:false,active:1,queued:2}),'Translating · 2 waiting');
  assert.equal(statusText({live:false,active:0}),'Live subtitles off');
  assert(!moved({x:0,y:0},{x:3,y:2}));assert(moved({x:0,y:0},{x:6,y:0}));
});
