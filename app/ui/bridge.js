// The only file that knows whether we run inside Tauri or in a plain browser (dev / screenshots).

const tauri = window.__TAURI__;
const params = new URLSearchParams(location.search);

const listeners = new Map();
const mockHandlers = {};

export const inTauri = !!tauri;

/** Register mock command handlers (browser only). */
export function mockCommands(map) {
  Object.assign(mockHandlers, map);
}

export function invoke(cmd, args = {}) {
  if (tauri) return tauri.core.invoke(cmd, args);
  const h = mockHandlers[cmd];
  return h ? Promise.resolve(h(args)) : Promise.reject(new Error(`no mock for ${cmd}`));
}

export function listen(name, cb) {
  if (tauri) return tauri.event.listen(name, (e) => cb(e.payload));
  if (!listeners.has(name)) listeners.set(name, new Set());
  listeners.get(name).add(cb);
  return Promise.resolve(() => listeners.get(name).delete(cb));
}

/** Browser only: deliver an event as if the backend had emitted it. */
export function emitMock(name, payload) {
  for (const cb of listeners.get(name) ?? []) cb(payload);
}

/** `?replay=file.json` plays back recorded engine events with their original spacing (screenshots, demos). */
export async function replayFromQuery(speed = 1) {
  const file = params.get('replay');
  if (!file || tauri) return false;
  const events = await (await fetch(file)).json();
  for (const e of events) {
    emitMock('maru-event', e);
    if (!params.has('instant')) await new Promise((r) => setTimeout(r, (e.type === 'translation' ? 35 : 5) / speed));
  }
  return true;
}
