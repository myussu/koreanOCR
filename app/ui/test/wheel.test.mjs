import { test } from 'node:test';
import assert from 'node:assert/strict';
import { wheelActions, nextWheelSelection, wheelKey, wheelReadingSummary, WHEEL_PAGES } from '../wheel-core.js';

const status = () => ({live:false,scan:false,scan_id:null,queued:0,active:0,reading:{areas:[{id:1,name:'Dialogue'}],selected:1,can_create:true,phase:'ready',automatic:false,held:false}});

test('saved reading actions occupy a single wheel page with an always accessible next page',()=>{
  const entries=wheelActions(0,status());
  assert.equal(entries.length,8);
  assert.deepEqual(entries.map(a=>a.id),['region','reading_read','reading_save','reading_auto','reading_hold','scan','reading','next_page']);
  assert(entries.find(a=>a.id==='reading_hold').disabled);
  assert(!entries.at(-1).disabled);
});
test('a wheel offers every floating action without changing its existing command ID',()=>{
  const ids=WHEEL_PAGES.flatMap((_,page)=>wheelActions(page,status()).map(a=>a.id));
  for(const id of ['region','translate','scan','scan_join_mode','scan_refresh','continuous','continuous_area','hover','speak','history','settings','resources','status','review','prepare_live','release_models','reading'])assert(ids.includes(id));
  assert.equal(new Set(ids.filter(id=>id!=='next_page')).size,21);
});
test('reading toggles display their actual state and hold becomes resume',()=>{
  const s=status();s.reading.automatic=true;s.reading.held=true;
  const entries=wheelActions(0,s),hold=entries.find(a=>a.id==='reading_hold');
  assert.equal(hold.label,'Resume reading');assert(hold.active);assert(!hold.disabled);
  assert(entries.find(a=>a.id==='reading_auto').active);
});
test('missing or choosing saved areas cannot launch reading and creation respects the game status',()=>{
  for(const reading of [null,{areas:[],selected:null},{...status().reading,phase:'choosing'}]) {
    const entries=wheelActions(0,{reading});
    for(const id of ['reading_read','reading_auto','reading_hold','reading_save'])assert(entries.find(a=>a.id===id).disabled);
  }
  const s=status();s.reading.can_create=false;
  assert(wheelActions(0,s).find(a=>a.id==='reading_save').disabled);
  assert(!wheelActions(0,s).find(a=>a.id==='reading_read').disabled);
});
test('join and refresh controls require the current visible scan ID',()=>{
  for(const s of [status(),{...status(),scan:true,scan_id:null},{...status(),scan:false,scan_id:4}]) {
    for(const id of ['scan_join_mode','scan_refresh'])assert(wheelActions(1,s).find(a=>a.id===id).disabled);
  }
  const entries=wheelActions(1,{...status(),scan:true,scan_id:4,scan_join_mode:true});
  assert(entries.find(a=>a.id==='scan_join_mode').active);
  assert(!entries.find(a=>a.id==='scan_refresh').disabled);
});
test('model preparation cannot overlap live work and busy actions cannot fire twice',()=>{
  const entries=wheelActions(2,{...status(),live:true,active:1});
  assert(entries.find(a=>a.id==='prepare_live').disabled);assert(!entries.find(a=>a.id==='release_models').disabled);
  for(let page=0;page<WHEEL_PAGES.length;page++) {
    const busy=wheelActions(page,status(),true);
    assert(busy.slice(0,7).every(a=>a.disabled));assert(!busy.at(-1).disabled);
  }
});
test('arrow navigation skips unavailable actions and wraps in either direction',()=>{
  const actions=[{disabled:false},{disabled:true},{disabled:true},{disabled:false}];
  assert.equal(nextWheelSelection(actions,0),3);assert.equal(nextWheelSelection(actions,3),0);
  assert.equal(nextWheelSelection(actions,0,-1),3);assert.equal(nextWheelSelection(actions,-1),0);
  assert.equal(nextWheelSelection([{disabled:true}],0),null);assert.equal(nextWheelSelection([],0),null);
  assert.deepEqual(wheelKey('ArrowRight',actions,0),{kind:'select',index:3});
  assert.deepEqual(wheelKey('ArrowUp',actions,0),{kind:'select',index:3});
});
test('number keys and enter activate only enabled choices while escape always closes',()=>{
  const actions=wheelActions(0,status());
  assert.deepEqual(wheelKey('2',actions,0),{kind:'activate',index:1});
  assert.deepEqual(wheelKey('5',actions,0),{kind:'unavailable',index:4});
  assert.deepEqual(wheelKey('Enter',actions,1),{kind:'activate',index:1});
  assert.deepEqual(wheelKey(' ',actions,1),{kind:'activate',index:1});
  assert.deepEqual(wheelKey('Escape',[],null),{kind:'close'});
  assert.equal(wheelKey('9',actions,0),null);assert.equal(wheelKey('Enter',actions,null),null);
});
test('keyboard page changes and unknown page input stay inside the known wheel pages',()=>{
  assert.deepEqual(wheelKey('PageDown',[],null),{kind:'page',step:1});
  assert.deepEqual(wheelKey('[',[],null),{kind:'page',step:-1});
  assert.deepEqual(wheelActions(10,status()),wheelActions(0,status()));
});
test('a selected area summary reports its real name and watching state',()=>{
  assert.equal(wheelReadingSummary(null).title,'No saved area selected');
  const s=status();s.reading.phase='watching';s.reading.automatic=true;
  assert.equal(wheelReadingSummary(s).title,'Dialogue');
  assert.match(wheelReadingSummary(s).detail,/Watching/);
});
