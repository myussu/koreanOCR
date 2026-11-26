import test from 'node:test';
import assert from 'node:assert/strict';
import '../captions-core.js';

const core = globalThis.MaruCaptions;
const base = (query) => `https://www.youtube.com/api/timedtext?v=dQw4w9WgXcQ&${query}`;
const track = (languageCode, kind, extra = '') => ({ languageCode, ...(kind ? { kind } : {}), baseUrl: base(`lang=${languageCode}${kind ? `&kind=${kind}` : ''}${extra}`), name: languageCode });

// ───────────── which track ─────────────

test('the Korean track the author uploaded is read, before any auto-generated one', () => {
  const picked = core.pickTrack([track('en'), track('ko', 'asr'), track('ko'), track('ja')]);
  assert.equal(picked.reason, 'uploaded');
  assert.equal(picked.kind, 'manual');
  assert.equal(picked.track.kind, undefined);
  // With the switch on, the uploaded one still wins.
  assert.equal(core.pickTrack([track('ko', 'asr'), track('ko')], { allowAsr: true }).reason, 'uploaded');
  // `ko` before a regional variant, whatever the order; a variant alone is still Korean.
  assert.equal(core.pickTrack([track('ko-KR'), track('ko')]).track.languageCode, 'ko');
  assert.equal(core.pickTrack([track('en'), track('ko-KR')]).track.languageCode, 'ko-KR');
});

test('auto-generated Korean is used only with the switch on', () => {
  const tracks = [track('en'), track('ko', 'asr')];
  assert.deepEqual(core.pickTrack(tracks), { track: null, kind: null, reason: 'asr_off' });
  assert.deepEqual(core.pickTrack(tracks, { allowAsr: false }), { track: null, kind: null, reason: 'asr_off' });
  const on = core.pickTrack(tracks, { allowAsr: true });
  assert.equal(on.reason, 'asr');
  assert.equal(on.kind, 'asr');
});

test('a video without a Korean track gives nothing, whatever else it has', () => {
  for (const tracks of [[], null, undefined, 'x', [track('en'), track('ja', 'asr')], [null, {}, { languageCode: 'ko' }], [track('kok')]]) {
    assert.deepEqual(core.pickTrack(tracks, { allowAsr: true }), { track: null, kind: null, reason: 'none' });
  }
});

test('a translated track is never used: not picked, not fetched, not taken from the player', () => {
  // YouTube translates a track when its address carries `tlang`.
  const translated = track('en', null, '&tlang=ko');
  const fake = { ...track('ko', null, '&tlang=ko') };
  assert.equal(core.pickTrack([translated, fake], { allowAsr: true }).reason, 'none');
  assert.equal(core.trackUrl(base('lang=en&tlang=ko')), null);
  assert.equal(core.trackUrl(base('lang=ko&tlang=ko')), null);
  const fetched = [base('lang=ko&pot=abc&tlang=en'), base('lang=en&pot=abc&tlang=ko')];
  assert.equal(core.playerUrlFor(fetched, track('ko'), 'dQw4w9WgXcQ'), null);
});

test("only YouTube's own caption address is fetched, as JSON", () => {
  const url = core.trackUrl(base('lang=ko&fmt=srv3&signature=abc'));
  assert.ok(url.startsWith('https://www.youtube.com/api/timedtext?'));
  assert.equal(new URL(url).searchParams.get('fmt'), 'json3');
  assert.equal(new URL(url).searchParams.get('signature'), 'abc', 'the rest of the address is kept');
  assert.equal(core.trackUrl('/api/timedtext?v=x&lang=ko').startsWith('https://www.youtube.com/api/timedtext'), true, 'a relative address is YouTube\'s');
  for (const elsewhere of ['https://evil.example/api/timedtext?lang=ko', 'http://www.youtube.com/api/timedtext?lang=ko', 'https://www.youtube.com/watch?v=x', 'https://www.youtube.com.evil.example/api/timedtext', 'javascript:alert(1)', '', null, 42]) {
    assert.equal(core.trackUrl(elsewhere), null, String(elsewhere));
    assert.equal(core.isTimedTextUrl(elsewhere), false, String(elsewhere));
  }
  assert.equal(core.pickTrack([{ languageCode: 'ko', baseUrl: 'https://evil.example/api/timedtext?lang=ko' }]).reason, 'none');
});

test("the player's own request for the chosen track is found among the page's fetches, newest first", () => {
  const fetched = [
    'https://www.youtube.com/youtubei/v1/player',
    base('lang=en&pot=old&c=WEB'),
    base('lang=ko&kind=asr&pot=auto&c=WEB'),
    base('lang=ko&pot=first&c=WEB'),
    'https://www.youtube.com/api/timedtext?v=otherVideo1&lang=ko&pot=other',
    base('lang=ko&pot=newest&c=WEB&fmt=srv3'),
  ];
  const manual = new URL(core.playerUrlFor(fetched, track('ko'), 'dQw4w9WgXcQ'));
  assert.equal(manual.searchParams.get('pot'), 'newest');
  assert.equal(manual.searchParams.get('fmt'), 'json3');
  assert.equal(new URL(core.playerUrlFor(fetched, track('ko', 'asr'), 'dQw4w9WgXcQ')).searchParams.get('pot'), 'auto');
  assert.equal(core.playerUrlFor(fetched, track('ja'), 'dQw4w9WgXcQ'), null);
  assert.equal(core.playerUrlFor(fetched, track('ko'), 'anotherVideo'), null);
  assert.equal(core.playerUrlFor(undefined, track('ko'), 'dQw4w9WgXcQ'), null);
});

test('only a watch page has a video', () => {
  assert.equal(core.videoIdOf('https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s'), 'dQw4w9WgXcQ');
  for (const href of ['https://www.youtube.com/', 'https://www.youtube.com/shorts/dQw4w9WgXcQ', 'https://www.youtube.com/watch', 'https://www.youtube.com/watch?v=<script>', 'https://example.com/watch?v=dQw4w9WgXcQ', 'not a url']) {
    assert.equal(core.videoIdOf(href), null, href);
  }
});

// ───────────── what the caption file says ─────────────

test('an uploaded track becomes one cue per line, rows joined, without lines that have no Korean', () => {
  const cues = core.parseJson3({
    events: [
      { tStartMs: 1000, dDurationMs: 2500, segs: [{ utf8: '안녕하세요,\n여러분.' }] },
      { tStartMs: 4000, dDurationMs: 1500, segs: [{ utf8: '[Music]' }] },
      { tStartMs: 6000, dDurationMs: 2000, segs: [{ utf8: '  오늘은\u200b  ' }, { utf8: '한국어를 배워요' }] },
      { tStartMs: 9000, dDurationMs: 1000, segs: [{ utf8: '\n' }] },
    ],
  });
  assert.deepEqual(cues, [
    { startMs: 1000, endMs: 3500, text: '안녕하세요, 여러분.' },
    { startMs: 6000, endMs: 8000, text: '오늘은 한국어를 배워요' },
  ]);
});

test('an auto-generated track gives its lines without the window and line-break events', () => {
  const cues = core.parseJson3({
    events: [
      { tStartMs: 0, dDurationMs: 600000, id: 1, wpWinPosId: 1, wsWinStyleId: 1 },
      { tStartMs: 80, dDurationMs: 4000, wWinId: 1, segs: [{ utf8: '안녕하세요', acAsrConf: 0 }, { utf8: ' 여러분', tOffsetMs: 640 }] },
      { tStartMs: 2070, dDurationMs: 2010, wWinId: 1, aAppend: 1, segs: [{ utf8: '\n' }] },
      { tStartMs: 2080, dDurationMs: 4300, wWinId: 1, segs: [{ utf8: '오늘도' }, { utf8: ' 반갑습니다', tOffsetMs: 500 }] },
    ],
  });
  assert.deepEqual(cues.map((c) => [c.startMs, c.text]), [[80, '안녕하세요 여러분'], [2080, '오늘도 반갑습니다']]);
});

test('a cue without an end lasts until the next one, and cues come out in order', () => {
  const cues = core.parseJson3({ events: [{ tStartMs: 5000, segs: [{ utf8: '둘째 줄' }] }, { tStartMs: 1000, segs: [{ utf8: '첫째 줄' }] }, { tStartMs: 20000, dDurationMs: 0, segs: [{ utf8: '마지막 줄' }] }] });
  assert.deepEqual(cues, [{ startMs: 1000, endMs: 5000, text: '첫째 줄' }, { startMs: 5000, endMs: 9000, text: '둘째 줄' }, { startMs: 20000, endMs: 24000, text: '마지막 줄' }]);
});

test('a file that is not what is expected gives no cues instead of something wrong', () => {
  for (const data of [null, undefined, '', 'text', 42, {}, { events: 'x' }, { events: [null, 1, {}, { segs: [{ utf8: '안녕' }] }, { tStartMs: 'soon', segs: [{ utf8: '안녕' }] }, { tStartMs: 10, segs: 'x' }, { tStartMs: 10, segs: [null, { utf8: 5 }] }] }]) {
    assert.deepEqual(core.parseJson3(data), [], JSON.stringify(data));
  }
});

// ───────────── which line, and when it is sent ─────────────

const CUES = [
  { startMs: 1000, endMs: 3000, text: '첫째 줄' },
  { startMs: 3000, endMs: 5000, text: '둘째 줄' },
  { startMs: 8000, endMs: 10000, text: '셋째 줄' },
  { startMs: 9000, endMs: 11000, text: '넷째 줄 (겹침)' },
];

test('the cue that is up at a moment of the video, none in a gap, two when they overlap', () => {
  const at = (t) => core.activeCues(CUES, t).map((c) => c.text);
  assert.deepEqual(at(0), []);
  assert.deepEqual(at(1000), ['첫째 줄']);
  assert.deepEqual(at(2999), ['첫째 줄']);
  assert.deepEqual(at(3000), ['둘째 줄'], 'the end is not part of a cue');
  assert.deepEqual(at(6000), []);
  assert.deepEqual(at(9500), ['셋째 줄', '넷째 줄 (겹침)']);
  assert.deepEqual(at(60000), []);
});

/** Play the video in real time from `from` to `to` (ms), looking every 100 ms; returns what was sent. */
function play(sender, clock, from, to, playback = {}) {
  const sent = [];
  for (let t = from; t <= to; t += 100) {
    sent.push(...sender.step(CUES, t, clock.now, playback).map((c) => c.text));
    clock.now += 100;
  }
  return sent;
}

test('each line is sent once, a moment after it comes up, in order', () => {
  const sender = core.createSender();
  const clock = { now: 50_000 };
  assert.deepEqual(sender.step(CUES, 1000, clock.now), [], 'not on the first look');
  clock.now += 100;
  assert.deepEqual(sender.step(CUES, 1100, clock.now), []);
  clock.now += 200;
  assert.deepEqual(sender.step(CUES, 1300, clock.now).map((c) => c.text), ['첫째 줄'], `after ${core.SETTLE_MS} ms`);
  clock.now += 100;
  assert.deepEqual(play(sender, clock, 1400, 12000), ['둘째 줄', '셋째 줄', '넷째 줄 (겹침)']);
  assert.equal(sender.sentCount(), 4);
});

test('winding back does not send a line again, and a line skipped by jumping ahead is sent when it is played', () => {
  const sender = core.createSender();
  const clock = { now: 0 };
  assert.deepEqual(play(sender, clock, 900, 2000), ['첫째 줄']);
  // Jump ahead over the second line.
  assert.deepEqual(play(sender, clock, 7900, 12000), ['셋째 줄', '넷째 줄 (겹침)']);
  // Wind back to the start: the first line was sent, the second never was.
  assert.deepEqual(play(sender, clock, 900, 6000), ['둘째 줄']);
  // And again: nothing is left to send.
  assert.deepEqual(play(sender, clock, 0, 12000), []);
});

test('lines passed over while the seek bar is dragged are not sent', () => {
  const sender = core.createSender();
  const clock = { now: 0 };
  // The time jumps from line to line faster than playback could.
  for (const t of [1500, 3500, 8500, 9500, 1500, 3500]) {
    assert.deepEqual(sender.step(CUES, t, clock.now, { seeking: true }), []);
    clock.now += 100;
  }
  // The same without the browser saying "seeking" (a click on the bar): the jump itself is noticed.
  for (const t of [1500, 8500, 3500, 9500]) {
    assert.deepEqual(sender.step(CUES, t, clock.now), []);
    clock.now += 100;
  }
  assert.equal(sender.sentCount(), 0);
  // Left to play where it landed, the line that is up is sent.
  assert.deepEqual(play(sender, clock, 9600, 9900), ['셋째 줄', '넷째 줄 (겹침)']);
});

test('a line shorter than the wait is still sent when the video simply plays past it', () => {
  const sender = core.createSender();
  const quick = [{ startMs: 1000, endMs: 1150, text: '네!' }];
  assert.deepEqual(sender.step(quick, 900, 0), []);
  assert.deepEqual(sender.step(quick, 1050, 150), [], 'up, but not for long enough yet');
  assert.deepEqual(sender.step(quick, 1200, 300).map((c) => c.text), ['네!'], 'gone again: it was shown');
  assert.deepEqual(sender.step(quick, 1050, 1000), [], 'and not sent again after winding back');
});

test('a paused video sends the line it stopped on once, and faster playback is not taken for a seek', () => {
  const sender = core.createSender();
  const clock = { now: 0 };
  const sent = [];
  for (let i = 0; i < 50; i++) {
    sent.push(...sender.step(CUES, 1500, clock.now, { paused: true }).map((c) => c.text));
    clock.now += 100;
  }
  assert.deepEqual(sent, ['첫째 줄']);
  // Double speed: 200 ms of video per 100 ms look.
  const fast = core.createSender();
  const heard = [];
  for (let t = 0; t <= 12000; t += 200) {
    heard.push(...fast.step(CUES, t, clock.now, { rate: 2 }).map((c) => c.text));
    clock.now += 100;
  }
  assert.deepEqual(heard, ['첫째 줄', '둘째 줄', '셋째 줄', '넷째 줄 (겹침)']);
});

test('a line Maru never got can be sent when it comes up again', () => {
  const sender = core.createSender();
  const clock = { now: 0 };
  assert.deepEqual(play(sender, clock, 900, 2000), ['첫째 줄']);
  sender.forget(CUES[0]);
  assert.deepEqual(play(sender, clock, 900, 2000), ['첫째 줄']);
});

// ───────────── what is sent, and what Maru answers ─────────────

test('a request carries the line and where it is from, and nothing else', () => {
  const line = core.lineFor({ startMs: 12340.4, endMs: 15000, text: '안녕하세요, 여러분.' }, { videoId: 'dQw4w9WgXcQ', title: '한국어 수업', kind: 'manual', lang: 'ko', cookie: 'secret', url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ' });
  assert.deepEqual(JSON.parse(JSON.stringify(line)), { text: '안녕하세요, 여러분.', video: 'dQw4w9WgXcQ', title: '한국어 수업', start_ms: 12340, kind: 'manual', lang: 'ko' });
  assert.equal(core.lineFor({ startMs: 0, text: '가' }, { kind: 'asr' }).kind, 'asr');
  assert.equal(core.lineFor({ startMs: 0, text: '가' }, { kind: 'anything' }).kind, 'manual');
  assert.equal(core.lineFor({ startMs: 0, text: '가' }, { title: '제'.repeat(500) }).title.length, 200);
});

test('lines only ever go to this PC', () => {
  assert.equal(core.endpoint(18098, '/captions/v1/line'), 'http://127.0.0.1:18098/captions/v1/line');
  assert.equal(core.endpoint('19000', '/captions/v1/hello'), 'http://127.0.0.1:19000/captions/v1/hello');
  for (const port of [80, 0, -1, 70000, 1.5, 'evil.example', '18098@evil.example', null, undefined, NaN]) {
    assert.equal(core.endpoint(port, '/x'), `http://127.0.0.1:${core.DEFAULT_PORT}/x`, String(port));
  }
});

test("Maru's answers are told apart: taken, dropped, wrong code, not there", () => {
  assert.deepEqual(core.resultOf(200, { ok: true, accepted: true }), { result: 'accepted' });
  assert.deepEqual(core.resultOf(200, { ok: true, accepted: false, reason: 'duplicate' }), { result: 'dropped', reason: 'duplicate' });
  assert.deepEqual(core.resultOf(200, { ok: true, app: 'maru', protocol: 1 }), { result: 'ok' });
  assert.deepEqual(core.resultOf(401, { ok: false }), { result: 'bad_code' });
  assert.deepEqual(core.resultOf(403, null), { result: 'refused' });
  assert.deepEqual(core.resultOf(0, null), { result: 'maru_off' });
  for (const [status, body] of [[200, null], [200, { hello: 'world' }], [404, null], [500, { ok: false }], [503, { ok: false }]]) {
    assert.deepEqual(core.resultOf(status, body), { result: 'error' });
  }
});

test('the popup says in words why nothing is sent', () => {
  assert.match(core.describe({ phase: 'none' }).video, /^No Korean captions on this video\./);
  assert.match(core.describe({ phase: 'none' }).video, /Maru reads the screen as before/);
  assert.match(core.describe({ phase: 'asr_off' }).video, /No Korean captions uploaded by the author.*auto-generated.*switch/);
  assert.match(core.describe({ phase: 'unknown' }).video, /could not be read.*Nothing is sent/);
  assert.match(core.describe({ phase: 'unreadable' }).video, /captions \(CC\) on in Korean/);
  assert.match(core.describe(null).video, /^Open a YouTube video/);
  assert.match(core.describe({ phase: 'something new' }).video, /^Open a YouTube video/);
  assert.equal(core.describe({ phase: 'sending', kind: 'manual', accepted: 1 }).video, "Sending this video's Korean captions (uploaded by the author) to Maru: 1 line so far.");
  assert.equal(core.describe({ phase: 'sending', kind: 'asr', accepted: 12 }).video, "Sending this video's Korean captions (auto-generated) to Maru: 12 lines so far.");
});

test('the popup says what is wrong between the extension and Maru', () => {
  assert.equal(core.describe({ phase: 'sending', accepted: 3 }, 'accepted').link, '');
  assert.equal(core.describe({ phase: 'sending', accepted: 3 }, 'accepted').ok, true);
  assert.match(core.describe({ phase: 'sending' }, 'no_code').link, /Paste the code/);
  assert.match(core.describe({ phase: 'sending' }, 'maru_off').link, /Maru is not listening/);
  assert.match(core.describe({ phase: 'sending' }, 'bad_code').link, /refused the code/);
  assert.match(core.describe({ phase: 'sending' }, 'refused').link, /another extension/);
  assert.match(core.describe({ phase: 'sending' }, 'error').link, /port/);
  assert.equal(core.describe({ phase: 'sending' }, 'maru_off').ok, false);
  assert.equal(core.describe({ phase: 'none' }, 'ok').ok, false);
});
