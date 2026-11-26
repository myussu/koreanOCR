// The decisions of the extension, with no page and no browser API in them, so they are tested with `node --test`
// (test/captions-core.test.mjs): which caption track to read, what its file says, which line is up at a given
// moment of the video, when a line is sent and when it is not, and what the popup says.
//
// It is a plain script that puts one object on the global scope rather than an ES module: a content script and a
// service worker loaded from the manifest cannot `import`, and the same file is used by both and by the popup.
(function (root) {
  'use strict';

  /** A line is sent once it has been up this long, so lines passed over while dragging the seek bar are not. */
  const SETTLE_MS = 250;
  /** The video's time moving this far from where playback would have taken it is a seek. */
  const JUMP_MS = 1500;
  /** A cue without an end is taken to last until the next one, and at most this long. */
  const DEFAULT_CUE_MS = 4000;
  const DEFAULT_PORT = 18098;
  const HANGUL = /[가-힣]/;

  const isKorean = (code) => /^ko(-|$)/i.test(String(code ?? ''));

  /** The video a watch page shows, or null on any other page (the home page, a channel, Shorts). */
  function videoIdOf(href) {
    try {
      const url = new URL(href);
      if (url.hostname !== 'www.youtube.com' || url.pathname !== '/watch') return null;
      const id = url.searchParams.get('v');
      return id && /^[A-Za-z0-9_-]{6,32}$/.test(id) ? id : null;
    } catch {
      return null;
    }
  }

  function timedText(url) {
    try {
      const parsed = new URL(url, 'https://www.youtube.com');
      return parsed.protocol === 'https:' && parsed.hostname === 'www.youtube.com' && parsed.pathname === '/api/timedtext' ? parsed : null;
    } catch {
      return null;
    }
  }

  /** Only YouTube's own caption address is ever fetched, whatever the page hands over. */
  const isTimedTextUrl = (url) => timedText(url) !== null;

  /**
   * The address to fetch a track from: the one the page gave, as JSON, and never translated. `tlang` is how
   * YouTube asks for its automatic translation of a track, so an address carrying it is refused outright.
   */
  function trackUrl(baseUrl) {
    const url = timedText(baseUrl);
    if (!url || url.searchParams.has('tlang')) return null;
    url.searchParams.set('fmt', 'json3');
    return url.toString();
  }

  /**
   * Which track to read. Korean uploaded by the author first; the auto-generated Korean one (`kind: "asr"`) only
   * with the switch; never anything else. `reason` says why for the popup: `uploaded`, `asr`, `asr_off` (only an
   * auto-generated Korean track exists and the switch is off) or `none`.
   */
  function pickTrack(tracks, options = {}) {
    const usable = (Array.isArray(tracks) ? tracks : []).filter((t) => t && isKorean(t.languageCode) && trackUrl(t.baseUrl));
    const uploaded = usable.filter((t) => t.kind !== 'asr');
    if (uploaded.length) {
      // `ko` before `ko-KR` and the like; otherwise the order the page lists them in.
      const track = uploaded.find((t) => String(t.languageCode).toLowerCase() === 'ko') ?? uploaded[0];
      return { track, kind: 'manual', reason: 'uploaded' };
    }
    const auto = usable.find((t) => t.kind === 'asr');
    if (auto) return options.allowAsr ? { track: auto, kind: 'asr', reason: 'asr' } : { track: null, kind: null, reason: 'asr_off' };
    return { track: null, kind: null, reason: 'none' };
  }

  /**
   * The player's own request for this track, among the addresses the page has fetched. YouTube may answer a
   * caption request with an empty body unless it carries a token only its player can make; the player's own
   * address has it. Newest first; an address asking for a translation (`tlang`) is never used.
   */
  function playerUrlFor(fetched, track, videoId) {
    for (const name of [...(fetched ?? [])].reverse()) {
      const url = timedText(name);
      if (!url || url.searchParams.has('tlang')) continue;
      const kind = url.searchParams.get('kind') ?? '';
      if (url.searchParams.get('v') !== videoId || url.searchParams.get('lang') !== track.languageCode || kind !== (track.kind ?? '')) continue;
      return trackUrl(name);
    }
    return null;
  }

  /** One row of text: caption rows joined, spaces collapsed, invisible characters removed. */
  function cleanText(text) {
    return String(text ?? '').replace(/[\u200b-\u200d\u2060\ufeff]/g, '').replace(/\s+/g, ' ').trim();
  }

  /**
   * The cues of a caption file in YouTube's `json3` form: `{ events: [{ tStartMs, dDurationMs, segs: [{ utf8 }] }] }`.
   * Events without text (window definitions, the line breaks of auto-generated captions) and lines without any
   * Korean are left out. Anything unexpected gives an empty list: nothing is sent rather than something wrong.
   */
  function parseJson3(data) {
    const events = data && Array.isArray(data.events) ? data.events : [];
    const cues = [];
    for (const event of events) {
      if (!event || !Array.isArray(event.segs) || !Number.isFinite(event.tStartMs)) continue;
      const text = cleanText(event.segs.map((seg) => (seg && typeof seg.utf8 === 'string' ? seg.utf8 : '')).join(''));
      if (!text || !HANGUL.test(text)) continue;
      const duration = Number.isFinite(event.dDurationMs) && event.dDurationMs > 0 ? event.dDurationMs : null;
      cues.push({ startMs: Math.round(event.tStartMs), endMs: duration === null ? null : Math.round(event.tStartMs + duration), text });
    }
    cues.sort((a, b) => a.startMs - b.startMs);
    cues.forEach((cue, i) => {
      if (cue.endMs === null) cue.endMs = Math.min(cue.startMs + DEFAULT_CUE_MS, cues[i + 1]?.startMs ?? Infinity);
    });
    return cues;
  }

  /** The cues on screen at `tMs` of the video (two can overlap), in the order they start. */
  const activeCues = (cues, tMs) => cues.filter((cue) => cue.startMs <= tMs && tMs < cue.endMs);

  const keyOf = (cue) => `${cue.startMs}|${cue.text}`;

  /**
   * Decides, tick by tick, which lines to send. Every line of a video is sent at most once: winding back does
   * not send it again, and a line skipped by jumping forward is not sent until it is really played. While the
   * seek bar is dragged nothing is sent. `step` returns the cues to send now.
   */
  function createSender(options = {}) {
    const settleMs = options.settleMs ?? SETTLE_MS;
    const sent = new Set();
    /** Cues that are up but not sent yet: key → { cue, since }. */
    let pending = new Map();
    let last = null;
    return {
      step(cues, tMs, nowMs, playback = {}) {
        const expected = last === null || playback.paused ? 0 : (nowMs - last.now) * (playback.rate ?? 1);
        const jumped = last !== null && Math.abs(tMs - last.t - expected) > JUMP_MS;
        last = { t: tMs, now: nowMs };
        const active = activeCues(cues, tMs);
        const out = [];
        const still = new Map();
        for (const [key, entry] of pending) {
          if (active.some((cue) => keyOf(cue) === key)) still.set(key, entry);
          // It ended while the video simply played on: a short line, shown and gone between two looks. A line
          // that vanished because the time jumped was only passed over.
          else if (!jumped && !playback.seeking) {
            sent.add(key);
            out.push(entry.cue);
          }
        }
        pending = jumped || playback.seeking ? new Map() : still;
        if (playback.seeking) return out;
        for (const cue of active) {
          const key = keyOf(cue);
          if (sent.has(key)) continue;
          if (!pending.has(key)) pending.set(key, { cue, since: nowMs });
          if (nowMs - pending.get(key).since >= settleMs) {
            pending.delete(key);
            sent.add(key);
            out.push(cue);
          }
        }
        return out;
      },
      /** A line Maru never got (it was not running) may be tried again when it next comes up. */
      forget(cue) {
        sent.delete(keyOf(cue));
      },
      sentCount: () => sent.size,
    };
  }

  const short = (value, max) => (typeof value === 'string' ? value.slice(0, max) : undefined);

  /** The body of one request to Maru. Only these fields ever leave the browser. */
  function lineFor(cue, meta = {}) {
    return {
      text: short(cue.text, 2000),
      video: short(meta.videoId, 32),
      title: short(meta.title, 200),
      start_ms: Number.isFinite(cue.startMs) ? Math.max(0, Math.round(cue.startMs)) : undefined,
      kind: meta.kind === 'asr' ? 'asr' : 'manual',
      lang: short(meta.lang, 16),
    };
  }

  /** Where Maru listens: this PC, nothing else. A port that is not a port falls back to Maru's default. */
  function endpoint(port, path) {
    const n = Number(port);
    return `http://127.0.0.1:${Number.isInteger(n) && n >= 1024 && n <= 65535 ? n : DEFAULT_PORT}${path}`;
  }

  /** What Maru's answer means. `status` 0 = no answer at all (nothing is listening). */
  function resultOf(status, body) {
    if (status === 0) return { result: 'maru_off' };
    if (status === 401) return { result: 'bad_code' };
    if (status === 403) return { result: 'refused' };
    if (status === 200 && body && body.ok === true) {
      if (body.app === 'maru') return { result: 'ok' };
      return body.accepted ? { result: 'accepted' } : { result: 'dropped', reason: String(body.reason ?? '') };
    }
    return { result: 'error' };
  }

  const LINK = {
    no_code: "Paste the code from Maru's settings below (Settings → Live subtitles → Code for the browser extension).",
    maru_off: 'Maru is not listening. Start Maru and switch on "Take YouTube captions from the browser" in its settings.',
    bad_code: "Maru refused the code. Copy it again from Maru's settings.",
    refused: 'Maru refused this extension. Its settings name another extension as the only one allowed to send.',
    error: 'Something other than Maru answered on that port. Check the port here and in Maru.',
  };

  /** What the popup says about the tab's video (`state` from the content script) and about Maru (`link`). */
  function describe(state, link) {
    const phase = state?.phase ?? 'no_video';
    const lines = (n) => `${n} ${n === 1 ? 'line' : 'lines'}`;
    const video = {
      no_video: 'Open a YouTube video to send its Korean captions to Maru.',
      loading: "Looking for this video's captions…",
      none: 'No Korean captions on this video. Nothing is sent; Maru reads the screen as before.',
      asr_off: 'No Korean captions uploaded by the author on this video. It has auto-generated ones; turn on the switch below to use them.',
      unknown: "This video's caption list could not be read (YouTube may have changed its page). Nothing is sent; Maru reads the screen as before.",
      unreadable: "This video has Korean captions, but YouTube did not hand them over. Turn the video's captions (CC) on in Korean once and they will be read.",
      sending: `Sending this video's Korean captions (${state?.kind === 'asr' ? 'auto-generated' : 'uploaded by the author'}) to Maru: ${lines(state?.accepted ?? 0)} so far.`,
    }[phase] ?? 'Open a YouTube video to send its Korean captions to Maru.';
    return { video, link: LINK[link] ?? '', ok: phase === 'sending' && !LINK[link] };
  }

  root.MaruCaptions = { SETTLE_MS, JUMP_MS, DEFAULT_PORT, videoIdOf, isTimedTextUrl, trackUrl, pickTrack, playerUrlFor, cleanText, parseJson3, activeCues, createSender, lineFor, endpoint, resultOf, describe };
})(globalThis);
