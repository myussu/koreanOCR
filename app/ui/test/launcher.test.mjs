import {test} from 'node:test';
import assert from 'node:assert/strict';
import {ACTIONS,liveDetails,modelActions,scanActions} from '../launcher-core.js';

test('joining and refresh controls appear only for an active screen scan and wait for its ID',()=>{
  assert.deepEqual(scanActions(null),{visible:false,disabled:true,joining:false});
  assert.deepEqual(scanActions({scan:true,scan_id:null}),{visible:true,disabled:true,joining:false});
  assert.deepEqual(scanActions({scan:true,scan_id:2,scan_join_mode:true}),{visible:true,disabled:false,joining:true});
  assert.deepEqual(scanActions({scan:false,scan_id:null,scan_join_mode:true}),{visible:false,disabled:true,joining:false});
  assert(scanActions({scan:true,scan_id:2},true).disabled);
});

test('the floating menu reports the selected live model while live capture is off',()=>{
  const s={live:false,model:'Gemma',model_status:'Ready',active:1,queued:3,live_setup:{model:'HY-MT2 1.8B Q4',device:'CPU',model_status:'Loads on demand',ocr:'CPU PP-OCR',ocr_status:'Ready',idle_reader_loaded:true,preparation:'Not prepared'}};
  const d=liveDetails(s);
  assert.match(d.model,/HY-MT2.*CPU.*Loads on demand/);
  assert.match(d.ocr,/CPU PP-OCR.*unused vision reader still loaded/);
  assert.equal(d.queue,'Queue: 1 translating · 3 waiting');
  assert.equal(d.message,'');
});
test('preparation waits for live capture and queued work while release remains available',()=>{
  assert.deepEqual(modelActions({live:true,queued:0,active:0}),{prepareDisabled:true,releaseDisabled:false});
  assert(modelActions({live:false,queued:1,active:0}).prepareDisabled);
  assert(modelActions({live:false,queued:0,active:1}).prepareDisabled);
  assert.deepEqual(modelActions({live:false,queued:0,active:0}),{prepareDisabled:false,releaseDisabled:false});
});
test('a model action prevents duplicate preparation and release but missing status is not fabricated',()=>{
  for(const s of [null,{live_setup:{busy:true}}])assert.deepEqual(modelActions(s),{prepareDisabled:true,releaseDisabled:true});
  assert.deepEqual(modelActions({},true),{prepareDisabled:true,releaseDisabled:true});
  assert.equal(liveDetails(null).ocr,'Live OCR: Unavailable');
  assert(ACTIONS.some(a=>a[0]==='prepare_live'));assert(ACTIONS.some(a=>a[0]==='release_models'));
});

test('the menu groups actions by what you are doing and shows each one its shortcut', async () => {
  const { GROUPS, shortcutText } = await import('../launcher-core.js');
  assert.deepEqual(GROUPS.map((g) => g.title), ['Live subtitles', 'Capture', 'Models and memory', 'Open']);
  const ids = GROUPS.flatMap((g) => g.actions.map((a) => a[0]));
  assert.equal(new Set(ids).size, ids.length, 'every action once');
  for (const id of ['continuous', 'continuous_area', 'star_line', 'region', 'translate', 'hover', 'scan', 'speak', 'prepare_live', 'release_models', 'history', 'settings']) assert.ok(ids.includes(id), id);
  assert.equal(shortcutText('Ctrl+Alt+T | F6'), 'Ctrl+Alt+T · F6');
  assert.equal(shortcutText(''), '');
  assert.equal(shortcutText(undefined), '');
});

test('each group names who translates for it, as Settings does', async () => {
  const { routeLine } = await import('../launcher-core.js');
  const s = { tasks: [
    { task: 'live', connection: 'NAS', model: 'gemma4-12b', place: 'network', state: 'working', message: 'NAS answered.' },
    { task: 'capture', connection: 'local', model: 'unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL', place: 'this_pc', state: 'not_loaded', message: "This PC's model loads when it is first used." },
    { task: 'card_sentence', connection: '', model: '', place: null, state: 'off', message: 'Cards keep the translation the line already has.' },
  ] };
  assert.deepEqual(routeLine(s, 'live'), { text: 'NAS · gemma4-12b', where: 'another computer', kind: 'ok', state: 'working', title: 'NAS answered.' });
  assert.equal(routeLine(s, 'capture').text, 'This PC · gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL');
  assert.equal(routeLine(s, 'capture').state, 'loads when used');
  assert.equal(routeLine(s, 'card_sentence').state, 'off');
  assert.equal(routeLine({}, 'live'), null, 'an older status without tasks shows nothing rather than a guess');
});
