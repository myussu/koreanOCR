// The background worker: the only part that talks to Maru. A content script's own requests would be made in the
// name of the YouTube page (and stopped by the browser); the worker's are made in the extension's name, to the one
// address the manifest allows: http://127.0.0.1.
//
// It keeps nothing between requests except what the popup saved (the code, the port, the switch).
importScripts('captions-core.js');

const core = globalThis.MaruCaptions;

async function post(path, body) {
  const { token, port } = await chrome.storage.local.get({ token: '', port: core.DEFAULT_PORT });
  if (!token) return { result: 'no_code' };
  let response;
  try {
    response = await fetch(core.endpoint(port, path), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Maru-Token': token },
      body: JSON.stringify(body),
      // Never follow a redirect or send anything but what is above.
      redirect: 'error',
      credentials: 'omit',
      cache: 'no-store',
    });
  } catch {
    return core.resultOf(0, null); // nothing is listening
  }
  return core.resultOf(response.status, await response.json().catch(() => null));
}

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  // Only this extension's own pages and content scripts are listened to.
  if (sender.id !== chrome.runtime.id) return;
  if (message?.type === 'line' && message.line && typeof message.line.text === 'string') {
    // Rebuilt field by field: whatever else a message carries does not leave the browser.
    const line = core.lineFor({ text: message.line.text, startMs: message.line.start_ms }, { videoId: message.line.video, title: message.line.title, kind: message.line.kind, lang: message.line.lang });
    post('/captions/v1/line', line).then(respond);
    return true; // the answer comes later
  }
  if (message?.type === 'hello') {
    post('/captions/v1/hello', {}).then(respond);
    return true;
  }
});
