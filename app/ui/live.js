// The live-subtitle band: draws each translation over the original line, as soon as its first words
// arrive, and takes it down a moment after the original is gone.

import { emitMock, inTauri, invoke, listen, mockCommands } from './bridge.js';
import { bandStyle, clampCenter, createBand, fontFor } from './live-core.js';

const band = document.getElementById('band');
const state = createBand();
let hideTimer = 0;

function place(p) {
  const dpr = window.devicePixelRatio || 1;
  const cover = { x: p.cover.x / dpr, y: p.cover.y / dpr, w: p.cover.w / dpr, h: p.cover.h / dpr };
  const win = { w: window.innerWidth, h: window.innerHeight };
  const st = bandStyle({ cover, win, anchorBottom: p.anchor_bottom });
  band.style.fontSize = `${fontFor(p.line_h / dpr)}px`;
  band.style.minWidth = `${st.minWidth}px`;
  band.style.maxWidth = `${st.maxWidth}px`;
  band.style.minHeight = `${st.minHeight}px`;
  band.style.top = st.top != null ? `${st.top}px` : 'auto';
  band.style.bottom = st.bottom != null ? `${st.bottom}px` : 'auto';
  band.textContent = p.text;
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

// Browser preview: ?demo shows a sample translation.
if (!inTauri && new URLSearchParams(location.search).has('demo')) {
  mockCommands({ live_idle: () => {} });
  document.body.style.background = '#334';
  emitMock('maru-live', { id: 1, text: 'I thought I would never see you again.', done: true, cover: { x: 220, y: 250, w: 560, h: 40 }, line_h: 34, anchor_bottom: true, linger_ms: 99999 });
}
