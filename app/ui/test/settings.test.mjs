import test from 'node:test';
import assert from 'node:assert/strict';
import { SECTIONS, CATEGORIES, FEATURES, ALL_FIELDS, controlKind, isVisible, matchesSetting, appliesLive, labelFor, sectionOf, valueAt, toEditor, fromEditor, withEdit, shown, isDefault, withReset, splitKeys, joinKeys, keyName, comboFromEvent, keyRisk } from '../settings-core.js';

const current = { ui: { font_size_px: 18, theme: 'auto' }, learning: { desired_retention: 0.9 }, llm: { model_path: null, parallel: 2 }, hotkeys: { scan: 'Ctrl+Alt+S' } };
const defaults = { ui: { font_size_px: 18, theme: 'auto' }, learning: { desired_retention: 0.9 }, llm: { model_path: null, parallel: 2 }, hotkeys: { scan: 'Ctrl+Alt+S' } };
const field = (path) => ALL_FIELDS.find((f) => f.path === path);

test('the page is organised by flow: an overview, the seven sections asked for, in that order', () => {
  assert.deepEqual(CATEGORIES.map((c) => c.title), ['Overview', 'Live subtitles', 'Captures', 'Making cards', 'Voice', 'Connections', 'Shortcuts', 'Advanced']);
  assert.ok(SECTIONS.every((s) => s.fields.length && s.intro));
});

test('every setting has a unique path, a plain label, one sentence of help and the features that use it', () => {
  const paths = ALL_FIELDS.map((f) => f.path);
  assert.equal(new Set(paths).size, paths.length);
  const features = Object.values(FEATURES);
  for (const f of ALL_FIELDS) {
    assert.ok(f.label && f.help, `${f.path} has a label and help`);
    assert.ok(f.help.length <= 190 && !/\. [A-Z]/.test(f.help.replace(/e\.g\. |i\.e\. /g, '')), `${f.path}: one sentence of help, not a paragraph: ${f.help}`);
    assert.ok(f.uses?.length && f.uses.every((u) => features.includes(u)), `${f.path} says which features use it`);
    assert.ok(['toggle', 'choice', 'route', 'route_optional', 'number', 'percent', 'text', 'password', 'tags', 'keys'].includes(f.type), f.path);
    if (f.type === 'choice') assert.ok(f.options.length >= 2, f.path);
    if (f.type === 'number' || f.type === 'percent') assert.ok(f.min < f.max && f.step > 0, f.path);
  }
});

test('internal names are gone from what the page says', () => {
  const said = [...SECTIONS.map((s) => `${s.title} ${s.intro}`), ...ALL_FIELDS.map((f) => `${f.label} ${f.help} ${(f.options ?? []).map((o) => `${o.label} ${o.detail ?? ''}`).join(' ')}`)].join('\n');
  for (const jargon of ['Snapshot quality server', 'real-game trial', 'Gemma / main model', 'main model', 'snapshot server', 'Experimental:', 'endpoint']) {
    assert.ok(!said.toLowerCase().includes(jargon.toLowerCase()), `"${jargon}" is not shown to the person`);
  }
});

test('saving what is read and the enlargement sit with live subtitles, among the settings that rarely change', () => {
  const live = SECTIONS.find((s) => s.id === 'live').fields;
  for (const path of ['continuous.save_reads', 'continuous.ocr_scale']) {
    const f = live.find((x) => x.path === path);
    assert.ok(f?.advanced, `${path} is an advanced setting of Live subtitles`);
    assert.deepEqual(f.uses, [FEATURES.live]);
  }
  assert.equal(controlKind(field('continuous.save_reads')), 'switch');
  assert.equal(controlKind(field('continuous.ocr_scale')), 'typed');
});

test('there are three kinds of control, and a choice is never a drop-down', () => {
  assert.equal(controlKind(field('learning.gloss_mode')), 'choice');
  assert.equal(controlKind(field('tasks.live.connection')), 'choice');
  assert.equal(controlKind(field('tasks.capture.while_live')), 'choice');
  assert.equal(controlKind(field('reading.capture.reader')), 'switch');
  for (const path of ['learning.anki.deck', 'continuous.linger_ms', 'learning.anki.tags', 'hotkeys.scan', 'tts.api_key', 'learning.desired_retention']) assert.equal(controlKind(field(path)), 'typed', path);
  assert.ok(ALL_FIELDS.every((f) => f.type !== 'select' && f.type !== 'range'));
  assert.deepEqual([...new Set(ALL_FIELDS.map(controlKind))].sort(), ['choice', 'switch', 'typed']);
});

test('search finds a setting by its name, by what it does, and by the feature that uses it', () => {
  const hits = (q) => ALL_FIELDS.filter((f) => matchesSetting(f, q)).map((f) => f.path);
  assert.ok(hits('screen scan').includes('tasks.capture.connection') && hits('screen scan').includes('hotkeys.scan'), 'by feature name');
  assert.ok(hits('make cards').includes('tasks.card_meanings.connection') && hits('make cards').includes('learning.anki.deck'));
  assert.ok(hits('hover').includes('capture.ocr_radius_px'));
  assert.ok(hits('clipboard').includes('capture.use_clipboard_fallback'), 'by what it does');
  assert.ok(hits('voice').includes('learning.anki.audio'), 'by its section and what it uses');
  assert.deepEqual(hits('zzzz'), []);
  assert.equal(hits('').length, ALL_FIELDS.length);
});

test('each task picks its connection in the section of that flow, and connections are edited in one place', () => {
  assert.equal(sectionOf('tasks.live.connection'), 'live');
  assert.equal(sectionOf('tasks.capture.connection'), 'captures');
  assert.equal(sectionOf('tasks.capture.while_live'), 'captures');
  assert.equal(sectionOf('tasks.card_meanings.connection'), 'cards');
  assert.equal(sectionOf('tasks.card_sentence.connection'), 'cards');
  assert.equal(SECTIONS.find((s) => s.id === 'connections').custom, 'connections');
  // An address, model name or key of a server is not a field anywhere: only the connection editor has them.
  for (const path of ['llm.endpoint', 'llm.model_name', 'llm.api_key', 'llm.provider', 'continuous.translation.snapshot_endpoint', 'continuous.translation.snapshot_api_key', 'continuous.translation.model']) assert.equal(field(path), undefined, path);
});

test('what applies without a restart matches the backend: shortcuts, the panel, connections and the card tasks', () => {
  assert.ok(appliesLive('hotkeys.scan') && appliesLive('ui.theme') && appliesLive('llm.servers'));
  assert.ok(appliesLive('tasks.card_meanings.connection') && appliesLive('tasks.card_sentence.connection'));
  assert.ok(!appliesLive('tasks.live.connection') && !appliesLive('tasks.capture.connection'), 'these hold models and servers');
  assert.ok(!appliesLive('llm.hf_repo') && !appliesLive('lite') && !appliesLive('learning.gloss_mode') && !appliesLive('continuous.cpu_ocr'));
  assert.equal(labelFor('llm.ctx_size'), 'Room per request on this PC');
  assert.equal(labelFor('nope.x'), 'nope.x');
});

test('editing tracks only real changes', () => {
  let d = withEdit({}, current, 'ui.font_size_px', 22);
  assert.deepEqual(d, { 'ui.font_size_px': 22 });
  assert.equal(shown(d, current, defaults, 'ui.font_size_px'), 22);
  d = withEdit(d, current, 'ui.font_size_px', 18);
  assert.deepEqual(d, {}, 'back to what the file has: nothing to save');
});

test('reset sends null and shows the default', () => {
  const cur = { ...current, ui: { font_size_px: 24, theme: 'dark' } };
  assert.ok(!isDefault({}, cur, defaults, 'ui.font_size_px'));
  const d = withReset({}, cur, defaults, 'ui.font_size_px');
  assert.deepEqual(d, { 'ui.font_size_px': null });
  assert.equal(shown(d, cur, defaults, 'ui.font_size_px'), 18);
  assert.ok(isDefault(d, cur, defaults, 'ui.font_size_px'));
  assert.deepEqual(withReset({}, current, defaults, 'ui.font_size_px'), {}, 'already the default: nothing to do');
});

test('editor conversions: percent, nullable text, rounding', () => {
  const pct = ALL_FIELDS.find((f) => f.path === 'learning.desired_retention');
  assert.equal(toEditor(pct, 0.8999999761581421), 90);
  assert.equal(fromEditor(pct, 87), 0.87);
  const model = ALL_FIELDS.find((f) => f.path === 'llm.model_path');
  assert.equal(fromEditor(model, '  '), null, 'empty removes the key');
  assert.equal(fromEditor(model, ' D:/m.gguf '), 'D:/m.gguf');
  assert.equal(toEditor(model, null), '');
  const lite = ALL_FIELDS.find((f) => f.path === 'lite_profile.hf_repo');
  assert.equal(fromEditor(lite, ''), '', 'an empty lite model is a real value (keep the main model)');
  const temp = ALL_FIELDS.find((f) => f.path === 'llm.temperature');
  assert.equal(toEditor(temp, 0.20000000298023224), 0.2);
  assert.equal(valueAt(current, 'llm.parallel'), 2);
  assert.equal(valueAt(current, 'a.b.c'), undefined);
});

test('shortcut lists', () => {
  assert.deepEqual(splitKeys('Ctrl+Alt+T | F6'), ['Ctrl+Alt+T', 'F6']);
  assert.deepEqual(splitKeys(' | '), []);
  assert.equal(joinKeys(['Ctrl+Alt+T', 'F6']), 'Ctrl+Alt+T | F6');
  assert.equal(joinKeys(splitKeys('Ctrl+,')), 'Ctrl+,', 'the comma key stays');
});

const ev = (code, mods = {}) => ({ code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods });

test('key presses become shortcut names', () => {
  assert.equal(comboFromEvent(ev('KeyT', { ctrlKey: true, altKey: true })), 'Ctrl+Alt+T');
  assert.equal(comboFromEvent(ev('F7')), 'F7');
  assert.equal(comboFromEvent(ev('Digit5', { shiftKey: true })), 'Shift+5');
  assert.equal(comboFromEvent(ev('Numpad5')), 'Numpad5');
  assert.equal(comboFromEvent(ev('Escape', { ctrlKey: true, altKey: true })), 'Ctrl+Alt+Escape');
  assert.equal(comboFromEvent(ev('ArrowUp', { ctrlKey: true })), 'Ctrl+Up');
  assert.equal(comboFromEvent(ev('Comma', { ctrlKey: true })), 'Ctrl+,');
  assert.equal(comboFromEvent(ev('Escape')), 'cancel');
  assert.equal(comboFromEvent(ev('ControlLeft', { ctrlKey: true })), null, 'a modifier alone is not a shortcut yet');
  assert.equal(comboFromEvent(ev('ShiftRight', { shiftKey: true })), null);
  assert.equal(comboFromEvent(ev('IntlBackslash')), 'unknown');
  assert.equal(keyName('F24'), 'F24');
  assert.equal(keyName('F25'), null);
});

test('warns about keys that would be taken from every program', () => {
  assert.equal(keyRisk('Ctrl+Alt+T'), null);
  assert.equal(keyRisk('Pause'), null);
  assert.equal(keyRisk('Numpad5'), null);
  assert.equal(keyRisk('F15'), null);
  assert.match(keyRisk('F6'), /browsers/);
  assert.match(keyRisk('T'), /everywhere else/);
});

test('a setting that only matters for one choice of another shows only then', () => {
  const key = field('tts.api_key'), device = field('tts.device');
  const cur = { tts: { provider: 'local' } }, def = { tts: { provider: 'local' } };
  assert.ok(!isVisible(key, {}, cur, def) && isVisible(device, {}, cur, def));
  const draft = { 'tts.provider': 'endpoint' };
  assert.ok(isVisible(key, draft, cur, def) && !isVisible(device, draft, cur, def));
  assert.ok(isVisible(field('tts.enabled'), {}, cur, def), 'fields without a condition always show');
});

test('the live section shows the model on this PC only for this PC, and the backup only for a server', () => {
  const model = field('tasks.live.model'), backup = field('continuous.translation.backup_model'), file = field('continuous.translation.lfm_path'), device = field('continuous.translation.device');
  const local = { tasks: { live: { connection: 'local', model: '' } } };
  assert.ok(isVisible(model, {}, local, local) && !isVisible(backup, {}, local, local));
  assert.ok(!isVisible(file, {}, local, local) && !isVisible(device, {}, local, local), 'no file or processor choice for the full model');
  const small = { 'tasks.live.model': 'lfm' };
  assert.ok(isVisible(file, small, local, local) && isVisible(device, small, local, local));
  assert.ok(!isVisible(field('continuous.translation.milmmt_path'), small, local, local), 'only the chosen model asks for its file');
  const server = { 'tasks.live.connection': 'NAS' };
  assert.ok(!isVisible(model, server, local, local) && isVisible(backup, server, local, local));
  assert.ok(!isVisible(file, { ...server, 'tasks.live.model': 'lfm' }, local, local), 'a stale model name does not show a file for a server');
  assert.equal(model.options[0].value, '', "this PC's full model is the empty model name");
});

test('an API key is a secret, an empty one removes it, and tags are a comma list', () => {
  const key = field('tts.api_key');
  assert.equal(key.type, 'password');
  assert.equal(fromEditor(key, '  '), null);
  const tags = field('learning.anki.tags');
  assert.equal(toEditor(tags, ['maru', 'game']), 'maru, game');
  assert.deepEqual(fromEditor(tags, ' maru ,, game , '), ['maru', 'game']);
  assert.deepEqual(fromEditor(tags, ''), []);
  assert.equal(fromEditor(field('learning.anki.api_key'), '  '), null, 'an empty key removes it');
});
