// The live-subtitle band: draws each translation next to the original line (above or below it, whichever side has
// room), as soon as its first words arrive, and takes it down a moment after the original is gone.

import { emitMock, inTauri, invoke, listen, mockCommands } from './bridge.js';
import { bandStyle, bandWidth, chooseSide, clampCenter, createBand, estimateHeight, fitFont, fontFor, gapFor, roomOn } from './live-core.js';

const band = document.getElementById('band');
const state = createBand();
let hideTimer = 0;
/** The side chosen for the line being shown: it stays put while the translation streams in, so the band does not jump. */
let chosen = { id: 0, side: 'above' };

function place(p) {
  const dpr = window.devicePixelRatio || 1;
  const cover = { x: p.cover.x / dpr, y: p.cover.y / dpr, w: p.cover.w / dpr, h: p.cover.h / dpr };
  const win = { w: window.innerWidth, h: window.innerHeight };
  const lineH = p.line_h / dpr;
  const gap = gapFor(lineH);
  const maxWidth = bandWidth({ cover, win });
  const at = { cover, win, gap };

  let font = fontFor(lineH);
  band.style.maxWidth = `${maxWidth}px`;
  band.style.fontSize = `${font}px`;
  band.textContent = p.text;

  // Decide the side once per line, from a guess of how tall the translation will be; later words may only move it if
  // what has arrived no longer fits and the other side has more room.
  let side = chosen.id === p.id ? chosen.side : chooseSide({ ...at, needed: estimateHeight({ sourceChars: p.source_chars, font, maxWidth }), prefer: p.prefer_above ? 'above' : 'below' });
  const other = side === 'above' ? 'below' : 'above';
  if (band.offsetHeight > roomOn(side, at) && roomOn(other, at) > roomOn(side, at)) side = other;
  // Neither side has room for it at this size: make the text smaller rather than cover the original.
  font = fitFont(font, roomOn(side, at), (f) => {
    band.style.fontSize = `${f}px`;
    return band.offsetHeight;
  });
  band.style.fontSize = `${font}px`;
  chosen = { id: p.id, side };

  const st = bandStyle({ cover, win, side, gap });
  band.style.top = st.top != null ? `${st.top}px` : 'auto';
  band.style.bottom = st.bottom != null ? `${st.bottom}px` : 'auto';
  // Measure at the left edge, where the whole window width is available, then centre it on the original.
  band.style.left = '0px';
  const width = band.offsetWidth;
  band.style.left = `${clampCenter(st.center, width, win.w) - width / 2}px`;
}

function run(result) {
  if (result.cancelHide) {
    clearTimeout(hideTimer);
    band.classList.remove('leaving');
  }
  if (result.show) {
    place(result.show);
    band.classList.add('on');
  }
  if (result.hideIn != null) {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      band.classList.add('leaving');
      band.classList.remove('on');
      // once it has faded, the window itself goes away
      hideTimer = setTimeout(() => invoke('live_idle').catch(() => {}), 180);
    }, result.hideIn);
  }
}

listen('maru-live', (p) => run(state.line(p)));
listen('maru-live-clear', (id) => run(state.cleared(Number(id) || 0)));
listen('maru-live-off', () => {
  run(state.off());
  band.classList.remove('on', 'leaving');
});

// Browser preview. `?demo` shows a sample translation under or over a pretend original subtitle: `?demo=low` (the default, a subtitle
// low on the screen), `?demo=high` (near the top), `?demo=tight` (no room on either side: the text shrinks), `?demo=long` (a long translation).
if (!inTauri && new URLSearchParams(location.search).has('demo')) {
  mockCommands({ live_idle: () => {} });
  const name = new URLSearchParams(location.search).get('demo') || 'low';
  document.body.style.background = '#334';
  const H = window.innerHeight;
  const spots = {
    low: { y: H - 90, prefer_above: true },
    high: { y: 40, prefer_above: false },
    tight: { y: Math.round(H / 2) - 20, prefer_above: true, shortWindow: true },
    long: { y: H - 90, prefer_above: true },
  };
  const spot = spots[name] ?? spots.low;
  const cover = { x: 220, y: spot.y, w: 560, h: 40 };
  const original = document.createElement('div');
  original.id = 'original';
  original.textContent = '다시는 못 볼 줄 알았어요';
  Object.assign(original.style, { position: 'fixed', left: `${cover.x}px`, top: `${cover.y}px`, width: `${cover.w}px`, height: `${cover.h}px`, color: '#fff', font: '700 28px/40px system-ui', textAlign: 'center', textShadow: '0 0 3px #000, 0 0 3px #000', outline: '1px dashed rgba(255,220,120,.8)' });
  document.body.append(original);
  const text = name === 'long' ? 'I thought I would never see you again, not after everything that happened between us in the village, and yet here you are standing in front of me.' : 'I thought I would never see you again.';
  emitMock('maru-live', { id: 1, text, done: true, cover, line_h: 34, source_chars: 13, prefer_above: spot.prefer_above, linger_ms: 99999 });
}
