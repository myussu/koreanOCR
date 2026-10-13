// GPU occupancy includes the game. Missing or stale counters never become a green zero.
export function bytesText(value) {
  if (!Number.isFinite(value) || value < 0) return 'Unavailable';
  return value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GiB` : `${(value / 1024 ** 2).toFixed(0)} MiB`;
}
export function percentText(value) { return Number.isFinite(value) && value >= 0 ? `${value.toFixed(1)}%` : 'Unavailable'; }
export function vramState(status, now = Date.now()) {
  const missing = { percent: null, level: 'unknown', label: 'VRAM unavailable', detail: 'Waiting for local GPU memory counters.' };
  if (status?.enabled === false) return { ...missing, label: 'VRAM monitoring paused', detail: 'Resume in Resources.' };
  const sample = status?.sample;
  if (!sample || !Number.isFinite(sample.at_ms) || now - sample.at_ms > 3500) return missing;
  const usable = (sample.gpus || []).filter(g => Number.isFinite(g.dedicated_total_bytes) && g.dedicated_total_bytes > 0 && Number.isFinite(g.dedicated_used_bytes) && g.dedicated_used_bytes >= 0);
  const gpu = usable.sort((a, b) => b.dedicated_used_bytes / b.dedicated_total_bytes - a.dedicated_used_bytes / a.dedicated_total_bytes)[0];
  if (!gpu) return missing;
  const percent = Math.min(100, Math.round(gpu.dedicated_used_bytes / gpu.dedicated_total_bytes * 100));
  const level = percent >= 95 ? 'critical' : percent >= 80 ? 'high' : 'normal';
  return { percent, level, label: `VRAM ${percent}%${level === 'normal' ? '' : ` · ${level}`}`, detail: `${gpu.name} · ${bytesText(gpu.dedicated_used_bytes)} / ${bytesText(gpu.dedicated_total_bytes)} · all apps` };
}

export function processCells(p) {
  return [percentText(p.cpu_percent), bytesText(p.ram_bytes), percentText(p.gpu_percent), bytesText(p.dedicated_gpu_bytes), bytesText(p.shared_gpu_bytes)];
}
