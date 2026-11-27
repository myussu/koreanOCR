// History's decisions about making cards, as pure functions (tested in history.test.mjs).

/** A line can become cards when it is Korean with a translation: live subtitles and captures alike. */
export function canMine(item) {
  return !!item && item.direction !== 'en_to_ko' && !!String(item.source ?? '').trim() && !!String(item.translation ?? '').trim();
}

/** The History item a line handed over by the panel is: the same request, or else the same Korean text. */
export function sameLine(item, line) {
  if (!item || !line) return false;
  if (line.id != null && item.id === line.id) return true;
  return String(item.source ?? '').trim() === String(line.sentence ?? '').trim();
}

/**
 * The rows to show with the panel's line among them. A capture History no longer lists (or one the current filter
 * hides) is put first, as a captured line, so its sheet can still open.
 */
export function withPendingLine(items, line) {
  if (!line || items.some((i) => sameLine(i, line))) return items;
  const item = { id: line.id ?? -1, at: Math.floor(Date.now() / 1000), source: line.sentence, translation: line.translation, direction: 'ko_to_en', app: line.app ?? null, live: false };
  return [item, ...items];
}
