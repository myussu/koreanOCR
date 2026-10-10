// Reading text aloud, one sentence at a time.
//
// A paragraph is spoken as a queue: while sentence N plays, sentence N+1 is already being made, so
// there is no gap between them and the first words start as soon as one sentence exists. The class
// holds no DOM and no Tauri: it is given a way to fetch a clip and a way to play one, which is what
// the tests replace.

/** @typedef {{ mime: string, data: string }} Clip */

export class Reader {
  /**
   * @param {{
   *   fetchClip: (text: string) => Promise<Clip>,
   *   createAudio: (clip: Clip) => { play(): Promise<void>, pause(): void, release(): void, onended: (() => void) | null },
   *   onChange?: () => void,
   * }} io
   */
  constructor({ fetchClip, createAudio, onChange = () => {} }) {
    this.fetchClip = fetchClip;
    this.createAudio = createAudio;
    this.onChange = onChange;
    this.sentences = [];
    this.index = -1;
    /** 'idle' | 'loading' | 'playing' | 'paused' | 'error' */
    this.status = 'idle';
    this.error = null;
    /** How many times each sentence is said (1 = once). */
    this.repeat = 1;
    this.#run = 0;
    this.#clips = new Map();
    this.#audio = null;
    this.#heard = 0;
  }

  #run;
  #clips;
  #audio;
  #heard;

  get active() {
    return this.status === 'loading' || this.status === 'playing' || this.status === 'paused';
  }

  get text() {
    return this.sentences[this.index] ?? '';
  }

  #set(status, error = null) {
    this.status = status;
    this.error = error;
    this.onChange();
  }

  #fetch(i) {
    if (i < 0 || i >= this.sentences.length) return null;
    if (!this.#clips.has(i)) {
      const p = this.fetchClip(this.sentences[i]);
      // a failed clip must be asked for again next time, not remembered as failed
      p.catch(() => this.#clips.delete(i));
      this.#clips.set(i, p);
    }
    return this.#clips.get(i);
  }

  /** Start reading `sentences` from `from`, replacing whatever was being read. */
  start(sentences, from = 0) {
    this.#stopAudio();
    this.#run += 1;
    this.#clips = new Map();
    this.sentences = sentences.filter((s) => s.trim());
    if (!this.sentences.length) return this.#set('idle');
    return this.#play(Math.min(Math.max(from, 0), this.sentences.length - 1));
  }

  async #play(i) {
    const run = this.#run;
    this.#stopAudio();
    this.index = i;
    this.#heard = 0;
    this.#set('loading');
    let clip;
    try {
      clip = await this.#fetch(i);
    } catch (e) {
      if (run === this.#run) this.#set('error', String(e));
      return;
    }
    if (run !== this.#run || i !== this.index) return; // stopped, restarted or skipped meanwhile
    this.#fetch(i + 1)?.catch(() => {}); // the next sentence is made while this one plays
    const audio = this.createAudio(clip);
    this.#audio = audio;
    audio.onended = () => {
      if (run !== this.#run || this.#audio !== audio) return;
      this.#heard += 1;
      if (this.#heard < this.repeat) return this.#again(audio, run);
      if (i + 1 < this.sentences.length) this.#play(i + 1);
      else this.stop();
    };
    if (this.status === 'paused') return; // paused while it was loading: wait for resume
    try {
      await audio.play();
      if (run === this.#run && this.#audio === audio && this.status === 'loading') this.#set('playing');
    } catch (e) {
      if (run === this.#run) this.#set('error', `Playback was blocked: ${e?.message ?? e}`);
    }
  }

  async #again(audio, run) {
    try {
      await audio.play();
    } catch (e) {
      if (run === this.#run) this.#set('error', String(e?.message ?? e));
    }
  }

  #stopAudio() {
    if (this.#audio) {
      this.#audio.onended = null;
      this.#audio.pause();
      this.#audio.release();
      this.#audio = null;
    }
  }

  pause() {
    if (this.status === 'playing') {
      this.#audio?.pause();
      this.#set('paused');
    } else if (this.status === 'loading') {
      this.#set('paused'); // the clip that is on its way will wait
    }
  }

  async resume() {
    if (this.status !== 'paused') return;
    if (!this.#audio) return this.#play(this.index); // paused before its clip arrived
    this.#set('loading');
    try {
      await this.#audio.play();
      if (this.status === 'loading') this.#set('playing');
    } catch (e) {
      this.#set('error', String(e?.message ?? e));
    }
  }

  toggle() {
    if (this.status === 'playing' || this.status === 'loading') this.pause();
    else if (this.status === 'paused') this.resume();
  }

  next() {
    if (this.active && this.index + 1 < this.sentences.length) {
      this.#run += 1;
      this.#play(this.index + 1);
    }
  }

  prev() {
    if (this.active && this.index > 0) {
      this.#run += 1;
      this.#play(this.index - 1);
    }
  }

  /** Say the current sentence again from its start. */
  replay() {
    if (this.sentences.length && this.index >= 0) {
      this.#run += 1;
      this.#play(this.index);
    }
  }

  stop() {
    this.#stopAudio();
    this.#run += 1;
    this.#clips = new Map();
    this.index = -1;
    this.#set('idle');
  }

  setRepeat(n) {
    this.repeat = Math.max(1, Math.min(5, Math.round(n)));
    this.onChange();
  }
}
