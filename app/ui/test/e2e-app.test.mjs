import test from 'node:test';
import assert from 'node:assert/strict';
import { CHANGES, SECTIONS, CONFIG } from '../../../tools/e2e_app.mjs';
import { CATEGORIES, ALL_FIELDS } from '../settings-core.js';

test('the end-to-end walk visits every Settings section and changes one setting in each one that has settings', () => {
  assert.deepEqual(SECTIONS, CATEGORIES.map((c) => c.id), 'the walk follows the page, section by section');
  const changed = CHANGES.map((c) => c.section);
  for (const id of SECTIONS.filter((s) => s !== 'home')) assert.ok(changed.includes(id), `a change in ${id}`);
  for (const c of CHANGES) {
    const field = ALL_FIELDS.find((f) => f.path === c.path);
    assert.ok(field, `${c.path} is a setting on the page`);
    assert.equal(c.section === 'advanced', !!field.advanced, `${c.path} is where the walk looks for it`);
  }
});

test('the temporary home never touches Anki or loads a model', () => {
  assert.match(CONFIG, /\[learning\.anki\]\s*\nenabled = false/);
  assert.match(CONFIG, /endpoint = "http:\/\/127\.0\.0\.1:9"/);
  assert.doesNotMatch(CONFIG, /api_key/);
});
