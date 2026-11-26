// The content script: on a YouTube watch page it finds the Korean caption track the author uploaded, reads its
// cues, and as the video plays hands each line to the background worker, which posts it to Maru on this PC.
// Every decision (which track, which line, when) is in captions-core.js; this file only talks to the page.
//
// It fails safe: when there is no usable track, or anything YouTube-specific is not where it is expected, nothing
// is sent and the popup says why. Maru's own screen reading is not touched by any of this.
(() => {
  'use strict';
  const core = globalThis.MaruCaptions;
  const TAG = 'maru-captions';
  /** How often the video's time is looked at. */
  const TICK_MS = 100;
  /** The page's player is asked again this often, this many times, while a video is still loading. */
  const ASK_EVERY_MS = 500;
  const ASK_TIMES = 20;

  /** Each navigation starts a new run; whatever an older run was waiting for is dropped. */
  let run = 0;
  let state = { phase: 'no_video' };
  let link = null;
  let timer = null;
  let allowAsr = false;
  /** The sender of the video and track last read, and which they were. */
  let kept = { key: null, sender: null };
  /** Caption addresses the page itself has fetched (the player's own requests), oldest first. */
  const fetched = [];

  const alive = () => {
    try {
      return Boolean(chrome.runtime?.id);
    } catch {
      return false;
    }
  };

  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  function noteFetched(name) {
    if (!core.isTimedTextUrl(name) || fetched.includes(name)) return;
    fetched.push(name);
    if (fetched.length > 20) fetched.shift();
  }

  // The player's own caption requests carry a token only it can make (see `playerUrlFor`). An observer sees them
  // even when the page's list of fetched resources is full, which on YouTube it soon is.
  try {
    for (const entry of performance.getEntriesByType('resource')) noteFetched(entry.name);
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) noteFetched(entry.name);
    }).observe({ type: 'resource', buffered: true });
  } catch {
    /* without it only the direct request is tried */
  }

  /** Ask the page-world script (page-bridge.js) for the player's caption tracks. */
  function askPage() {
    return new Promise((resolve) => {
      const nonce = `${Date.now()}-${Math.random()}`;
      const done = (answer) => {
        window.removeEventListener('message', onMessage);
        clearTimeout(giveUp);
        resolve(answer);
      };
      const onMessage = (event) => {
        if (event.source !== window || event.origin !== location.origin) return;
        const data = event.data;
        if (data && data.source === TAG && data.type === 'tracks' && data.nonce === nonce) done(data);
      };
      const giveUp = setTimeout(() => done(null), 1000);
      window.addEventListener('message', onMessage);
      window.postMessage({ source: TAG, type: 'tracks?', nonce }, location.origin);
    });
  }

  /** The player's data for `videoId`, once it has loaded; null if it never shows up. */
  async function pageData(videoId, mine) {
    for (let i = 0; i < ASK_TIMES && mine === run; i++) {
      const answer = await askPage();
      // After a click on another video the player still describes the previous one for a moment.
      if (answer?.found && answer.videoId === videoId) return answer;
      await wait(ASK_EVERY_MS);
    }
    return null;
  }

  async function cuesFrom(url) {
    if (!url) return [];
    try {
      const response = await fetch(url, { credentials: 'include' });
      if (!response.ok) return [];
      const text = await response.text();
      // An empty answer is YouTube declining a request its player did not make.
      return text.trim() ? core.parseJson3(JSON.parse(text)) : [];
    } catch {
      return [];
    }
  }

  /** The track's cues: from the address the page lists, else from the player's own request for the same track. */
  async function loadCues(track, videoId) {
    const direct = await cuesFrom(core.trackUrl(track.baseUrl));
    return direct.length ? direct : cuesFrom(core.playerUrlFor(fetched, track, videoId));
  }

  function stop() {
    if (timer !== null) clearInterval(timer);
    timer = null;
  }

  async function start() {
    const mine = ++run;
    stop();
    const videoId = core.videoIdOf(location.href);
    if (!videoId) return void (state = { phase: 'no_video' });
    state = { phase: 'loading', videoId };
    const page = await pageData(videoId, mine);
    if (mine !== run) return;
    if (!page) return void (state = { phase: 'unknown', videoId });
    const picked = core.pickTrack(page.tracks, { allowAsr });
    if (!picked.track) return void (state = { phase: picked.reason, videoId });

    const meta = { videoId, title: page.title, kind: picked.kind, lang: picked.track.languageCode };
    let cues = await loadCues(picked.track, videoId);
    if (mine !== run) return;
    if (!cues.length) state = { phase: 'unreadable', videoId, kind: picked.kind };
    // Not handed over: wait for the person to turn the video's captions on, then use the player's own request.
    // Each address is asked once, so a video whose captions stay unreadable costs YouTube nothing more.
    const tried = new Set();
    while (!cues.length && mine === run && alive()) {
      await wait(2000);
      const url = core.playerUrlFor(fetched, picked.track, videoId);
      if (mine !== run) return;
      if (!url || tried.has(url)) continue;
      tried.add(url);
      cues = await cuesFrom(url);
    }
    if (mine !== run || !cues.length) return;

    // The same video and track again (the switch was flipped back, YouTube announced the page twice): keep
    // what was already sent, so no line goes out a second time.
    const key = `${videoId}|${picked.track.languageCode}|${picked.kind}`;
    if (kept.key !== key) kept = { key, sender: core.createSender() };
    const sender = kept.sender;
    state = { phase: 'sending', videoId, kind: picked.kind, accepted: 0, sent: 0 };
    const current = state;
    timer = setInterval(() => {
      if (mine !== run) return;
      if (!alive()) return stop(); // the extension was reloaded or removed: this page's copy is orphaned
      if (core.videoIdOf(location.href) !== videoId) return void start(); // the page changed without telling us
      const player = document.getElementById('movie_player');
      const video = player?.querySelector('video') ?? document.querySelector('video');
      // During an advertisement the same <video> plays the ad: its time is not the video's.
      if (!video || player?.classList.contains('ad-showing')) return;
      const due = sender.step(cues, video.currentTime * 1000, performance.now(), { seeking: video.seeking, paused: video.paused, rate: video.playbackRate });
      for (const cue of due) send(cue, meta, sender, current);
    }, TICK_MS);
  }

  async function send(cue, meta, sender, current) {
    current.sent += 1;
    let answer;
    try {
      answer = await chrome.runtime.sendMessage({ type: 'line', line: core.lineFor(cue, meta) });
    } catch {
      return; // the extension was reloaded mid-request
    }
    link = answer?.result ?? 'error';
    if (link === 'accepted') current.accepted += 1;
    // Maru did not get it (not running, or the code is wrong): let it be sent if it comes up again.
    else if (link !== 'dropped') sender.forget(cue);
  }

  chrome.runtime.onMessage.addListener((message, _sender, respond) => {
    if (message?.type !== 'status?') return;
    respond({ state, link });
  });

  chrome.storage.local.get({ allowAsr: false }).then((saved) => {
    allowAsr = Boolean(saved.allowAsr);
    start();
  });
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area !== 'local' || !changes.allowAsr) return;
    allowAsr = Boolean(changes.allowAsr.newValue);
    start();
  });
  // YouTube changes videos without loading a new page and announces it with this event. If it ever stops doing
  // so, the check inside the timer and this slow one still notice the address changing.
  document.addEventListener('yt-navigate-finish', () => start());
  setInterval(() => {
    if (alive() && core.videoIdOf(location.href) !== (state.videoId ?? null)) start();
  }, 2000);
})();
