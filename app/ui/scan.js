// The screen-scan overlay: receives scan events, places labels with layout.js, draws them, and
// tells the app where they are so it can accept clicks only while the pointer is over one.

import { emitMock, invoke, inTauri, listen, mockCommands } from './bridge.js';
import { placeLabels } from './layout.js';
import { CHIP_H, chipFor, chipPlacement, labelClickAction, startsNewScan, clearsCurrentScan, acceptsJoinReply, unionRect } from './scan-core.js';
import { contextMenu } from './context-menu.js';
import { labelStatus, finishLabels } from './scan-status-core.js';
import { paintLabelStatus } from './scan-status.js';
import { canSeparate, nextJoinId, acceptsJoinFailure, joinedTreeIds } from './scan-core.js';

const layer = document.getElementById('layer');
const toast = document.getElementById('toast');
const params = new URLSearchParams(location.search);

const FONT = '12px "Segoe UI", system-ui, sans-serif';
const PAD = 18; // 7px padding each side, 1px border each side, 2px of slack for rounding
const HEIGHT = 18;

/** item id → { data: ScanItem, status, hidden: boolean, el: HTMLElement | null, parts?: number[] (the labels a joined one was made of) } */
const items = new Map();
/** Labels picked with Shift+click, to be joined (or one joined label, to be split). */
const selected = new Set();
/** A join the backend has not answered yet: new item id → the labels it absorbs (hidden meanwhile). */
const pending = new Map();
const reserved = new Set();
const separating = new Set();
let chip = null;
let chipRect = null;
let scanId = null;
let lastScanId = 0;
let joining = false;
let modeRevision = 0;
let origin = { x: 0, y: 0 }; // physical pixel of the window's top-left corner
let dpr = window.devicePixelRatio || 1;
let placements = new Map();
let frame = 0;
let reported = '';
const labelMenu = contextMenu(report);

const canvas = document.createElement('canvas').getContext('2d');
canvas.font = FONT;
const measure = (text) => Math.ceil(canvas.measureText(text).width) + PAD;

function labelText(it) {
  return it.status.text;
}

/** A scan item's text box, in the window's CSS pixels. */
function cssRect(r) {
  return { x: (r.x - origin.x) / dpr, y: (r.y - origin.y) / dpr, w: r.w / dpr, h: r.h / dpr };
}

function say(message, ms = 4500) {
  toast.textContent = message;
  toast.classList.add('show');
  clearTimeout(say.timer);
  say.timer = setTimeout(() => toast.classList.remove('show'), ms);
}

function clearAll() {
  joining = false;
  modeRevision = 0;
  for (const it of items.values()) it.el?.remove();
  items.clear();
  selected.clear();
  pending.clear();
  reserved.clear();
  separating.clear();
  chip?.remove();
  chip = null;
  chipRect = null;
  placements = new Map();
  reported = '';
  labelMenu.close();
  invoke('scan_labels', { scanId, dpr, rects: [] }).catch(() => {});
}

function schedule() {
  if (!frame) frame = requestAnimationFrame(render);
}

function render() {
  frame = 0;
  // The window may just have moved to a monitor with another scale factor.
  dpr = window.devicePixelRatio || 1;
  const forLayout = [];
  for (const [id, it] of items) {
    const text = it.hidden ? '' : labelText(it);
    forLayout.push({ id, rect: cssRect(it.data.rect), size: text ? { w: measure(text), h: HEIGHT } : null });
  }
  placements = placeLabels(forLayout, { bounds: { x: 0, y: 0, w: innerWidth, h: innerHeight }, maxWidth: 260, previous: placements });

  for (const [id, it] of items) {
    const p = placements.get(id);
    if (!p || it.hidden) {
      it.el?.remove();
      it.el = null;
      continue;
    }
    if (!it.el) it.el = makeLabel(id, it);
    const el = it.el;
    paintLabelStatus(el, it.status, it.data.text);
    el.className = `lab ${it.data.source}${it.data.known ? ' known' : ''}${p.mode === 'dot' ? ' dot' : ''}${selected.has(id) ? ' sel' : ''}`;
    el.style.left = `${p.x}px`;
    el.style.top = `${p.y}px`;
    // A shrunken label is the normal label scaled as a whole, so padding and border shrink with the text.
    el.style.width = `${p.w / p.scale}px`;
    el.style.height = `${p.h / p.scale}px`;
    el.style.transform = p.scale === 1 ? '' : `scale(${p.scale})`;
    el.title = [it.data.text, it.status.hint, `Left-click: ${joining ? 'select for joining' : 'deep dive'} · Right-click: correct OCR or hide`].filter(Boolean).join('\n');
  }
  updateChip();
  report();
}

function makeLabel(id, it) {
  const el = document.createElement('div');
  const t = document.createElement('span');
  t.className = 't';
  el.append(t);
  el.dataset.item = id;
  el.addEventListener('click', (e) => {
    labelMenu.close();
    if (labelClickAction(joining, e.shiftKey) === 'select') return toggleSelect(id);
    clearSelection();
    open(id);
  });
  el.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    const currentScan = scanId;
    const actions = [
      { label: 'Correct Korean OCR…', disabled: separating.has(id), action: () => invoke('correction_open_scan', { scanId: currentScan, item: id }).catch(e => say(String(e))) },
    ];
    if (canSeparate(it)) actions.push({ label: 'Separate joined label', disabled: separating.has(id), action: () => separate(id) });
    labelMenu.open({ x: e.clientX, y: e.clientY }, [
      ...actions,
      { label: 'Hide this label', action: () => hide(id) },
      { label: 'Cancel' },
    ]);
  });
  layer.append(el);
  return el;
}

function hide(id) {
  const it = items.get(id);
  if (!it) return;
  it.hidden = true;
  selected.delete(id);
  schedule();
}

// ───────────── joining labels the detector cut in two ─────────────

function applyJoinMode(enabled) {
  if (joining === enabled) return;
  joining = enabled;
  if (!joining) { clearSelection(); say('Left-click opens explanations again.', 2500); }
  else say('Join words: click each label, then press Join. Click Join words again to return to deep dive.', 6000);
  document.body.classList.toggle('joining', joining);
  schedule();
}

async function setJoinMode(enabled) {
  const id = scanId;
  if (id == null) return;
  const revision = modeRevision;
  try {
    const actual = await invoke('scan_join_mode', { scanId: id, enabled });
    if (acceptsJoinReply(scanId, id, modeRevision, revision)) { modeRevision++; applyJoinMode(actual); }
  } catch (error) { say(String(error)); }
}

function toggleSelect(id) {
  const it = items.get(id);
  if (!it || it.hidden) return;
  if (!selected.delete(id)) selected.add(id);
  if (selected.size === 1 && !canSeparate(it)) say(joining ? 'Click the other labels of the sentence, then press Join.' : 'Shift+click the other label(s) of the same text, then press Join.', 3200);
  schedule();
}

function clearSelection() {
  if (!selected.size) return;
  selected.clear();
  schedule();
}

/** The picked labels that are still on screen, in the order they were picked. */
function picked() {
  return [...selected].map((id) => ({ id, it: items.get(id) })).filter((x) => x.it && !x.it.hidden);
}

function updateChip() {
  const sel = picked().filter((x) => placements.has(x.id));
  const kind = chipFor(sel.map((x) => x.it));
  if (!kind) {
    chip?.remove();
    chip = null;
    chipRect = null;
    return;
  }
  if (!chip) {
    chip = document.createElement('div');
    chip.id = 'chip';
    const main = document.createElement('button');
    main.type = 'button';
    main.className = 'main';
    main.addEventListener('click', () => (chip.dataset.kind === 'split' ? doSplit() : doJoin()));
    const word = document.createElement('button');
    word.type = 'button';
    word.className = 'word';
    word.textContent = 'One word';
    word.title = 'Combine OCR fragments without adding spaces';
    word.addEventListener('click', () => doJoin(true));
    const cancel = document.createElement('button');
    cancel.type = 'button';
    cancel.className = 'cancel';
    cancel.textContent = '×';
    cancel.setAttribute('aria-label', 'Clear the selection');
    cancel.addEventListener('click', () => { clearSelection(); if (joining) setJoinMode(false); });
    chip.append(main, word, cancel);
    layer.append(chip);
  }
  chip.dataset.kind = kind.kind;
  chip.firstChild.textContent = kind.kind === 'split' ? 'Separate' : `Join ${kind.n}`;
  chip.firstChild.title = kind.kind === 'split' ? 'Restore original labels and forget their saved joins' : 'Join with spaces between selected labels';
  chip.querySelector('.word').hidden = kind.kind !== 'join';
  const union = unionRect(sel.map((x) => ({ x: placements.get(x.id).x, y: placements.get(x.id).y, w: placements.get(x.id).w, h: placements.get(x.id).h })));
  chipRect = chipPlacement(union, { w: chip.offsetWidth || 96, h: CHIP_H }, { w: innerWidth, h: innerHeight });
  chip.style.left = `${chipRect.x}px`;
  chip.style.top = `${chipRect.y}px`;
}

function restore(ids) {
  for (const id of ids) {
    const it = items.get(id);
    if (it) {
      it.hidden = false;
      delete it.absorbedBy;
    }
  }
}

/** The picked labels become one: they disappear now and the backend sends the joined one (translated as a whole). */
function doJoin(asWord = false) {
  const sel = picked();
  if (sel.length < 2 || scanId == null) return;
  const currentScan = scanId;
  const item = nextJoinId(items.keys(), pending.keys(), reserved);
  for (const { it } of sel) {
    it.hidden = true;
    it.absorbedBy = item;
  }
  const parts = sel.map((x) => x.id);
  pending.set(item, parts);
  selected.clear();
  invoke('scan_join_items', { scanId: currentScan, item, items: parts, asWord }).catch((e) => {
    if (!acceptsJoinFailure(scanId, currentScan, pending.get(item), parts)) return;
    say(String(e));
    restore(pending.get(item) ?? []);
    pending.delete(item);
    schedule();
  });
  schedule();
}

/** A joined label goes back to the pieces it was made of, and the join is forgotten. */
function doSplit() {
  const [only] = picked();
  if (only && canSeparate(only.it)) separate(only.id);
}

async function separate(id) {
  const currentScan = scanId;
  if (currentScan == null || separating.has(id)) return;
  separating.add(id);
  try {
    await invoke('scan_separate', { scanId: currentScan, item: id });
    if (scanId === currentScan) say('Separated the original labels. This join will not return on the next scan.');
  } catch (error) { if (scanId === currentScan) say(String(error)); }
  finally { if (scanId === currentScan) { separating.delete(id); schedule(); } }
}

function open(id) {
  const it = items.get(id);
  if (it) invoke('scan_open_item', { scanId, item: id }).catch((e) => say(String(e)));
}

/** Where the labels are, so the app can accept clicks only while the pointer is over one. */
function report() {
  const rects = [];
  for (const [id, it] of items) {
    const p = placements.get(id);
    if (!p || it.hidden) continue;
    // A dot opens into a full label when pointed at: keep that area clickable too.
    const full = p.mode === 'dot' ? { w: Math.max(p.w, measure(labelText(it)) + 8), h: HEIGHT } : { w: p.w, h: p.h };
    rects.push({ x: p.x - 2, y: p.y - 2, w: full.w + 4, h: full.h + 4 });
  }
  if (chipRect) rects.push({ x: chipRect.x - 2, y: chipRect.y - 2, w: chipRect.w + 4, h: chipRect.h + 4 });
  const menuRect = labelMenu.rect();
  if (menuRect) rects.push({ x: menuRect.x, y: menuRect.y, w: menuRect.width, h: menuRect.height });
  const key = JSON.stringify(rects);
  if (key === reported) return;
  reported = key;
  invoke('scan_labels', { scanId, dpr, rects }).catch(() => {});
}

function onEvent(e) {
  if (e.type === 'scan_started') {
    if (!startsNewScan(e.id, lastScanId)) return;
    lastScanId = e.id;
    scanId = e.id;
    clearAll();
    document.body.classList.remove('joining');
    origin = { x: e.screen.x, y: e.screen.y };
    dpr = window.devicePixelRatio || 1;
    say('Reading the screen…', 1800);
    invoke('launcher_status').then(s => { if (modeRevision === 0 && scanId === e.id && s.scan_id === e.id) applyJoinMode(!!s.scan_join_mode); }).catch(() => {});
    return;
  }
  if (e.id !== scanId) return; // an older scan
  if(e.type==='scan_correction'){
    reserved.add(e.replacement);
    labelMenu.close();
    const old=items.get(e.item);if(old){old.hidden=true;old.el?.remove();old.el=null;}
    if (old && canSeparate(old)) pending.set(e.replacement, [e.item]);
    selected.delete(e.item);schedule();return;
  }
  if (e.type === 'scan_separated') {
    if (e.next_item > 0) reserved.add(e.next_item - 1);
    labelMenu.close();
    for (const id of joinedTreeIds(items, e.item)) {
      const it = items.get(id);
      if (!it) continue;
      it.el?.remove(); items.delete(id); selected.delete(id);
    }
    separating.delete(e.item);
    if (joining) setJoinMode(false);
    schedule();return;
  }
  if (e.type === 'scan_items') {
    for (const data of e.items) {
      reserved.delete(data.item);
      const parts = pending.get(data.item);
      pending.delete(data.item);
      items.set(data.item, { data, status: labelStatus(data), hidden: false, el: null, parts });
      if (parts && joining) setJoinMode(false);
    }
    schedule();
  } else if (e.type === 'scan_label') {
    const it = items.get(e.item);
    if (it) {
      it.data = { ...it.data, label: e.label, source: e.source };
      it.status = labelStatus(it.data);
      schedule();
    }
  } else if (e.type === 'scan_labels_finished') {
    for (const [id, status] of finishLabels(items, e)) items.get(id).status = status;
    schedule();
  } else if (e.type === 'error') {
    say(e.message);
    // a join that was refused (its scan is gone) brings its labels back
    if (e.stage === 'capture') {
      for (const [item, ids] of pending) {
        restore(ids);
        pending.delete(item);
      }
    }
    schedule();
  }
}

listen('maru-scan', onEvent);
listen('maru-scan-clear', (event) => {
  if (!clearsCurrentScan(scanId, event?.scanId)) return;
  lastScanId = Math.max(lastScanId, event?.scanId ?? 0);
  scanId = null;
  clearAll();
  document.body.classList.remove('joining');
});
listen('maru-scan-join-mode', (event) => { if (event.scanId === scanId) { modeRevision++; applyJoinMode(!!event.enabled); } });
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !event.defaultPrevented) { clearSelection(); if (joining) setJoinMode(false); }
});
addEventListener('resize', schedule);

// ───────────── browser-only demo (layout screenshots; never runs inside the app) ─────────────

function demo(name) {
  // The pretend game exists only here: in the app the page must stay fully transparent.
  document.body.classList.add('demo');
  const fake = document.createElement('div');
  fake.id = 'fake';
  fake.setAttribute('aria-hidden', 'true');
  document.body.prepend(fake);
  const list = [];
  const put = (text, x, y, w, h, label, source = 'model', known = false) => list.push({ text, rect: { x, y, w, h }, label, source, known });

  const menu = ['새 게임', '이어하기', '불러오기', '설정', '도감', '업적', '상점', '제작', '종료'];
  const menuEn = ['New game', 'Continue', 'Load', 'Options', 'Collection', 'Achievements', 'Shop', 'Crafting', 'Quit'];
  const items_ = ['철검', '나무 방패', '체력 물약', '마나 물약', '가죽 갑옷', '낡은 지도', '황금 열쇠', '마법 반지', '사과', '빵', '약초', '활'];
  const itemsEn = ['Iron sword', 'Wooden shield', 'Health potion', 'Mana potion', 'Leather armor', 'Old map', 'Golden key', 'Magic ring', 'Apple', 'Bread', 'Herb', 'Bow'];

  if (name === 'menu' || name === 'all') {
    // a tight column: 20px text on a 25px pitch
    menu.forEach((t, i) => put(t, 140, 120 + i * 25, 120, 20, menuEn[i], i % 4 === 3 ? 'dictionary' : 'model'));
  }
  if (name === 'status') {
    put('카메라', 180, 120, 100, 24, 'Camera');
    put('각종', 180, 180, 100, 24, '', 'pending');
    put('설정을', 180, 240, 100, 24, '', 'pending');
    put('조정합니다', 180, 300, 160, 24, '', 'pending');
    put('도감', 180, 360, 100, 24, 'Collection', 'dictionary');
  }
  if (name === 'remembered') {
    put('카메라의 각종 설정을 조정합니다', 180, 180, 360, 24, 'Adjust the camera settings');
    list[0].join = { as_word: false, rules: [['카메라의','각종','설정을','조정합니다']],
      parts: ['카메라의','각종','설정을','조정합니다'].map((text, i) => ({text,rect:{x:180+i*90,y:180,w:80,h:24}})) };
  }
  if (name === 'grid' || name === 'all') {
    // an inventory grid: narrow cells, short gaps
    for (let r = 0; r < 3; r++)
      for (let c = 0; c < 6; c++) {
        const k = (r * 6 + c) % items_.length;
        put(items_[k], 420 + c * 86, 140 + r * 56, 74, 16, itemsEn[k], c === 4 && r === 1 ? 'pending' : 'model', c === 5 && r === 2);
      }
  }
  if (name === 'split' || name === 'all') {
    // the detector cut these phrases in two
    put('새', 140, 360, 24, 20, 'New');
    put('게임', 172, 360, 44, 20, 'Game');
    put('이어하기', 140, 385, 80, 20, 'Continue', 'model');
    put('빨간', 420, 360, 40, 18, 'Red');
    put('물약', 468, 360, 40, 18, 'Potion');
  }
  if (name === 'hud' || name === 'all') {
    put('체력 87/100', 30, 20, 110, 18, 'HP 87/100');
    put('경험치', 30, 44, 60, 16, 'EXP', 'dictionary');
    put('임무: 마을 장로와 대화하기', 760, 22, 300, 20, 'Quest: talk to the village elder');
    put('저장하려면 Enter 키를 누르세요', 380, 520, 330, 20, 'Press Enter to save');
  }
  if (name === 'dense') {
    // hopeless: rows touching, wide labels
    for (let i = 0; i < 14; i++) put('아주 긴 메뉴 항목 이름 ' + (i + 1), 200, 80 + i * 21, 220, 20, 'A very long menu item name ' + (i + 1));
    for (let r = 0; r < 4; r++) for (let c = 0; c < 8; c++) put('치유 물약', 520 + c * 62, 120 + r * 22, 58, 18, 'Healing potion');
  }
  list.forEach((it, i) => {
    it.item = i;
    const g = document.createElement('div');
    g.className = 'g';
    g.textContent = it.text;
    Object.assign(g.style, { left: `${it.rect.x}px`, top: `${it.rect.y}px`, width: `${it.rect.w}px`, height: `${it.rect.h}px`, fontSize: `${it.rect.h * 0.78}px`, lineHeight: `${it.rect.h}px` });
    fake.append(g);
  });
  const withLabels = list.map((it) => ({ ...it, label: it.source === 'dictionary' || it.source === 'pending' ? it.label : '', source: it.source === 'dictionary' ? 'dictionary' : 'pending' }));
  const withoutPending = withLabels.map((it) => (it.text.length && it.label === '' ? it : it));
  emitMock('maru-scan', { type: 'scan_started', id: 1, screen: { x: 0, y: 0, w: innerWidth, h: innerHeight } });
  emitMock('maru-scan', { type: 'scan_items', id: 1, items: withoutPending });
  // the model's answers arrive a moment later
  const finish = () => {
    list.forEach((it) => it.source !== 'pending' && it.source !== 'dictionary' && emitMock('maru-scan', { type: 'scan_label', id: 1, item: it.item, label: it.label, source: 'model' }));
    if (name === 'status') {
      emitMock('maru-scan', { type: 'scan_labels_finished', id: 1, items: [0], error: null });
      emitMock('maru-scan', { type: 'scan_labels_finished', id: 1, items: [1], error: 'The translation request timed out.' });
      emitMock('maru-scan', { type: 'scan_labels_finished', id: 1, items: [2], error: null });
    } else {
      emitMock('maru-scan', { type: 'scan_labels_finished', id: 1, items: list.map(it => it.item), error: null });
      emitMock('maru-scan', { type: 'done', id: 1 });
    }
  };
  if (params.has('instant')) finish();
  else setTimeout(finish, 350);
}

if (!inTauri) {
  const joinItemsMock = ({ scanId: id, item, items: selectedItems, asWord }) => {
    if (id !== scanId) throw new Error('That scan changed.');
    const chosen = selectedItems.map(n => items.get(n)?.data);
    if (chosen.some(it => !it)) throw new Error('A label changed.');
    const sorted = [...chosen].sort((a,b) => a.rect.y-b.rect.y || a.rect.x-b.rect.x);
    const text = sorted.map(p => p.text).join(asWord ? '' : ' ');
    const parts = sorted.flatMap(p => p.join?.parts ?? [{text:p.text,rect:p.rect}]);
    const rules = sorted.flatMap(p => p.join?.rules ?? []).concat([sorted.map(p => p.text)]);
    const rect = unionRect(sorted.map(p => p.rect));
    window.__joined = (window.__joined ?? []).concat([{id,item,text,asWord}]);
    setTimeout(() => emitMock('maru-scan', {type:'scan_items',id,items:[{item,text,rect,label:'',source:'pending',known:false,join:{parts,rules,as_word:asWord}}]}),60);
    setTimeout(() => {
      emitMock('maru-scan', {type:'scan_label',id,item,label:`Joined: ${text}`,source:'model'});
      emitMock('maru-scan', {type:'scan_labels_finished',id,items:[item],error:null});
      emitMock('maru-scan', {type:'done',id});
    },300);
    return null;
  };
  const separateMock = ({ scanId:id, item }) => {
    const it = items.get(item);
    if (id !== scanId || !it?.data.join) throw new Error('Use the current joined label.');
    const first = nextJoinId(items.keys(),pending.keys(),reserved);
    const restored = it.data.join.parts.map((p,i)=>({...p,item:first+i,label:'',source:'pending',known:false}));
    window.__separated = (window.__separated ?? []).concat([{id,item,texts:restored.map(p=>p.text)}]);
    emitMock('maru-scan',{type:'scan_separated',id,item,next_item:first+restored.length});
    emitMock('maru-scan',{type:'scan_items',id,items:restored});
    setTimeout(()=>{
      for(const part of restored) emitMock('maru-scan',{type:'scan_label',id,item:part.item,label:`Part: ${part.text}`,source:'model'});
      emitMock('maru-scan',{type:'scan_labels_finished',id,items:restored.map(p=>p.item),error:null});
    },100);
    return null;
  };
  // The browser has no backend: answer a join the way the real one does (reading order, one box, a label a moment later).
  const joinMock = ({ scanId: id, item, pieces }) => {
    const sorted = [...pieces].sort((a, b) => a.rect.y - b.rect.y || a.rect.x - b.rect.x);
    const x0 = Math.min(...sorted.map((p) => p.rect.x));
    const y0 = Math.min(...sorted.map((p) => p.rect.y));
    const x1 = Math.max(...sorted.map((p) => p.rect.x + p.rect.w));
    const y1 = Math.max(...sorted.map((p) => p.rect.y + p.rect.h));
    const text = sorted.map((p) => p.text).join(' ');
    window.__joined = (window.__joined ?? []).concat([{ id, item, text }]);
    setTimeout(() => emitMock('maru-scan', { type: 'scan_items', id, items: [{ item, rect: { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, text, label: '', source: 'pending', known: false }] }), 60);
    setTimeout(() => {
      emitMock('maru-scan', { type: 'scan_label', id, item, label: `Joined: ${text}`, source: 'model' });
      emitMock('maru-scan', { type: 'scan_labels_finished', id, items: [item], error: null });
      emitMock('maru-scan', { type: 'done', id });
    }, 300);
    return null;
  };
  mockCommands({
    scan_join_items: joinItemsMock,
    scan_separate: separateMock,
    scan_open_item: ({scanId:id,item}) => {
      if(id!==scanId || !items.get(item)) throw new Error('That label changed.');
      const data=items.get(item).data;
      window.__opened=(window.__opened??[]).concat([{text:data.text,rect:data.rect,scanId:id,item}]);return null;
    },
    scan_labels: (args) => { window.__hitRects = args; return null; },
    scan_open: (args) => { window.__opened = (window.__opened ?? []).concat([args]); return null; },
    launcher_status: () => ({ scan: scanId != null, scan_id: scanId, scan_join_mode: joining }),
    scan_join_mode: ({ scanId: id, enabled }) => { if (id !== scanId) throw new Error('The scan changed.'); return enabled; },
    correction_open_scan: () => {location.href='correction.html?demo=scan';},
    scan_join: joinMock,
    scan_unjoin: (a) => {
      window.__unjoined = (window.__unjoined ?? []).concat([a.joined]);
      return 1;
    },
  });
  const name = params.get('demo');
  if (name) queueMicrotask(() => demo(name));
}
