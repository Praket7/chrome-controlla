import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runOfflineFixture, resetFixture, validateFixtureContract } from './offline-task-harness.mjs';

const fixture = JSON.parse(await readFile(new URL('../fixtures/offline-reset-predicate.json', import.meta.url), 'utf8'));

test('offline fixture resets from the original state and verifies independent readback predicates', () => {
  const first = runOfflineFixture(fixture);
  first.final_state.records[0].status = 'tampered-after-run';
  const second = runOfflineFixture(fixture);
  assert.equal(second.kind, 'offline-fixture-validation-only');
  assert.equal(second.all_predicates_passed, true);
  assert.deepEqual(second.final_state.records.map((row) => row.status), ['ready', 'queued']);
  assert.deepEqual(second.predicate_results.map((row) => row.passed), [true, true, true, true]);
  assert.deepEqual(resetFixture(fixture), fixture.initial_state);
});

test('offline fixture rejects unsafe paths and unsupported operations', () => {
  const invalid = structuredClone(fixture);
  invalid.actions[0].path = '/records/0/__proto__/polluted';
  invalid.actions.push({ op: 'delete', path: '/records/0/status' });
  assert.ok(validateFixtureContract(invalid).some((error) => error.includes('unsafe JSON pointer')));
  assert.ok(validateFixtureContract(invalid).some((error) => error.includes('unsupported operation')));
  assert.throws(() => runOfflineFixture(invalid), /invalid offline fixture/);
});

test('malformed set-if conditions return validation errors instead of throwing', () => {
  for (const when of [null, { path: '/records/0/status', equals: 'ready' }]) {
    const invalid = structuredClone(fixture);
    invalid.actions[0] = { op: 'set-if', path: '/records/0/status', value: 'done', when };
    assert.ok(validateFixtureContract(invalid).some((error) => error.includes('set-if requires conditions')));
    assert.throws(() => runOfflineFixture(invalid), /invalid offline fixture/);
  }
});

test('predicate failure stays visible and never becomes a pass from the action description', () => {
  const invalid = structuredClone(fixture);
  invalid.predicates[0].expected = 'done';
  const output = runOfflineFixture(invalid);
  assert.equal(output.all_predicates_passed, false);
  assert.equal(output.predicate_results[0].observed, 'ready');
  assert.equal(output.predicate_results[0].passed, false);
});
