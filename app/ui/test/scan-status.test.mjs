import { test } from 'node:test';
import assert from 'node:assert/strict';
import { labelStatus, finishLabelStatus, finishLabels } from '../scan-status-core.js';

const waiting = () => labelStatus({ label: '', source: 'pending' });

test('an unanswered label says it is waiting for a translation', () => {
  assert.deepEqual(waiting(), { kind: 'loading', text: '•••', hint: 'Waiting for translation…' });
});

test('a dictionary or remembered translation remains visible', () => {
  for (const source of ['dictionary', 'cache', 'glossary', 'model', 'pending']) {
    const status = labelStatus({ label: 'Camera', source });
    assert.equal(status.kind, 'ready');
    assert.equal(status.text, 'Camera');
    assert.equal(finishLabelStatus(status, 'Server failed'), status);
  }
});

test('a failed request replaces waiting dots with a warning and its reason', () => {
  const status = finishLabelStatus(waiting(), 'The request timed out');
  assert.equal(status.kind, 'failed');
  assert.equal(status.text, '!');
  assert.match(status.hint, /The request timed out/);
});

test('a completed request without a label shows a question mark', () => {
  const status = finishLabelStatus(waiting());
  assert.equal(status.kind, 'unavailable');
  assert.equal(status.text, '?');
  assert.match(status.hint, /No translation returned/);
});

test('a blank completed answer is unavailable rather than still loading', () => {
  assert.equal(labelStatus({ label: '  ', source: 'model' }).kind, 'unavailable');
});

test('a duplicate completion cannot erase an existing failure reason', () => {
  const failed = finishLabelStatus(waiting(), 'NAS is unavailable');
  assert.equal(finishLabelStatus(failed), failed);
});

test('completion leaves another join or correction under the same scan id waiting', () => {
  const items = new Map([[0, { status: waiting() }], [4, { status: waiting() }], [5, { status: waiting() }]]);
  const changes = finishLabels(items, { id: 1, items: [4], error: 'Timed out' });
  assert.deepEqual(changes.map(([id]) => id), [4]);
  assert.equal(changes[0][1].kind, 'failed');
  assert.equal(items.get(0).status.kind, 'loading');
  assert.equal(items.get(5).status.kind, 'loading');
});

test('late completion for a removed label and finished labels has no effect', () => {
  const items = new Map([[0, { status: labelStatus({ label: 'Camera', source: 'model' }) }]]);
  assert.deepEqual(finishLabels(items, { items: [0, 9], error: 'Timed out' }), []);
});
