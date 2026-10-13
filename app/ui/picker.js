import { invoke, inTauri, mockCommands } from './bridge.js';
import { pickerArea } from './reading-core.js';

const box = document.getElementById('box');
const sizeTag = document.getElementById('size');
let start = null;
let preview = null;
let submitted = false;
if(!inTauri) {
  const mode=new URLSearchParams(location.search).get('demo');
  mockCommands({picker_session:()=>1,picker_ready:({sessionId})=>{window.pickerReady=sessionId;},reading_picker_hint:()=>mode==='reading-preview'?{name:'Dialogue',rect:{x:210,y:150,w:630,h:180},origin:{x:0,y:0}}:null,picker_done:args=>{window.pickerResult=args;(window.pickerResults??=[]).push(args);}});
}
const sessionId=await invoke('picker_session').catch(e=>{document.querySelector('#hint').textContent=String(e);return null;});
function finish(rect) {
  if(submitted||sessionId==null)return;submitted=true;start=null;
  invoke('picker_done',{rect,sessionId}).catch(e=>{submitted=false;document.querySelector('#hint').textContent=String(e);});
}
invoke('reading_picker_hint').then(hint=>{
  if(!hint||start||submitted)return;
  document.querySelector('#hint').textContent=hint.rect?`${hint.name} · Enter keeps this area · drag to reposition · Esc cancels`:`${hint.name} · drag around the text to save this area · Esc cancels`;
  preview=pickerArea(hint.rect,hint.origin,devicePixelRatio||1);if(preview)draw(preview);
}).catch(()=>{});

const rectOf = (a, b) => ({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) });

function draw(r) {
  Object.assign(box.style, { display: 'block', left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px` });
  Object.assign(sizeTag.style, { display: 'block', left: `${r.x}px`, top: `${Math.max(0, r.y - 20)}px` });
  sizeTag.textContent = `${Math.round(r.w)} × ${Math.round(r.h)}`;
}

addEventListener('mousedown', (e) => {
  if(e.button!==0||submitted||sessionId==null)return;
  preview = null;
  start = { x: e.clientX, y: e.clientY };
});
addEventListener('mousemove', (e) => start && draw(rectOf(start, { x: e.clientX, y: e.clientY })));
addEventListener('mouseup', (e) => {
  if (e.button!==0||!start) return;
  const r = rectOf(start, { x: e.clientX, y: e.clientY });
  start = null;
  const dpr = window.devicePixelRatio || 1;
  // A click without a drag cancels; very small boxes are accidents.
  if (r.w < 8 || r.h < 8) return finish(null);
  finish({ x: r.x * dpr, y: r.y * dpr, w: r.w * dpr, h: r.h * dpr });
});
addEventListener('keydown', (e) => e.key === 'Escape' && finish(null));
addEventListener('keydown',e=>{
  if(e.key!=='Enter'||!preview||start)return;
  const dpr=devicePixelRatio||1;
  finish({x:preview.x*dpr,y:preview.y*dpr,w:preview.w*dpr,h:preview.h*dpr});
});
addEventListener('contextmenu', (e) => {
  e.preventDefault();
  finish(null);
});
if(sessionId!=null)invoke('picker_ready',{sessionId}).catch(e=>{document.querySelector('#hint').textContent=String(e);});
