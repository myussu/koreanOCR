// Optional live models are staged as ordinary settings edits, never applied on a card click.
export const LIVE_MODELS = [
  { id:'main', name:'Gemma / main model', note:'Your existing setup. Full analysis; greatest GPU demand.', tag:'Current default' },
  { id:'hy_mt2', name:'HY-MT2 7B', note:'Q4_K_M · quality candidate for Korean dialogue.', tag:'Quality candidate', path:'continuous.translation.hy_mt2_path' },
  { id:'milmmt', name:'MiLMMT 4B', note:'Q6_K · lower memory use with fast translation.', tag:'Lighter candidate', path:'continuous.translation.milmmt_path' },
  { id:'hy_mt2_small', name:'HY-MT2 1.8B', note:'Q4_K_M · smaller footprint; more scene-term errors.', tag:'Low memory', path:'continuous.translation.hy_mt2_small_path' },
  { id:'lfm', name:'LFM2 KoEn 1.2B', note:'Q5_K_M · smallest option; larger quality tradeoff.', tag:'Lowest memory', path:'continuous.translation.lfm_path' },
];
export function modelEdits(id, values, files) {
  const model=LIVE_MODELS.find(m=>m.id===id);
  if(!model) return {};
  const edits={'continuous.translation.model':id};
  if(model.path&&!values[model.path]) {
    const path=files?.find(f=>f.model===id)?.path;
    if(path)edits[model.path]=path;
  }
  return edits;
}
export function matchesSetting(field, query) {
  const q=query.trim().toLowerCase();
  return !q || `${field.label} ${field.help??''} ${field.path}`.toLowerCase().includes(q);
}
export const LIVE_FIELDS = [
  {path:'continuous.translation.model',label:'Live translation model',type:'select',options:LIVE_MODELS.map(m=>({value:m.id,label:m.name}))},
  {path:'continuous.translation.hy_mt2_path',label:'HY-MT2 GGUF file',type:'text',nullable:true,placeholder:'D:/models/Hy-MT2-7B-Q4_K_M.gguf',when:{path:'continuous.translation.model',value:'hy_mt2'}},
  {path:'continuous.translation.milmmt_path',label:'MiLMMT GGUF file',type:'text',nullable:true,placeholder:'D:/models/MiLMMT-46-4B-v1.0.Q6_K.gguf',when:{path:'continuous.translation.model',value:'milmmt'}},
  {path:'continuous.translation.hy_mt2_small_path',label:'HY-MT2 1.8B GGUF file',type:'text',nullable:true,placeholder:'D:/models/Hy-MT2-1.8B-Q4_K_M.gguf',when:{path:'continuous.translation.model',value:'hy_mt2_small'}},
  {path:'continuous.translation.lfm_path',label:'LFM2 KoEn GGUF file',type:'text',nullable:true,placeholder:'D:/models/lfm2-1.2b-koen-mt-v8-rl-10k-merged-Q5_K_M.gguf',when:{path:'continuous.translation.model',value:'lfm'}},
  {path:'continuous.translation.device',label:'Run live translation on',type:'select',options:[{value:'gpu',label:'GPU — fastest'},{value:'cpu',label:'CPU — leaves more GPU for the game'}]},
  {path:'continuous.translation.cpu_threads',label:'Translation CPU threads',type:'number',min:1,max:32,step:1,when:{path:'continuous.translation.device',value:'cpu'}},
  {path:'continuous.translation.dialogue_context',label:'Experimental dialogue context',help:'Earlier answers sometimes improved pronouns but also introduced meaning errors in our tests. Off uses the native single-line prompt. The lightweight models have less room for long context.',type:'toggle'},
  {path:'continuous.translation.snapshot_endpoint',label:'Snapshot quality server',help:'Optional. While live is on, snapshot text and context are sent to this address for translation and analysis. If unavailable, use the local live translator and offline meanings. Blank keeps snapshots local. An IP or hostname works with Detect server below.',type:'text',nullable:true,placeholder:'192.168.18.217 or http://192.168.18.217:11434'},
  {path:'continuous.translation.snapshot_model_name',label:'Snapshot server model name',help:'Exact name expected by the server, such as the Gemma name from ollama list.',type:'text',placeholder:'maru'},
];
