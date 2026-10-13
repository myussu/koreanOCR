const PHASES = { capturing:0, queued:1, reading:2, done:3, error:3 };
const LABELS = { capturing:'Reading saved area…', queued:'Waiting to translate…', reading:'Translating and explaining…', done:'Reading ready', error:'Could not read this area' };

export function readingProgress(previous, update) {
  if(!update||!Number.isSafeInteger(update.id)||update.id<0||!Object.hasOwn(PHASES,update.phase))return previous;
  if(previous&&update.id<previous.id)return previous;
  if(previous&&update.id===previous.id) {
    if(previous.phase==='error'&&update.phase!=='error')return previous;
    if(PHASES[update.phase]<PHASES[previous.phase])return previous;
  }
  return {id:update.id,area:typeof update.area==='string'?update.area:'Saved reading area',phase:update.phase,message:typeof update.message==='string'?update.message:''};
}
export function readingProgressView(progress) {
  return progress?{visible:true,busy:PHASES[progress.phase]<3,title:progress.area,label:LABELS[progress.phase],message:progress.message,phase:progress.phase}:{visible:false,busy:false,title:'',label:'',message:'',phase:'idle'};
}
