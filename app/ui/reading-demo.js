import { inTauri, mockCommands } from './bridge.js';
let demo;
export function installReadingDemo() {
  if (inTauri || demo) return demo;
  const mode = new URLSearchParams(location.search).get('demo');
  const area = { id:1, exe:'hollow_knight.exe', game:'Hollow Knight', name:'Dialogue', region:{x:.2,y:.1,w:.6,h:.2}, reference_width:1920, reference_height:1080, selected:true };
  demo = { areas:[area,{...area,id:2,name:'Item descriptions',selected:false}], selected:1, game:'Hollow Knight', exe:area.exe, automatic:false, held:false, phase:'ready', message:null, pending:false, busy:false, can_create:true };
  if (mode==='reading-empty') {demo.areas=[];demo.selected=null;}
  if (mode==='reading-watch') {demo.automatic=true;demo.phase='watching';}
  if (mode==='reading-resized') {demo.automatic=true;demo.phase='paused';demo.position_check=true;demo.message='The game window changed size; check or reposition the saved area.';}
  if (mode==='reading-error') {demo.phase='error';demo.message='The translation server did not respond. Your previous reading remains available.';}
  mockCommands({
    reading_status:()=>structuredClone(demo),
    reading_select:({areaId})=>{demo.selected=areaId;const a=demo.areas.find(a=>a.id===areaId);demo.game=a.game;demo.exe=a.exe;demo.automatic=false;demo.held=false;demo.phase='ready';},
    reading_rename:({areaId,name})=>{if(demo.areas.some(a=>a.id!==areaId&&a.name.toLowerCase()===name.trim().toLowerCase()))throw new Error('An area with that name already exists');demo.areas.find(a=>a.id===areaId).name=name.trim();},
    reading_remove:({areaId})=>{demo.areas=demo.areas.filter(a=>a.id!==areaId);if(demo.selected===areaId){demo.selected=null;demo.automatic=false;demo.held=false;}},
    reading_create:({name,areaId})=>{if(areaId)return;const id=Math.max(0,...demo.areas.map(a=>a.id))+1;demo.areas.push({...area,id,name});demo.selected=id;},
    reading_auto:({enabled})=>{demo.automatic=enabled;demo.held=false;demo.phase=enabled?'watching':'ready';demo.message=null;},
    reading_hold:({held})=>{demo.held=held;demo.phase=held?'held':demo.automatic?'watching':'ready';},
    reading_confirm_position:()=>{demo.message=null;demo.position_check=false;demo.phase=demo.automatic?'watching':'ready';},
    reading_read:async()=>{demo.busy=true;demo.phase='capturing';demo.message=null;await new Promise(r=>setTimeout(r,700));demo.phase=demo.automatic?'watching':'ready';demo.busy=false;},
  });
  return demo;
}
