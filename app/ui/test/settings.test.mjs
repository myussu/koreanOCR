import test from 'node:test';
import assert from 'node:assert/strict';
import { ANKI, isVisible, SPEECH, SECTIONS, ALL_FIELDS, appliesLive, labelFor, valueAt, toEditor, fromEditor, withEdit, shown, isDefault, withReset, splitKeys, joinKeys, keyName, comboFromEvent, keyRisk } from '../settings-core.js';

const current = { ui: { font_size_px: 18, theme: 'auto' }, learning: { desired_retention: 0.9 }, llm: { model_path: null, parallel: 2 }, hotkeys: { scan: 'Ctrl+Alt+S' } };
const defaults = { ui: { font_size_px: 18, theme: 'auto' }, learning: { desired_retention: 0.9 }, llm: { model_path: null, parallel: 2 }, hotkeys: { scan: 'Ctrl+Alt+S' } };

test('live OCR and request experiments need a restart and do not edit the general OCR reader', () => {
  const fields = SECTIONS.find((s) => s.id === 'live').fields;
  for (const path of ['continuous.cpu_ocr', 'continuous.max_in_flight', 'continuous.filter_ocr_jitter']) {
    assert.ok(fields.some((f) => f.path === path));
    assert.equal(appliesLive(path), false);
  }
  const edited = withEdit({}, { continuous: { cpu_ocr: false } }, 'continuous.cpu_ocr', true);
  assert.deepEqual(edited, { 'continuous.cpu_ocr': true });
  assert.deepEqual(withEdit({}, { continuous: { filter_ocr_jitter: false } }, 'continuous.filter_ocr_jitter', true), { 'continuous.filter_ocr_jitter': true });
});

test('every field has a unique path, a label and a known type', () => {
  const paths = ALL_FIELDS.map((f) => f.path);
  assert.equal(new Set(paths).size, paths.length);
  for (const f of ALL_FIELDS) {
    assert.ok(f.label && ['toggle', 'select', 'number', 'range', 'percent', 'text', 'password', 'tags', 'keys'].includes(f.type), f.path);
    if (f.type === 'select') assert.ok(f.options.length >= 2, f.path);
    if (f.type === 'range' || f.type === 'number' || f.type === 'percent') assert.ok(f.min < f.max && f.step > 0, f.path);
  }
  assert.ok(SECTIONS.every((s) => s.fields.length));
});

test('live settings are the shortcuts and the panel, as in the backend', () => {
  assert.ok(appliesLive('hotkeys.scan') && appliesLive('ui.theme'));
  assert.ok(!appliesLive('llm.hf_repo') && !appliesLive('lite') && !appliesLive('learning.gloss_mode'));
  assert.equal(labelFor('llm.ctx_size'), 'Context per request');
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

test('speech settings: server fields only show for a server, local ones for the local voice', () => {
  const key = SPEECH.fields.find((f) => f.path === 'tts.api_key');
  const device = SPEECH.fields.find((f) => f.path === 'tts.device');
  const cur = { tts: { provider: 'local' } };
  const def = { tts: { provider: 'local' } };
  assert.ok(!isVisible(key, {}, cur, def) && isVisible(device, {}, cur, def));
  const draft = { 'tts.provider': 'endpoint' };
  assert.ok(isVisible(key, draft, cur, def) && !isVisible(device, draft, cur, def));
  assert.ok(isVisible(SPEECH.fields[0], {}, cur, def), 'fields without a condition always show');
  assert.ok(SECTIONS.includes(SPEECH) && SECTIONS.indexOf(SPEECH) < SECTIONS.findIndex((s) => s.id === 'model'));
});

test('an API key is nullable text and never echoed into a label', () => {
  const key = ALL_FIELDS.find((f) => f.path === 'tts.api_key');
  assert.equal(key.type, 'password');
  assert.equal(fromEditor({ ...key, type: 'text' }, '  '), null);
});

test('the Anki settings: tags are a comma list, the key is a secret, and nothing is listed twice', () => {
  const tags = ALL_FIELDS.find((f) => f.path === 'learning.anki.tags');
  assert.equal(toEditor(tags, ['maru', 'game']), 'maru, game');
  assert.deepEqual(fromEditor(tags, ' maru ,, game , '), ['maru', 'game']);
  assert.deepEqual(fromEditor(tags, ''), []);
  const key = ALL_FIELDS.find((f) => f.path === 'learning.anki.api_key');
  assert.equal(key.type, 'password');
  assert.equal(fromEditor(key, '  '), null, 'an empty key removes it');
  assert.ok(SECTIONS.includes(ANKI));
  const paths = ALL_FIELDS.map((f) => f.path).filter((p) => p.startsWith('learning.anki.'));
  assert.equal(new Set(paths).size, paths.length);
  assert.ok(paths.includes('learning.anki.reverse') && paths.includes('learning.anki.sync_after_add'));
});

test('the model name a server such as Ollama expects can be set from the settings page', () => {
  const field = ALL_FIELDS.find((f) => f.path === 'llm.model_name');
  assert.ok(field, 'the field exists');
  assert.equal(field.type, 'text');
  assert.ok(!appliesLive('llm.model_name'), 'it is read when Maru starts');
});
