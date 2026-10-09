import { invoke } from './bridge.js';

const box = document.getElementById('box');
const sizeTag = document.getElementById('size');
let start = null;

const rectOf = (a, b) => ({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) });

function draw(r) {
  Object.assign(box.style, { display: 'block', left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px` });
  Object.assign(sizeTag.style, { display: 'block', left: `${r.x}px`, top: `${Math.max(0, r.y - 20)}px` });
  sizeTag.textContent = `${Math.round(r.w)} × ${Math.round(r.h)}`;
}

addEventListener('mousedown', (e) => {
  start = { x: e.clientX, y: e.clientY };
});
addEventListener('mousemove', (e) => start && draw(rectOf(start, { x: e.clientX, y: e.clientY })));
addEventListener('mouseup', (e) => {
  if (!start) return;
  const r = rectOf(start, { x: e.clientX, y: e.clientY });
  start = null;
  const dpr = window.devicePixelRatio || 1;
  // A click without a drag cancels; very small boxes are accidents.
  if (r.w < 8 || r.h < 8) return invoke('picker_done', { rect: null });
  invoke('picker_done', { rect: { x: r.x * dpr, y: r.y * dpr, w: r.w * dpr, h: r.h * dpr } });
});
addEventListener('keydown', (e) => e.key === 'Escape' && invoke('picker_done', { rect: null }));
addEventListener('contextmenu', (e) => {
  e.preventDefault();
  invoke('picker_done', { rect: null });
});
