// Placement of the live translation band, kept free of the DOM so it can be tested.
//
// The window holds the subtitle area with room above and below it. The original line stays visible: the band
// goes on one side of the box OCR found (`cover`, in the window's CSS pixels), on the side with room for it, and
// is shrunk only when neither side has enough.

const EDGE = 8;
/** Narrowest and widest the band is allowed to be (CSS pixels), before it is limited by the window. */
const WIDE = 640;
const MIN_FONT = 14;

/** The text size that matches the original's: about 0.8 of its line height, kept readable. */
export function fontFor(lineH) {
  if (!(lineH > 0)) return 22;
  return Math.max(15, Math.min(40, Math.round(lineH * 0.8)));
}

/** Clear space between the original line and the band: about a third of a line, never touching. */
export function gapFor(lineH) {
  return Math.max(8, Math.round((lineH > 0 ? lineH : 24) * 0.35));
}

/** How wide the band may be: as wide as the original with some to spare, at least a comfortable line, never wider than the window. */
export function bandWidth({ cover, win }) {
  return Math.max(120, Math.min(win.w - EDGE * 2, Math.max(cover.w + 80, WIDE)));
}

/**
 * How tall the translation will be, guessed before it has arrived from the length of the original: English runs to
 * roughly twice the Korean character count and a character is about half an em wide.
 */
export function estimateHeight({ sourceChars, font, maxWidth }) {
  const chars = Math.max(8, Math.round((sourceChars > 0 ? sourceChars : 20) * 2.3));
  const perLine = Math.max(8, Math.floor((maxWidth - 32) / (font * 0.52)));
  const lines = Math.max(1, Math.ceil(chars / perLine));
  return Math.round(lines * font * 1.3 + 12);
}

/** Free space (CSS pixels) on one side of the original line inside the window. */
export function roomOn(side, { cover, win, gap }) {
  return side === 'above' ? cover.y - gap - EDGE : win.h - (cover.y + cover.h) - gap - EDGE;
}

/**
 * Which side of the original line the band goes on. The preferred side if the band fits there (`needed` is its
 * height), otherwise the other side if it fits there, otherwise whichever has more room.
 */
export function chooseSide({ cover, win, gap, needed, prefer }) {
  const first = prefer === 'above' ? 'above' : 'below';
  const second = first === 'above' ? 'below' : 'above';
  const at = { cover, win, gap };
  if (roomOn(first, at) >= needed) return first;
  if (roomOn(second, at) >= needed) return second;
  return roomOn('above', at) >= roomOn('below', at) ? 'above' : 'below';
}

/** CSS for the band: pinned to the original line's edge on `side`, `gap` away from it, centred on it. */
export function bandStyle({ cover, win, side, gap }) {
  const style = { side, center: cover.x + cover.w / 2 };
  if (side === 'above') {
    style.bottom = Math.max(EDGE, win.h - cover.y + gap);
  } else {
    style.top = Math.min(win.h - EDGE, cover.y + cover.h + gap);
  }
  return style;
}

/** The font to use so the band fits `room`: `measure(font)` is the band's height at that size. Shrinks in steps, to a floor. */
export function fitFont(font, room, measure) {
  let f = font;
  while (f > MIN_FONT && measure(f) > room) f -= 2;
  return Math.max(MIN_FONT, f);
}

/** Move a band of `width` so that it stays inside a window of `winW`, as close to `center` as it can. */
export function clampCenter(center, width, winW) {
  const half = width / 2;
  return Math.max(EDGE + half, Math.min(winW - EDGE - half, center));
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

/** Capture ids keep live history in dialogue order even when answers finish out of order. */
export const liveHistory = (items) => items.filter((i) => i.live).sort((a, b) => a.id - b.id);
