// "Make cards" in History: the words of a line (live or captured), each with its meaning in that line, become Anki word cards (with
// a reverse card if wanted) and the line itself a sentence card. The decisions are in mine-core.js.

import { invoke } from './bridge.js';
import { soundsText } from './anki.js';
import { addLabel, ankiIsOff, answeredText, applySense, askFor, deckOptions, formChoice, meaningHint, mineBlocker, mineDraft, mineRequest, outcomeSummary, queueLines, redraft, retryChoices, senseIndex, senseLabel, typedFormNote } from './mine-core.js';

let lists = 0;

/** The sheet for one history item, built with the page's `h`; `onAdded` runs once cards were made. */
export function mineSheet(h, item, { onClose, onAdded }) {
  const root = h('div', { class: 'mine', role: 'region', 'aria-label': 'Make Anki cards from this line' });
  let sheet = null;
  let draft = null;
  let busy = false;
  let reply = null;
  let error = null;
  let asking = null; // "Retry with …" in progress: what the sheet says meanwhile
  let sentenceBy = null; // who translated the sentence for the card, when a connection did
  let countTimer = null;
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
      sentenceTools(),
      sheet.note && h('p', { class: 'mine-note warn', text: sheet.note }),
      sheet.words.length
        ? h('div', { class: 'mine-words' }, sheet.words.map((w, i) => wordRow(w, draft.words[i], i)))
        : h('p', { class: 'mine-note', text: 'No words to learn were found in this line; it can still be a sentence card.' }),
      sheet.words.length > 0 && meaningTools(),
      h('label', { class: 'mine-check' },
        h('input', { type: 'checkbox', checked: draft.sentenceCard, onchange: (e) => { draft.sentenceCard = e.target.checked; draw(); } }),
        h('span', { text: sheet.sentence_in_anki ? 'Sentence card (Anki already has this line)' : 'Sentence card: the whole line with its sound, its translation and the words above' })),
      h('div', { class: 'mine-opts' },
        h('label', {}, h('span', { text: 'Deck for the words' }),
          h('input', { type: 'text', value: draft.deck, list: ids.decks, oninput: (e) => { draft.deck = e.target.value; refresh(); recount(); } }),
          h('datalist', { id: ids.decks }, deckOptions(sheet).map((d) => h('option', { value: d.deck, label: d.label })))),
        h('label', {}, h('span', { text: 'Deck for the sentence' }),
          h('input', { type: 'text', value: draft.sentenceDeck, list: ids.decks, oninput: (e) => { draft.sentenceDeck = e.target.value; refresh(); recount(); } })),
        h('label', { class: 'mine-check', title: 'An English → Korean card for each word, to write it from its meaning' },
          h('input', { type: 'checkbox', checked: draft.reverse, onchange: (e) => { draft.reverse = e.target.checked; draw(); } }), h('span', { text: 'Reverse cards for words' })),
        h('label', { class: 'mine-check' },
          h('input', { type: 'checkbox', checked: draft.audio, onchange: (e) => { draft.audio = e.target.checked; } }), h('span', { text: 'Sound' }))),
      h('div', { class: 'mine-queue' }, queueLines(sheet.queue, draft).map((line) => h('p', { text: line }))),
      h('div', { class: 'mine-go' },
        h('button', { class: 'btn primary', disabled: busy || !!blocker, onclick: add, text: busy ? 'Adding…' : label.text }),
        h('span', { class: 'mine-why', text: busy ? 'Making sounds and cards… the voice can take a minute to start.' : blocker ?? label.cards }),
        ankiIsOff(sheet) && turnOn()),
      reply && results(),
      error && h('div', { class: 'err', text: error }),
      sheet.credit && h('p', { class: 'mine-credit', text: sheet.credit }),
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
    root.querySelector('.mine-queue')?.replaceChildren(...queueLines(sheet.queue, draft).map((line) => h('p', { text: line })));
  };

  // A deck was typed over: count the new cards waiting there (a moment after the last key, and only by reading Anki).
  const recount = () => {
    clearTimeout(countTimer);
    countTimer = setTimeout(async () => {
      try {
        sheet.queue = await invoke('mine_queue', { wordDeck: draft.deck.trim(), sentenceDeck: (draft.sentenceDeck || draft.deck).trim() });
      } catch { sheet.queue = null; }
      refresh();
    }, 500);
  };

  // Who chose the meanings, and the connections the line can be asked again with.
  const meaningTools = () => h('div', { class: 'mine-by' },
    h('span', { class: 'mine-by-who', text: asking ?? answeredText(sheet) }),
    !asking && retryChoices(sheet).filter((c) => !c.current || c.afterLive).map((c) => h('button', { class: 'btn', type: 'button', title: c.detail, text: c.label, onclick: () => ask(c) })));

  async function ask(choice) {
    asking = choice.afterLive ? 'Waiting for live subtitles to stop, then asking…' : `Asking ${choice.connection === 'local' ? "this PC's model" : choice.connection}…`;
    draw();
    try {
      const again = await invoke('mine_sheet', { sentence: item.source, translation: draft.translation, app: item.app ?? null, ask: askFor(choice) });
      draft = redraft(draft, sheet, again);
      sheet = again;
    } catch (e) {
      error = String(e);
    }
    asking = null;
    draw();
  }

  // The sentence card's translation: the line's own, or a fresh one from a connection.
  const sentenceTools = () => {
    const choices = retryChoices(sheet).filter((c) => !c.afterLive);
    if (!choices.length) return null;
    const out = h('span', { class: 'mine-by-who', text: sentenceBy ?? (item.live ? 'The translation made while you played.' : 'The translation made when you captured it.') });
    return h('div', { class: 'mine-by' }, out, choices.map((c) => h('button', { class: 'btn', type: 'button', title: c.detail, text: c.label.replace('Retry with', 'Translate again with'), onclick: async (e) => {
      e.target.disabled = true;
      out.textContent = 'Translating…';
      const r = await invoke('mine_sentence_translation', { sentence: item.source, app: item.app ?? null, ask: askFor(c) }).catch((err) => ({ translation: null, note: String(err) }));
      if (r.translation) {
        draft.translation = r.translation;
        sentenceBy = `Translated for the card by ${r.answered?.connection === 'local' ? "this PC's model" : r.answered?.connection ?? c.connection}.${r.note ? ` ${r.note}` : ''}`;
      } else sentenceBy = r.note ?? 'No translation came back; the line keeps the one it had.';
      draw();
    } })));
  };

  const head = () => h('div', { class: 'mine-head' }, h('strong', { text: 'Make cards' }),
    h('button', { class: 'icon', title: 'Close', 'aria-label': 'Close', text: '✕', onclick: onClose }));

  const wordRow = (w, d, i) => {
    const list = `mine-senses-${lists}-${i}`;
    const facts = [w.pos, w.level, w.known ? 'known' : '', w.in_anki ? 'in Anki' : ''].filter(Boolean).join(' · ');
    return h('div', { class: d.picked ? 'mine-word on' : 'mine-word' },
      h('input', { type: 'checkbox', checked: d.picked, 'aria-label': `Make a card for ${w.key}`, onchange: (e) => { d.picked = e.target.checked; draw(); } }),
      h('div', { class: 'mine-key' }, d.editing ? formInput(w, d) : [h('span', { class: 'ko', text: d.key }), h('button', { class: 'icon fix', type: 'button', title: 'Fix the dictionary form, when the word was split wrongly', 'aria-label': `Fix the dictionary form of ${d.key}`, text: '✎', onclick: () => { d.editing = true; draw(); root.querySelector(`[data-form="${i}"]`)?.focus(); } })],
        w.surface !== d.key && h('small', { class: 'ko', text: w.surface }), forms(w, d), typedFormNote(w, d) && h('small', { class: 'typed-form', text: typedFormNote(w, d) })),
      h('div', { class: 'mine-meaning' },
        h('input', { type: 'text', value: d.meaning, list, placeholder: 'meaning', 'aria-label': `Meaning of ${w.key}`, oninput: (e) => { d.meaning = e.target.value; refresh(); } }),
        w.dictionary.length > 0 && h('datalist', { id: list }, w.dictionary.map((g) => h('option', { value: g }))),
        // With the learner's dictionary every card also carries a Spanish meaning.
        (sheet.credit || d.meaningEs) && h('input', { type: 'text', class: 'es', lang: 'es', value: d.meaningEs, placeholder: 'significado en español', 'aria-label': `Spanish meaning of ${w.key}`, oninput: (e) => { d.meaningEs = e.target.value; } }),
        (w.senses?.length ?? 0) > 1 && senses(w, d),
        h('small', { text: [meaningHint(w), facts].filter(Boolean).join(' · ') })));
  };

  // The dictionary form typed by hand: the card is made for it (Enter or leaving the box keeps it).
  const formInput = (w, d) => h('input', { type: 'text', class: 'ko form-edit', lang: 'ko', value: d.key, 'data-form': String(draft.words.indexOf(d)), 'aria-label': `Dictionary form of ${w.surface}`,
    oninput: (e) => { d.key = e.target.value.trim(); refresh(); },
    onkeydown: (e) => { if (e.key === 'Enter' || e.key === 'Escape') { if (e.key === 'Escape') d.key = w.key; d.editing = false; draw(); } },
    onblur: () => { if (d.editing) { d.editing = false; draw(); } } });

  // The model reads another dictionary form than the word splitter: both are shown, and a button switches the card.
  const forms = (w, d) => {
    const choice = formChoice(w, d);
    return choice && h('div', { class: 'mine-form' }, h('small', { text: choice.text }),
      h('button', { class: 'btn', type: 'button', text: choice.button, onclick: () => { d.key = choice.next; draw(); } }));
  };

  // The dictionary's other senses of a word: choosing one changes both meanings.
  const senses = (w, d) => {
    const at = senseIndex(w, d);
    const select = h('select', { class: 'mine-senses', 'aria-label': `Dictionary senses of ${w.key}`, onchange: (e) => { const s = w.senses[Number(e.target.value)]; if (s) { applySense(d, s); draw(); } } },
      at < 0 && h('option', { value: '', text: 'edited by hand — choose a dictionary sense…' }),
      w.senses.map((s, n) => h('option', { value: String(n), text: `${n + 1}. ${senseLabel(s)}` })));
    select.value = at < 0 ? '' : String(at);
    return select;
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
    // With a connection chosen for the sentence (Settings → Making cards) the card gets a fresh translation.
    if (sheet) {
      const r = await invoke('mine_sentence_translation', { sentence: item.source, app: item.app ?? null }).catch(() => null);
      if (r?.translation) {
        draft.translation = r.translation;
        sentenceBy = `Translated for the card by ${r.answered?.connection === 'local' ? "this PC's model" : r.answered?.connection}.${r.note ? ` ${r.note}` : ''}`;
        draw();
      } else if (r?.note) { sentenceBy = r.note; draw(); }
    }
  })();
  return root;
}
