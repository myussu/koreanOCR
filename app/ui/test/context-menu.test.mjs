import { test } from 'node:test';
import assert from 'node:assert/strict';
import { menuPlacement } from '../context-menu-core.js';

test('a right-click menu stays inside the window at each edge', () => {
  const size = { w: 210, h: 112 }, bounds = { w: 800, h: 600 };
  assert.deepEqual(menuPlacement({ x: 300, y: 200 }, size, bounds), { x: 300, y: 200 });
  assert.deepEqual(menuPlacement({ x: 799, y: 599 }, size, bounds), { x: 586, y: 484 });
  assert.deepEqual(menuPlacement({ x: -2, y: -2 }, size, bounds), { x: 4, y: 4 });
});
