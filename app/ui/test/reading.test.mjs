import test from 'node:test';
import assert from 'node:assert/strict';
import { selectedArea, readingText, readingControls, readingBannerVisible, pickerArea, acceptsReadingStatus } from '../reading-core.js';
import { readingProgress, readingProgressView } from '../reading-progress-core.js';
const ready={areas:[{id:1,name:'Dialogue'}],selected:1,automatic:false,held:false,phase:'ready',can_create:true};
test('a saved area can be read manually while automatic reading is off',()=>{
  assert.equal(readingControls(ready).readDisabled,false);assert.equal(readingControls(ready).automatic,false);assert.equal(readingBannerVisible(ready),false);
});
test('an absent or deleted selection disables reading without selecting an unrelated area',()=>{
  assert.equal(selectedArea({...ready,selected:2}),null);assert.equal(readingControls({...ready,selected:2}).readDisabled,true);
});
test('hold is offered only for automatic reading and preserves the selected area',()=>{
  assert.equal(readingControls(ready).holdDisabled,true);const held={...ready,automatic:true,held:true,phase:'held'};
  assert.equal(readingControls(held).holdDisabled,false);assert.equal(readingControls(held).held,true);assert.equal(selectedArea(held).id,1);
});
test('pending dialogue explicitly identifies the answer below as the previous reading',()=>{
  assert.match(readingText({...ready,phase:'queued'}),/previous reading/);assert.match(readingText({...ready,phase:'settling'}),/previous reading/);
});
test('loading and failures remain visible even for a manual reading',()=>{
  assert.equal(readingBannerVisible({...ready,busy:true}),true);assert.equal(readingBannerVisible({...ready,phase:'error'}),true);assert.match(readingText({...ready,phase:'error'}),/Read again/);
});
test('an open picker or pending control prevents duplicate capture actions',()=>{
  assert.equal(readingControls({...ready,phase:'choosing'}).readDisabled,true);assert.equal(readingControls(ready,true).createDisabled,true);
});
test('a saved area preview respects high DPI and a monitor left of the primary display',()=>{
  assert.deepEqual(pickerArea({x:-1600,y:200,w:600,h:150},{x:-1920,y:0},2),{x:160,y:100,w:300,h:75});
  assert.equal(pickerArea(null,{x:0,y:0}),null);assert.equal(pickerArea({x:0,y:0,w:100,h:20},{x:0,y:0},0),null);
});
test('manual OCR capture disables a duplicate read while queued and model work permit a newer manual reading',()=>{
  assert(readingControls({...ready,phase:'capturing'}).readDisabled);
  for(const phase of ['queued','reading'])assert(!readingControls({...ready,phase,busy:true,pending:true}).readDisabled);
  assert(readingControls({...ready,phase:'reading'},true).readDisabled);
});
test('a status poll started before a newer pushed update is refused',()=>{
  assert(acceptsReadingStatus(4,4));assert(!acceptsReadingStatus(5,4));
});
test('progress begins before a translation exists and describes capture queue and model phases',()=>{
  let progress=null;
  for(const phase of ['capturing','queued','reading']) {
    progress=readingProgress(progress,{id:1,phase,area:'Dialogue',message:'Working'});
    const view=readingProgressView(progress);assert(view.visible);assert(view.busy);assert.equal(view.title,'Dialogue');assert.equal(view.message,'Working');
  }
  assert.equal(readingProgressView(null).visible,false);
});
test('an older operation error cannot replace a newer reading',()=>{
  const current=readingProgress(null,{id:2,phase:'capturing',area:'Journal',message:'New reading'});
  assert.equal(readingProgress(current,{id:1,phase:'error',area:'Dialogue',message:'Old failure'}),current);
});
test('older phase updates cannot replace current completion and errors remain until a newer operation',()=>{
  const complete=readingProgress(null,{id:1,phase:'done',area:'Dialogue'});
  assert.equal(readingProgress(complete,{id:1,phase:'reading'}),complete);
  const failed=readingProgress(complete,{id:1,phase:'error',message:'Incomplete response'});
  assert.equal(readingProgress(failed,{id:1,phase:'done'}),failed);
  assert.equal(readingProgressView(failed).busy,false);assert.equal(readingProgressView(failed).phase,'error');
  assert.equal(readingProgress(failed,{id:2,phase:'capturing'}).phase,'capturing');
});
test('unknown or malformed progress never erases a valid reading state',()=>{
  const current=readingProgress(null,{id:1,phase:'reading'});
  for(const update of [null,{id:NaN,phase:'error'},{id:1,phase:'unknown'},{id:-1,phase:'done'}])assert.equal(readingProgress(current,update),current);
  assert.equal(readingProgress(current,{id:2,phase:'capturing',area:{unsafe:true},message:{unsafe:true}}).message,'');
});
