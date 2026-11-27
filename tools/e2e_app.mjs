// End-to-end check of the real Maru app on Windows: it starts maru-app with a temporary home, drives its own windows
// through WebView2's DevTools port (clicks are DOM clicks inside Maru's pages: no mouse or keyboard input reaches the
// desktop or any other program), walks every Settings section, changes and saves one setting per section and checks
// config.toml, then opens History → Starred → Make cards. Anki is switched off in the temporary home, so nothing is
// written to Anki; the translation server is a dead address, so no model loads.
//
//   node tools/e2e_app.mjs --app app\src-tauri\target\release\maru-app.exe --out <empty folder> [--history <history.db>]
//
// `--history` copies that file into the temporary home, so there are starred lines for Make cards (give a copy of your
// own `history.db`, never the file Maru is using). Screenshots and report.json go to --out. Needs Node 22 or newer.
import { spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const arg = (name, fallback = null) => { const i = process.argv.indexOf(name); return i > 0 ? process.argv[i + 1] : fallback; };
const app = resolve(arg('--app', 'app/src-tauri/target/release/maru-app.exe'));
const out = resolve(arg('--out', 'e2e-out'));
const history = arg('--history');
const port = Number(arg('--port', '9339'));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** One setting per section: where it is, what to do with it, and what config.toml must then say. */
export const CHANGES = [
  { section: 'live', path: 'reading.live.detector', act: { choose: 'Standard' }, expect: /\[reading\.live\][^[]*detector = "standard"/s },
  { section: 'captures', path: 'learning.gloss_mode', act: { choose: 'Every word' }, expect: /gloss_mode = "all"/ },
  { section: 'cards', path: 'learning.anki.new_words_per_day', act: { type: '5' }, expect: /new_words_per_day = 5/ },
  { section: 'voice', path: 'tts.voice', act: { type: 'e2e-voice' }, expect: /voice = "e2e-voice"/ },
  { section: 'connections', path: 'llm.server_keep_minutes', act: { type: '3' }, expect: /server_keep_minutes = 3/ },
  { section: 'shortcuts', path: 'hotkeys.star', act: { key: 'F9' }, expect: /star = "[^"]*F9[^"]*"/ },
  { section: 'advanced', path: 'continuous.drop_stray_lines', act: { toggle: true }, expect: /drop_stray_lines = true/ },
];
export const SECTIONS = ['home', 'live', 'captures', 'cards', 'voice', 'connections', 'shortcuts', 'advanced'];

/** The temporary home's config: no keys, Anki off, a dead translation server. */
export const CONFIG = `# Maru end-to-end check (temporary). Nothing here is anyone's real setting.
[llm]
endpoint = "http://127.0.0.1:9"
model_name = "none"

[learning.anki]
enabled = false
`;

async function page(match) {
  for (let i = 0; i < 60; i++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const t = targets.find((x) => x.type === 'page' && x.url.includes(match));
      if (t) return connect(t.webSocketDebuggerUrl);
    } catch { /* not up yet */ }
    await sleep(500);
  }
  throw new Error(`no Maru window showing ${match}`);
}

async function connect(url) {
  const ws = new WebSocket(url);
  let next = 1;
  const waiting = new Map();
  ws.onmessage = (m) => { const msg = JSON.parse(m.data); waiting.get(msg.id)?.(msg); };
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  const send = (method, params = {}) => new Promise((r) => { const id = next++; waiting.set(id, r); ws.send(JSON.stringify({ id, method, params })); });
  return {
    async eval(expression) {
      const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
      if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description ?? 'page error');
      return r.result?.result?.value;
    },
    async shot(file) { const r = await send('Page.captureScreenshot', { format: 'png' }); writeFileSync(file, Buffer.from(r.result.data, 'base64')); },
    close: () => ws.close(),
  };
}

// Runs inside the Settings page: DOM clicks and typing, the way a person's input arrives at the page.
const IN_PAGE = `
window.__e2e = {
  go(title) { const b = [...document.querySelectorAll('.settings-sidebar button')].find((x) => x.textContent.trim() === title); if (!b) throw Error('no section ' + title); b.click(); },
  rows() { return [...document.querySelectorAll('.set-row')].filter((r) => !r.hidden).map((r) => r.dataset.path); },
  row(path) { const r = document.querySelector('.set-row[data-path="' + path + '"]'); if (!r || r.hidden) throw Error('setting not shown: ' + path); return r; },
  choose(path, label) { const b = [...this.row(path).querySelectorAll('button[role=radio]')].find((x) => x.textContent.trim().startsWith(label)); if (!b) throw Error('no choice ' + label); b.click(); },
  type(path, text) { const i = this.row(path).querySelector('input'); i.focus(); i.value = text; i.dispatchEvent(new Event('input', { bubbles: true })); },
  toggle(path, on) { const i = this.row(path).querySelector('input[role=switch]'); if (i.checked !== on) i.click(); },
  key(path, key) { const add = [...this.row(path).querySelectorAll('button')].find((b) => /Add another key|Set key/.test(b.textContent)); add.click(); dispatchEvent(new KeyboardEvent('keydown', { key, code: key, bubbles: true })); },
  save() { const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim() === 'Save'); if (b.disabled) throw Error('nothing to save'); b.click(); },
  message() { return document.querySelector('.msg')?.textContent ?? ''; },
};`;

async function main() {
  if (!existsSync(app)) throw new Error(`no app at ${app} (cargo build --release --manifest-path app/src-tauri/Cargo.toml)`);
  const home = join(out, 'home');
  if (existsSync(home)) throw new Error(`${home} exists; give an empty --out folder`);
  mkdirSync(join(home, 'data'), { recursive: true });
  writeFileSync(join(home, 'config.toml'), CONFIG);
  if (history) copyFileSync(history, join(home, 'data', 'history.db'));
  const report = { app, home, sections: [], changes: [], make_cards: null, unclicked: [] };
  const child = spawn(app, [], { env: { ...process.env, MARU_HOME: home, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` }, stdio: 'ignore' });
  try {
    const launcher = await page('launcher.html');
    await launcher.eval(`window.__TAURI_INTERNALS__.invoke('launcher_action', { action: 'settings' })`);
    const hub = await page('hub.html');
    await sleep(1500);
    await hub.eval(IN_PAGE);
    const titles = await hub.eval(`[...document.querySelectorAll('.settings-sidebar button')].map((b) => b.textContent.trim())`);
    for (const [i, id] of SECTIONS.entries()) {
      await hub.eval(`__e2e.go(${JSON.stringify(titles[i])})`);
      await sleep(400);
      await hub.shot(join(out, `settings-${id}.png`));
      report.sections.push({ id, title: titles[i], rows: (await hub.eval('__e2e.rows()')).length });
    }
    for (const c of CHANGES) {
      const at = SECTIONS.indexOf(c.section);
      await hub.eval(`__e2e.go(${JSON.stringify(titles[at])})`);
      await sleep(300);
      const [verb, value] = Object.entries(c.act)[0];
      await hub.eval(`__e2e.${verb}(${JSON.stringify(c.path)}, ${JSON.stringify(value)})`);
      await sleep(200);
      await hub.eval('__e2e.save()');
      let message = '';
      for (let k = 0; k < 40 && !/Saved|Nothing changed|could not|error/i.test(message); k++) { await sleep(250); message = await hub.eval('__e2e.message()'); }
      const file = readFileSync(join(home, 'config.toml'), 'utf8');
      report.changes.push({ section: c.section, path: c.path, message, written: c.expect.test(file) });
    }
    await hub.eval(`document.querySelector('button[data-tab=history]').click()`);
    await sleep(1200);
    // Starred lines first; Make cards is offered on any live line, so Live subtitles is the fallback.
    const chips = [];
    let opened = false;
    for (const name of ['Starred', 'Live subtitles']) {
      chips.push(await hub.eval(`(() => { const b = [...document.querySelectorAll('.chips .chip')].find((x) => x.textContent.includes(${JSON.stringify(name)})); b?.click(); return b?.textContent ?? null; })()`));
      await sleep(1200);
      opened = await hub.eval(`(() => { const b = document.querySelector('.mine-open'); if (!b) return false; b.click(); return true; })()`);
      if (opened) break;
    }
    let sheet = null;
    for (let k = 0; opened && k < 60 && !sheet; k++) { await sleep(500); sheet = await hub.eval(`(() => { const m = document.querySelector('.mine'); if (!m || m.querySelector('.mine-wait')) return null; return { words: m.querySelectorAll('input[type=checkbox]').length, text: m.textContent.slice(0, 160) }; })()`); }
    await hub.shot(join(out, 'make-cards.png'));
    report.make_cards = { chips, opened, sheet };
    report.unclicked = [
      'Connections: adding, editing, testing or removing a connection (it needs a real server address)',
      'Each task\'s Test button and the voice test buttons (they ask a model or a voice server)',
      'Make cards: Add to Anki, Retry with…, sounds (Anki is off in the temporary home)',
      'The restart button the page offers after saving',
      'Any window other than the launcher and the hub: panel, live band, scan labels, picker, wheel',
    ];
    await launcher.eval(`window.__TAURI_INTERNALS__.invoke('launcher_action', { action: 'quit' })`).catch(() => {});
  } finally {
    await sleep(2000);
    if (child.exitCode === null) child.kill();
    writeFileSync(join(out, 'report.json'), JSON.stringify(report, null, 2));
  }
  const ok = report.changes.every((c) => c.written) && report.sections.length === SECTIONS.length && report.make_cards?.sheet;
  console.log(JSON.stringify(report, null, 2));
  process.exit(ok ? 0 : 1);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'))) {
  main().catch((e) => { console.error(e.message); process.exit(2); });
}
