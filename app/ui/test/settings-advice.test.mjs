import test from 'node:test';
import assert from 'node:assert/strict';
import { family, recommendFor, adviceTask, measuredFor } from '../settings-advice-core.js';

const servers = [
  { name: 'NAS', endpoint: 'http://192.168.1.20:11434', model_name: 'gemma4-12b', provider: 'openai' },
  { name: 'Claude', endpoint: 'https://api.anthropic.com', model_name: 'claude-sonnet-5-5', provider: 'openai' },
  { name: 'Azure', endpoint: 'https://contoso.openai.azure.com', model_name: 'gpt-4.1-mini', provider: 'azure' },
  { name: 'Gemini', endpoint: 'https://generativelanguage.googleapis.com/v1beta/openai', model_name: 'gemini-flash', provider: 'gemini' },
];
const values = (over = {}) => ({ llm: { hf_repo: 'unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL', servers, ...over } });

test('models are told apart by family, whatever the server calls them', () => {
  assert.deepEqual(['gemma4-12b-3060ti', 'gemma-4-12B-it-qat-GGUF', 'claude-sonnet-5-5', 'claude-haiku-5-5', 'gpt-4.1-mini', 'gemini-3.8-flash', 'qwen3'].map(family), ['gemma', 'gemma', 'sonnet', 'haiku', 'gpt-mini', 'gemini', 'other']);
});

test('live subtitles go to the NAS when there is one: as fast as this PC, and the game keeps the graphics card', () => {
  const r = recommendFor('live', values());
  assert.equal(r.value, 'NAS');
  assert.match(r.why, /0\.4 s a line in run 1/);
  assert.equal(recommendFor('live', values({ servers: servers.filter((s) => s.name !== 'NAS') })).value, 'local');
  assert.equal(recommendFor('live', { llm: { servers: servers.filter((s) => s.name === 'Gemini') } }), null, 'a free Gemini key is never recommended');
});

test('captures stay on this PC, and go to the NAS while live subtitles run so models are not swapped mid-game', () => {
  assert.equal(recommendFor('capture', values()).value, 'local');
  assert.equal(recommendFor('while_live', values()).value, 'NAS');
  assert.match(recommendFor('while_live', values()).why, /never swaps models/);
});

test('card meanings follow the blind review: the free Gemma, saying Sonnet scored higher without a proven difference', () => {
  const r = recommendFor('card_meanings', values());
  assert.equal(r.value, 'local');
  assert.match(r.why, /83 % right in your blind review.*88 %/);
  assert.equal(recommendFor('card_meanings', { llm: { servers: [servers[1]] } }).value, 'Claude');
  assert.deepEqual(recommendFor('card_sentence', values()).value, '', 'keeping the translation the line has: nothing was scored');
});

test('each route field knows its task, and a connection card can say what it was measured at', () => {
  assert.equal(adviceTask('tasks.capture.while_live'), 'while_live');
  assert.equal(adviceTask('learning.anki.deck'), null);
  assert.equal(measuredFor('live', 'claude-haiku-5-5'), 'about 0.7 s a line in run 1');
  assert.equal(measuredFor('while_live', 'gpt-4.1-mini'), 'about 1 s a sentence in run 2');
  assert.equal(measuredFor('live', 'qwen3'), null);
});
