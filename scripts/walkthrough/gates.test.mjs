import { test } from 'node:test';
import assert from 'node:assert/strict';
import { requirePersistedKey, requireCleanRuntime } from './gates.mjs';

test('persisted key is mandatory, even when connected through an environment key', () => {
  for (const status of [null, {}, { connected: true, has_key: true, source: 'env: SELORA_API_KEY' },
    { has_key: false, source: 'keyring' }]) assert.throws(() => requirePersistedKey(status));
  assert.doesNotThrow(() => requirePersistedKey({ has_key: true, source: 'keyring' }));
});

test('exceptions, console errors, and browser log errors fail the walkthrough', () => {
  assert.doesNotThrow(() => requireCleanRuntime([], []));
  assert.throws(() => requireCleanRuntime([{ description: 'exception' }], []));
  assert.throws(() => requireCleanRuntime([], ['console.error: failure']));
  assert.throws(() => requireCleanRuntime([], ['log.error: failure']));
});
