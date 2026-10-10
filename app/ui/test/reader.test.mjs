import test from 'node:test';
import assert from 'node:assert/strict';
import { Reader } from '../reader.js';
import { readableText, sentenceRanges, emptyState, reduce } from '../core.js';

/** A fake world: clips arrive when released, "audio" ends when the test says so. */
function world({ failFetch = [] } = {}) {
  const w = { fetched: [], pending: new Map(), audios: [], plays: 0, changes: [] };
  w.fetchClip = (text) => {
    w.fetched.push(text);
    return new Promise((resolve, reject) => {
      w.pending.set(text, { resolve: () => resolve({ mime: 'audio/wav', data: text }), reject });
      if (failFetch.includes(text)) reject(new Error(`no voice for ${text}`));
    });
  };
  w.createAudio = (clip) => {
    const a = { clip, paused: false, released: false, onended: null, played: 0 };
    a.play = async () => {
      a.paused = false;
      a.played += 1;
      w.plays += 1;
    };
    a.pause = () => (a.paused = true);
    a.release = () => (a.released = true);
    a.finish = () => a.onended?.();
    w.audios.push(a);
    return a;
  };
  w.arrive = async (text) => {
    w.pending.get(text).resolve();
    await tick();
  };
  return w;
}
const tick = () => new Promise((r) => setTimeout(r, 0));
const make = (w) => new Reader({ fetchClip: w.fetchClip, createAudio: w.createAudio, onChange: () => w.changes.push('x') });

test('plays sentences in order and fetches the next while one plays', async () => {
  const w = world();
  const r = make(w);
  r.start(['하나.', '둘.', '셋.']);
  assert.equal(r.status, 'loading');
  await w.arrive('하나.');
  assert.equal(r.status, 'playing');
  assert.deepEqual(w.fetched, ['하나.', '둘.'], 'the second is already on its way');
  await w.arrive('둘.');
  w.audios[0].finish();
  await tick();
  assert.equal(r.index, 1);
  assert.deepEqual(w.fetched, ['하나.', '둘.', '셋.']);
  assert.equal(w.audios[0].released, true, 'a finished clip is released');
  await w.arrive('셋.');
  w.audios[1].finish();
  await tick();
  w.audios[2].finish();
  await tick();
  assert.equal(r.status, 'idle', 'the end of the text ends the reading');
});

test('pause and resume keep the same clip; pausing while loading waits for the clip', async () => {
  const w = world();
  const r = make(w);
  r.start(['가.', '나.']);
  await w.arrive('가.');
  r.pause();
  assert.equal(r.status, 'paused');
  assert.equal(w.audios[0].paused, true);
  await r.resume();
  assert.equal(r.status, 'playing');
  assert.equal(w.audios.length, 1, 'no new clip was made');

  const w2 = world();
  const r2 = make(w2);
  r2.start(['다.']);
  r2.pause(); // before the clip exists
  await w2.arrive('다.');
  assert.equal(r2.status, 'paused');
  assert.equal(w2.plays, 0, 'nothing plays while paused');
  await r2.resume();
  await tick();
  assert.equal(r2.status, 'playing');
  assert.equal(w2.plays, 1);
});

test('skipping forward and back, and going back does not fetch again', async () => {
  const w = world();
  const r = make(w);
  r.start(['가.', '나.', '다.']);
  await w.arrive('가.');
  await w.arrive('나.');
  r.next();
  await tick();
  assert.equal(r.index, 1);
  assert.equal(w.audios[0].released, true, 'the skipped clip stops');
  r.prev();
  await tick();
  assert.equal(r.index, 0);
  assert.equal(w.fetched.filter((t) => t === '가.').length, 1, 'the first sentence was fetched once');
  r.prev();
  assert.equal(r.index, 0, 'cannot go before the start');
});

test('stopping cancels clips that are still on their way', async () => {
  const w = world();
  const r = make(w);
  r.start(['가.', '나.']);
  r.stop();
  await w.arrive('가.');
  assert.equal(r.status, 'idle');
  assert.equal(w.audios.length, 0, 'a late clip is never played');
});

test('starting again replaces the old reading', async () => {
  const w = world();
  const r = make(w);
  r.start(['옛날.']);
  r.start(['새것.']);
  await w.arrive('옛날.');
  await w.arrive('새것.');
  assert.equal(w.audios.length, 1);
  assert.equal(w.audios[0].clip.data, '새것.');
  assert.equal(r.text, '새것.');
});

test('a sentence can be repeated for practice', async () => {
  const w = world();
  const r = make(w);
  r.setRepeat(3);
  r.start(['가.', '나.']);
  await w.arrive('가.');
  w.audios[0].finish();
  await tick();
  w.audios[0].finish();
  await tick();
  assert.equal(r.index, 0, 'still on the first sentence');
  assert.equal(w.audios[0].played, 3, 'said three times in all: the third is playing now');
  w.audios[0].finish();
  await tick();
  assert.equal(r.index, 1, 'then on to the next');
  assert.equal(new Reader({ fetchClip: w.fetchClip, createAudio: w.createAudio }).repeat, 1);
});

test('a failed clip shows an error and can be retried', async () => {
  const w = world({ failFetch: ['가.'] });
  const r = make(w);
  r.start(['가.']);
  await tick();
  assert.equal(r.status, 'error');
  assert.match(r.error, /no voice/);
  w.fetchClip = (t) => Promise.resolve({ mime: 'audio/wav', data: t });
  r.fetchClip = w.fetchClip;
  r.replay();
  await tick();
  await tick();
  assert.equal(r.status, 'playing');
});

test('blocked playback is reported, not swallowed', async () => {
  const w = world();
  const r = make(w);
  w.createAudio = (clip) => ({ clip, onended: null, pause() {}, release() {}, play: () => Promise.reject(new Error('NotAllowedError')) });
  r.createAudio = w.createAudio;
  r.start(['가.']);
  await w.arrive('가.');
  assert.equal(r.status, 'error');
  assert.match(r.error, /blocked/);
});

test('empty text reads nothing', () => {
  const w = world();
  const r = make(w);
  r.start(['  ', '']);
  assert.equal(r.status, 'idle');
  assert.equal(w.fetched.length, 0);
});

// ───────────── which words are being said ─────────────

const e = (surface) => ({ surface });

test('sentences map onto consecutive words, ignoring punctuation and spacing', () => {
  const words = [e('안녕하세요.'), e('만나서'), e('반갑습니다.'), e('저는'), e('학생이에요!')];
  assert.deepEqual(sentenceRanges(words, ['안녕하세요.', '만나서 반갑습니다.', '저는 학생이에요!']), [[0, 1], [1, 3], [3, 5]]);
  const split = [e('안녕하세요'), e('.'), e('네'), e('?')];
  assert.deepEqual(sentenceRanges(split, ['안녕하세요.', '네?']), [[0, 2], [2, 4]], 'a punctuation-only word stays with its sentence');
});

test('words the analyser split differently still line up when they spell the same', () => {
  const words = [e('학교'), e('에서'), e('공부해요')];
  assert.deepEqual(sentenceRanges(words, ['학교에서 공부해요']), [[0, 3]]);
});

test('anything that does not line up gives no highlighting at all', () => {
  assert.equal(sentenceRanges([e('가'), e('나')], ['다 라']), null);
  assert.equal(sentenceRanges([e('가')], ['가 나']), null, 'more words said than shown');
  assert.deepEqual(sentenceRanges([], []), []);
});

test('what to read: the captured Korean, or the Korean that was written', () => {
  let s = reduce(emptyState(), { type: 'started', id: 1, direction: 'ko_to_en', capture: { text: ' 안녕하세요 ' }, cached: false });
  assert.equal(readableText(s), '안녕하세요');
  s = reduce(emptyState(), { type: 'started', id: 2, direction: 'en_to_ko', capture: { text: 'Hello' }, cached: false });
  s = reduce(s, { type: 'translation', id: 2, variant: 'primary', delta: '안녕하세요' });
  assert.equal(readableText(s), '안녕하세요');
  assert.equal(readableText(emptyState()), '');
});
