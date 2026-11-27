// Which connection Settings recommends for each step of the flow, and why, from what was measured on this PC
// (docs/reports/run-1.md and run-2.md). Pure functions, tested in settings-advice.test.mjs.
//
// The advice is about kinds of model, not names: a connection is recognised by where it runs and its model's family,
// so it follows the connections the person saved. Where nothing was measured the advice says so.

import { LOCAL, connections } from './settings-flow-core.js';

/** The model family of a connection, from its model name. */
export function family(model) {
  const m = String(model ?? '').toLowerCase();
  if (m.includes('sonnet')) return 'sonnet';
  if (m.includes('haiku')) return 'haiku';
  if (m.includes('gemma')) return 'gemma';
  if (/gpt-4\.1-mini|gpt-4o-mini/.test(m)) return 'gpt-mini';
  if (m.includes('gemini')) return 'gemini';
  return 'other';
}

/** What run 1 and run 2 measured, by task and family: one sentence each, in the page's words. */
export const MEASURED = {
  live: {
    gemma: 'about 0.4 s a line in run 1, the fastest measured',
    haiku: 'about 0.7 s a line in run 1',
    'gpt-mini': 'about 0.9 s a line in run 1',
    sonnet: 'about 1.6 s a line in run 1, with slow outliers',
    gemini: 'the free key ran out of quota after a few lines in run 1',
  },
  capture: {
    gemma: 'about 0.2–0.3 s a sentence in run 2',
    haiku: 'about 0.6 s a sentence in run 2',
    sonnet: 'about 0.8 s a sentence in run 2',
    'gpt-mini': 'about 1 s a sentence in run 2',
    gemini: 'the free key ran out of quota in runs 1 and 2',
  },
  card_meanings: {
    sonnet: '88 % right in your blind review (run 1)',
    gemma: '83 % right in your blind review (run 1)',
    'gpt-mini': '81 % right in your blind review (run 1)',
    haiku: '77 % right in your blind review (run 1)',
    gemini: 'could not be judged: the free key ran out of quota',
  },
};

const pick = (all, test) => all.find(test) ?? null;
const ready = (c) => c.ready !== false;

/**
 * The recommended connection for `task` (`live`, `capture`, `while_live`, `card_meanings`, `card_sentence`) among
 * the person's connections, as `{ value, why }`; `null` when none of them was measured for it.
 */
export function recommendFor(task, values) {
  const all = connections(values).filter(ready).map((c) => ({ ...c, family: family(c.model) }));
  const local = pick(all, (c) => c.name === LOCAL && c.family === 'gemma') ?? pick(all, (c) => c.name === LOCAL);
  const nasGemma = pick(all, (c) => c.place === 'network' && c.family === 'gemma');
  const sonnet = pick(all, (c) => c.family === 'sonnet');
  const haiku = pick(all, (c) => c.family === 'haiku');
  const say = (c, sentence) => c && { value: c.name, why: sentence };
  switch (task) {
    case 'live':
      return say(nasGemma, `${MEASURED.live.gemma}, as fast as this PC, and the game keeps this PC's graphics card`)
        ?? say(local, `${MEASURED.live.gemma}; it shares the graphics card with the game`)
        ?? say(haiku, MEASURED.live.haiku);
    case 'capture':
      return say(local, `${MEASURED.capture.gemma} and free; while live subtitles run, the setting below sends captures elsewhere`)
        ?? say(nasGemma, MEASURED.capture.gemma)
        ?? say(haiku, MEASURED.capture.haiku);
    case 'while_live':
      return say(nasGemma, "so this PC never swaps models in the middle of a game; it is as fast as this PC's model")
        ?? say(haiku, `${MEASURED.capture.haiku}, and nothing is loaded on this PC`);
    case 'card_meanings':
      return say(local, `${MEASURED.card_meanings.gemma} and free; Claude Sonnet scored 88 %, a difference the review could not prove`)
        ?? say(nasGemma, `${MEASURED.card_meanings.gemma}, the same model as this PC's, and free`)
        ?? say(sonnet, MEASURED.card_meanings.sonnet);
    case 'card_sentence':
      return { value: '', why: 'no extra request; how well each model translates whole sentences was not scored yet' };
    default:
      return null;
  }
}

/** The task a route field chooses for, as `recommendFor` names it. */
export function adviceTask(path) {
  return { 'tasks.live.connection': 'live', 'tasks.capture.connection': 'capture', 'tasks.capture.while_live': 'while_live', 'tasks.card_meanings.connection': 'card_meanings', 'tasks.card_sentence.connection': 'card_sentence' }[path] ?? null;
}

/** What a connection was measured at for a task, for its card ("about 0.4 s a line in run 1"); `null` if nothing. */
export function measuredFor(task, model) {
  const table = MEASURED[task === 'while_live' ? 'capture' : task];
  return table?.[family(model)] ?? null;
}
