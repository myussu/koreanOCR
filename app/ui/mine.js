// "Make cards" in History: the words of a live line, each with its meaning in that line, become Anki word cards (with
// a reverse card if wanted) and the line itself a sentence card. The decisions are in mine-core.js.

import { invoke } from './bridge.js';
import { soundsText } from './anki.js';
import { addLabel, ankiIsOff, deckOptions, mineBlocker, mineDraft, mineRequest, outcomeSummary } from './mine-core.js';

let lists = 0;

/** The sheet for one history item, built with the page's `h`; `onAdded` runs once cards were made. */
export function mineSheet(h, item, { onClose, onAdded }) {
  const root = h('div', { class: 'mine', role: 'region', 'aria-label': 'Make Anki cards from this line' });
  let sheet = null;
  let draft = null;
  let busy = false;
  let reply = null;
  let error = null;
  const ids = { decks: `mine-decks-${++lists}` };

  // replaceChildren would print a skipped part as "null": keep only real nodes.
  const put = (...kids) => root.replaceChildren(...kids.filter(Boolean));
  const draw = () => {
    if (error && !sheet) return put(head(), h('div', { class: 'err', text: error }));
    if (!sheet) return put(head(), h('p', { class: 'mine-wait', text: 'Finding each word and its meaning in this line…' }));
    const blocker = mineBlocker(draft, sheet);
    const label = addLabel(draft);
    put(
      head(),
      h('div', { class: 'mine-line ko', text: sheet.sentence }),
      h('label', { class: 'mine-tr' }, h('span', { text: 'Translation' }),
        h('input', { type: 'text', value: draft.translation, oninput: (e) => { draft.translation = e.target.value; } })),
      sheet.note && h('p', { class: 'mine-note', text: sheet.note }),
      sheet.words.length
        ? h('div', { class: 'mine-words' }, sheet.words.map((w, i) => wordRow(w, draft.words[i], i)))
        : h('p', { class: 'mine-note', text: 'No words to learn were found in this line; it can still be a sentence card.' }),
      h('label', { class: 'mine-check' },
        h('input', { type: 'checkbox', checked: draft.sentenceCard, onchange: (e) => { draft.sentenceCard = e.target.checked; draw(); } }),
        h('span', { text: sheet.sentence_in_anki ? 'Sentence card (Anki already has this line)' : 'Sentence card: the whole line with its sound, its translation and the words above' })),
      h('div', { class: 'mine-opts' },
        h('label', {}, h('span', { text: 'Deck' }),
          h('input', { type: 'text', value: draft.deck, list: ids.decks, oninput: (e) => { draft.deck = e.target.value; refresh(); } }),
          h('datalist', { id: ids.decks }, deckOptions(sheet).map((d) => h('option', { value: d.deck, label: d.label })))),
        h('label', { class: 'mine-check', title: 'An English → Korean card for each word, to write it from its meaning' },
          h('input', { type: 'checkbox', checked: draft.reverse, onchange: (e) => { draft.reverse = e.target.checked; draw(); } }), h('span', { text: 'Reverse cards for words' })),
        h('label', { class: 'mine-check' },
          h('input', { type: 'checkbox', checked: draft.audio, onchange: (e) => { draft.audio = e.target.checked; } }), h('span', { text: 'Sound' }))),
      h('div', { class: 'mine-go' },
        h('button', { class: 'btn primary', disabled: busy || !!blocker, onclick: add, text: busy ? 'Adding…' : label.text }),
        h('span', { class: 'mine-why', text: busy ? 'Making sounds and cards… the voice can take a minute to start.' : blocker ?? label.cards }),
        ankiIsOff(sheet) && turnOn()),
      reply && results(),
      error && h('div', { class: 'err', text: error }),
    );
  };

  // Typing in a text box must not rebuild the sheet (it would lose the caret); only the button's state follows.
  const refresh = () => {
    const button = root.querySelector('.mine-go button');
    const why = root.querySelector('.mine-why');
    if (!button || !why) return;
    const blocker = mineBlocker(draft, sheet);
    button.disabled = busy || !!blocker;
    button.textContent = busy ? 'Adding…' : addLabel(draft).text;
    why.textContent = blocker ?? addLabel(draft).cards;
  };

  const head = () => h('div', { class: 'mine-head' }, h('strong', { text: 'Make cards' }),
    h('button', { class: 'icon', title: 'Close', 'aria-label': 'Close', text: '✕', onclick: onClose }));

  const wordRow = (w, d, i) => {
    const list = `mine-senses-${lists}-${i}`;
    const facts = [w.pos, w.level, w.known ? 'known' : '', w.in_anki ? 'in Anki' : ''].filter(Boolean).join(' · ');
    return h('div', { class: d.picked ? 'mine-word on' : 'mine-word' },
      h('input', { type: 'checkbox', checked: d.picked, 'aria-label': `Make a card for ${w.key}`, onchange: (e) => { d.picked = e.target.checked; draw(); } }),
      h('div', { class: 'mine-key' }, h('span', { class: 'ko', text: w.key }), w.surface !== w.key && h('small', { class: 'ko', text: w.surface })),
      h('div', { class: 'mine-meaning' },
        h('input', { type: 'text', value: d.meaning, list, placeholder: 'meaning', 'aria-label': `Meaning of ${w.key}`, oninput: (e) => { d.meaning = e.target.value; refresh(); } }),
        w.dictionary.length > 0 && h('datalist', { id: list }, w.dictionary.map((g) => h('option', { value: g }))),
        h('small', { text: [w.in_context ? 'as used in this line' : 'from the dictionary', facts].filter(Boolean).join(' · ') })));
  };

  const results = () => h('div', { class: 'mine-results' },
    h('strong', { text: outcomeSummary(reply) }),
    reply.audio_note && h('p', { class: 'mine-note' }, reply.audio_note, ' ', fillButton()),
    h('ul', {}, reply.outcomes.map((o) => h('li', { class: o.result },
      h('span', { class: 'mark', text: o.result === 'added' ? '✓' : o.result === 'exists' ? '•' : '✕' }),
      h('span', { class: o.kind === 'sentence' ? '' : 'ko', text: o.kind === 'sentence' ? 'Sentence card' : o.what }),
      h('small', { text: o.detail })))));

  // Anki is off in Maru: one setting and a restart. The first press says what will happen; the second does it.
  const turnOn = () => {
    let sure = false;
    const out = h('span', { class: 'mine-why' });
    const button = h('button', { class: 'btn', text: 'Turn on Anki', onclick: async () => {
      if (!sure) {
        sure = true;
        button.textContent = 'Restart Maru now';
        out.textContent = 'Saves "Review in Anki" and restarts Maru; open History again afterwards. Anki must be open with AnkiConnect.';
        return;
      }
      button.disabled = true;
      try {
        await invoke('save_settings', { changes: { 'learning.anki.enabled': true } });
        await invoke('restart_app');
      } catch (e) {
        out.textContent = String(e);
        button.disabled = false;
      }
    } });
    return h('span', {}, button, ' ', out);
  };

  // Cards came out without sound: once speech works, their sounds can be added right here.
  const fillButton = () => {
    const out = h('span', { class: 'mine-why' });
    const button = h('button', { class: 'btn', text: 'Add missing sounds', onclick: async () => {
      button.disabled = true;
      out.textContent = ' Making sounds… the voice can take a minute to start.';
      try { out.textContent = ` ${soundsText(await invoke('anki_fill_sounds'))}`; } catch (e) { out.textContent = ` ${e}`; }
      button.disabled = false;
    } });
    return h('span', {}, button, out);
  };

  async function add() {
    if (busy || mineBlocker(draft, sheet)) return;
    busy = true;
    error = null;
    reply = null;
    draw();
    try {
      reply = await invoke('mine_add', { request: mineRequest(draft, item, sheet) });
      if (reply.outcomes.some((o) => o.result !== 'failed')) onAdded?.();
    } catch (e) {
      error = String(e);
    }
    busy = false;
    draw();
  }

  (async () => {
    draw();
    try {
      sheet = await invoke('mine_sheet', { sentence: item.source, translation: item.translation, app: item.app ?? null });
      draft = mineDraft(sheet);
    } catch (e) {
      error = String(e);
    }
    draw();
  })();
  return root;
}
