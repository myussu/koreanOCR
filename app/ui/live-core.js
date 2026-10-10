// Placement of the live translation band, kept free of the DOM so it can be tested.
//
// The window sits over the subtitle area. The band must hide the original line: it covers the box OCR
// found (`cover`, in the window's CSS pixels), is never narrower or shorter than it, and when the
// translation needs more room than the original it grows away from the screen edge.

const MARGIN = 8;

/** The text size that matches the original's: about 0.8 of its line height, kept readable. */
export function fontFor(lineH) {
  if (!(lineH > 0)) return 22;
  return Math.max(15, Math.min(40, Math.round(lineH * 0.8)));
}

/**
 * CSS for the band.
 * `win` is the window's size, `anchorBottom` says whether the band grows upward.
 */
export function bandStyle({ cover, win, anchorBottom, pad = 10 }) {
  const style = {
    minWidth: Math.min(cover.w + pad * 2, win.w - MARGIN * 2),
    maxWidth: win.w - MARGIN * 2,
    minHeight: cover.h + pad * 2,
    center: cover.x + cover.w / 2,
  };
  if (anchorBottom) {
    style.bottom = Math.max(0, win.h - (cover.y + cover.h) - pad);
  } else {
    style.top = Math.max(0, cover.y - pad);
  }
  return style;
}

/** Move a band of `width` so that it stays inside a window of `winW`, as close to `center` as it can. */
export function clampCenter(center, width, winW) {
  const half = width / 2;
  return Math.max(MARGIN + half, Math.min(winW - MARGIN - half, center));
}

/**
 * What the band shows and when it goes. Pure state machine: feed it what arrives, ask what to do.
 *   line(p)        a translation (partial or final) for line p.id
 *   cleared(id)    the original line `id` is gone
 * Both return `{ show: payload | null, hideIn: ms | null, cancelHide: bool }`.
 */
export function createBand() {
  let current = 0;
  let done = false;
  let linger = 1500;
  let clearedUpTo = 0;
  const act = (over = {}) => ({ show: null, hideIn: null, cancelHide: false, ...over });
  const maybeHide = () => (done && clearedUpTo >= current ? act({ hideIn: linger }) : act());
  return {
    line(p) {
      if (p.id < current) return act(); // an older line finishing late
      current = p.id;
      done = !!p.done;
      linger = p.linger_ms ?? linger;
      const hide = maybeHide();
      return act({ show: p, cancelHide: hide.hideIn == null, hideIn: hide.hideIn });
    },
    cleared(id) {
      clearedUpTo = Math.max(clearedUpTo, id);
      return maybeHide();
    },
    off() {
      current = 0;
      done = false;
      clearedUpTo = 0;
      return act({ cancelHide: true });
    },
  };
}

/** "0.8 s to the first word · 1.4 s in all": how fast a live line was translated, for the history. */
export function latencyText(item) {
  if (item.first_ms == null || item.total_ms == null) return '';
  const s = (ms) => `${(ms / 1000).toFixed(1)} s`;
  return `${s(item.first_ms)} to the first word · ${s(item.total_ms)} in all`;
}
