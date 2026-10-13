// Keep a right-click menu inside its window, including near screen edges.
export function menuPlacement(point, size, bounds) {
  const edge = 4;
  return {
    x: Math.max(edge, Math.min(point.x, bounds.w - size.w - edge)),
    y: Math.max(edge, Math.min(point.y, bounds.h - size.h - edge)),
  };
}
