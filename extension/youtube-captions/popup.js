// The popup (also the options page): the code, the port and the one switch, and what is happening on the tab in
// front. What it says is decided in captions-core.js (`describe`).
(() => {
  'use strict';
  const core = globalThis.MaruCaptions;
  const el = (id) => document.getElementById(id);
  const RESULT = {
    ok: 'Maru answered: the code is right.',
    no_code: 'Paste the code first.',
    maru_off: 'Maru is not listening on that port.',
    bad_code: 'Maru refused the code.',
    refused: 'Maru refused this extension.',
    error: 'Something other than Maru answered.',
  };

  async function load() {
    const saved = await chrome.storage.local.get({ token: '', port: core.DEFAULT_PORT, allowAsr: false });
    el('token').value = saved.token;
    el('port').value = saved.port;
    el('asr').checked = Boolean(saved.allowAsr);
    el('origin').textContent = `chrome-extension://${chrome.runtime.id}`;
  }

  async function save() {
    const port = Number(el('port').value);
    await chrome.storage.local.set({
      token: el('token').value.trim(),
      port: Number.isInteger(port) && port >= 1024 && port <= 65535 ? port : core.DEFAULT_PORT,
      allowAsr: el('asr').checked,
    });
    await load();
  }

  /** What the content script of the tab in front says; nothing when that tab is not a YouTube page. */
  async function tabStatus() {
    try {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      return tab?.id === undefined ? null : await chrome.tabs.sendMessage(tab.id, { type: 'status?' });
    } catch {
      return null;
    }
  }

  async function draw() {
    const status = await tabStatus();
    const { token } = await chrome.storage.local.get({ token: '' });
    const said = core.describe(status?.state, token ? status?.link : 'no_code');
    el('video').textContent = said.video;
    el('video').className = said.ok ? 'ok' : '';
    el('link').textContent = said.link;
  }

  el('save').addEventListener('click', async () => {
    await save();
    el('result').textContent = 'Saved.';
    draw();
  });
  el('test').addEventListener('click', async () => {
    await save();
    el('result').textContent = 'Asking Maru…';
    const answer = await chrome.runtime.sendMessage({ type: 'hello' }).catch(() => null);
    el('result').textContent = RESULT[answer?.result] ?? RESULT.error;
  });

  load().then(draw);
  setInterval(draw, 1000);
})();
