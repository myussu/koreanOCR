import { invoke, inTauri, mockCommands } from './bridge.js';
import { bytesText, percentText, processCells, vramState } from './resources-core.js';

if (!inTauri) {
  const demo = new URLSearchParams(location.search).get('demo');
  const status = { enabled: true, recording: false, path: null, queued: 0, active: 1, dropped_events: 0 };
  mockCommands({
    resource_status: () => ({ ...status, sample: {
      at_ms: Date.now(), collection_ms: 2.3, notes: [],
      gpus: demo === 'missing' ? [] : [{ name: 'NVIDIA GPU', dedicated_total_bytes: 16 * 1024 ** 3, dedicated_used_bytes: (demo === 'critical' ? 15.5 : 9.5) * 1024 ** 3, gpu_percent: 78 }],
      processes: [
        { target: { name: 'Maru + in-process OCR', state: 'running', pid: 4200 }, cpu_percent: 3.1, ram_bytes: 650 * 1024 ** 2, gpu_percent: 0, dedicated_gpu_bytes: 0, shared_gpu_bytes: 0 },
        { target: { name: 'MiLMMT 4B Q6', state: 'loaded', pid: 4201 }, cpu_percent: 1.2, ram_bytes: 500 * 1024 ** 2, gpu_percent: 72, dedicated_gpu_bytes: 4.5 * 1024 ** 3, shared_gpu_bytes: 0 },
        { target: { name: 'Gemma / main model', state: 'unloaded', pid: null } },
        { target: { name: 'PP-OCR / Windows OCR', state: 'inside Maru; included in the Maru row', pid: null } },
      ],
    }}),
    resource_record: ({ enabled }) => { status.recording = enabled; if (enabled) status.path = 'Demo only — no file is written'; },
    resource_enable: ({ enabled }) => { status.enabled = enabled; if (!enabled) status.recording = false; },
  });
}

function element(tag, text, className) {
  const el = document.createElement(tag); if (text != null) el.textContent = text; if (className) el.className = className; return el;
}
function styles() {
  if (!document.querySelector('link[data-resources]')) {
    const link = document.createElement('link'); link.rel = 'stylesheet'; link.href = 'resources.css'; link.dataset.resources = ''; document.head.append(link);
  }
}
export function makeVramBar() {
  styles();
  const root = element('section', null, 'vram-summary');
  const title = element('strong'), track = element('div', null, 'vram-track'), fill = element('i'), detail = element('small');
  track.setAttribute('role', 'progressbar'); track.setAttribute('aria-label', 'Local dedicated GPU memory occupancy, all apps');
  track.setAttribute('aria-valuemin', '0'); track.setAttribute('aria-valuemax', '100'); track.append(fill); root.append(title, track, detail);
  root.update = status => {
    const state = vramState(status); root.dataset.level = state.level;
    title.textContent = state.label; detail.textContent = state.detail; fill.style.width = `${state.percent ?? 0}%`;
    track.setAttribute('aria-valuetext', state.label);
    if (state.percent == null) track.removeAttribute('aria-valuenow'); else track.setAttribute('aria-valuenow', String(state.percent));
  };
  root.update(null); return root;
}

export async function resourcesView() {
  styles();
  const root = element('section', null, 'resources-page');
  const title = element('h3', 'Resources');
  const intro = element('p', 'Updates about once per second while this view or the floating menu is visible, or while recording.');
  const bar = makeVramBar(), controls = element('div', null, 'resource-controls');
  const pause = element('button', 'Pause monitoring', 'btn'), record = element('button', 'Record resource session', 'btn');
  const message = element('p', '', 'resource-message'); message.setAttribute('role', 'status');
  const path = element('code', '', 'resource-path'), metrics = element('p'), gpuList = element('div');
  const wrap = element('div', null, 'resource-table-wrap'), table = element('table');
  table.append(element('caption', 'Owned local processes and model roles'));
  const head = element('thead'), row = element('tr');
  for (const name of ['Component', 'CPU', 'RAM', 'GPU', 'VRAM', 'Shared GPU']) row.append(element('th', name));
  head.append(row); const body = element('tbody'); table.append(head, body); wrap.append(table);
  const notes = element('div'), legend = element('p', 'CPU is a share of all logical CPUs. GPU is the busiest engine; percentages cannot be added. RAM is the process working set. Shared allocations can overlap. In-process OCR is included in Maru. External servers are not measured here. VRAM high ≥80%, critical ≥95%; occupancy alone does not prove a slowdown.', 'resource-legend');
  const recordingHelp = element('p', 'Recording stays active when these controls close. Logs contain local counters and request timings, not subtitle text. Up to 16 MiB per session and 64 MiB total; older logs are retained. Pause monitoring stops recording.');
  controls.append(pause, record); root.append(title, intro, bar, controls, message, path, metrics, gpuList, wrap, notes, legend, recordingHelp);
  let current, pending = false, disposed = false, refreshing = false;
  async function refresh() {
    if (disposed || refreshing || document.hidden) return;
    refreshing = true;
    try {
      current = await invoke('resource_status'); if (disposed) return;
      bar.update(current); pause.textContent = current.enabled ? 'Pause monitoring' : 'Resume monitoring';
      pause.setAttribute('aria-pressed', String(!current.enabled)); record.textContent = current.recording ? 'Stop recording' : 'Record resource session';
      record.setAttribute('aria-pressed', String(current.recording)); record.disabled = pending || !current.enabled;
      message.textContent = current.error || (current.recording ? 'Recording locally, including while this window is closed.' : current.enabled ? 'Monitoring on demand. Recording is off.' : 'Monitoring paused.');
      path.textContent = current.path ? `Log: ${current.path}` : '';
      const sample = current.sample;
      const age = sample ? Math.max(0, Date.now() - sample.at_ms) : null;
      metrics.textContent = `Queue: ${current.queued} waiting · ${current.active} active · ${current.dropped_events} dropped log events${sample ? ` · counter collection ${sample.collection_ms.toFixed(1)} ms · sample age ${(age / 1000).toFixed(1)} s` : ''}`;
      gpuList.replaceChildren(...(sample?.gpus || []).map(g => element('p', `${g.name}: ${bytesText(g.dedicated_used_bytes)} / ${bytesText(g.dedicated_total_bytes)} VRAM · GPU ${percentText(g.gpu_percent)}`)));
      body.replaceChildren(...(sample?.processes || []).map(p => {
        const tr = element('tr'), component = element('td'); component.append(element('strong', p.target.name), element('small', `${p.target.state}${p.target.pid ? ` · PID ${p.target.pid}` : ''}`));
        tr.append(component, ...processCells(p).map(value => element('td', value))); return tr;
      }));
      notes.replaceChildren(...(sample?.notes || []).map(note => element('p', note)));
    } catch (e) { message.textContent = String(e); bar.update(null); }
    finally { refreshing = false; }
  }
  async function command(name, enabled) {
    if (pending) return; pending = true; pause.disabled = record.disabled = true;
    try { await invoke(name, { enabled }); await refresh(); }
    catch (e) { message.textContent = String(e); }
    finally { pending = false; pause.disabled = false; record.disabled = !current?.enabled; }
  }
  pause.onclick = () => command('resource_enable', !current?.enabled);
  record.onclick = () => command('resource_record', !current?.recording);
  await refresh();
  const interval = setInterval(() => { if (!root.isConnected) root.dispose(); else refresh(); }, 1000);
  root.dispose = () => { disposed = true; clearInterval(interval); };
  return [root];
}
