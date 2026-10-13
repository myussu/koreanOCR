import test from 'node:test';
import assert from 'node:assert/strict';
import { selectedArea, readingText, readingControls, readingBannerVisible, pickerArea } from '../reading-core.js';
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
