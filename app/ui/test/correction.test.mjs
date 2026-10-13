import test from 'node:test';import assert from 'node:assert/strict';
import {correctionForm} from '../correction-core.js';
test('Korean corrections are normalized and can only be remembered with a captured program',()=>{
  assert.deepEqual(correctionForm({program:'eden.exe'},'  스킵  '),{text:'스킵',error:'',disabled:false,canRemember:true});
  assert.equal(correctionForm({program:''},'스킵').canRemember,false);
});
test('empty oversized missing and busy correction forms cannot be submitted',()=>{
  for(const text of ['','...','가'.repeat(4097)])assert.equal(correctionForm({program:'game.exe'},text).disabled,true);
  assert.equal(correctionForm(null,'스킵').disabled,true);
  assert.equal(correctionForm({program:'game.exe'},'스킵',true).disabled,true);
});
