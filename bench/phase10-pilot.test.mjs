import test from 'node:test';
import assert from 'node:assert/strict';
import { validatePilot } from '../scripts/check-phase10-pilot.mjs';

test('pilot requires 30 unique templates with six in each preregistered category', () => {
  const pilot = {
    schema_version: 1,
    status: 'preregistered-no-results',
    candidate_baselines: '../baselines/versions.lock.json',
    results: null,
    preregistration: {
      primary_outcomes: ['verified_success'],
      categories: ['generic', 'extraction', 'interference', 'multi_tab', 'design'],
      pilot_repetitions_per_template: 3,
      order_randomization: { seed: 20261006 },
    },
    tasks: Array.from({ length: 30 }, (_, index) => ({
      id: `P${String(index + 1).padStart(2, '0')}`,
      category: ['generic', 'extraction', 'interference', 'multi_tab', 'design'][Math.floor(index / 6)],
      prompt: 'Do task',
      initial_state: { value: 'initial' },
      reset: 'Reset fixture to the declared initial_state',
      success_predicate: 'Read state',
      offline_fixture: { before_actions: [], actions: [], predicates: [{ id: 'state', op: 'equals', path: '/value', expected: 'initial' }] },
      ...(index >= 24 ? { review_sample_disclosure: 'included in six design-review templates; report separately' } : {}),
    })),
  };
  assert.deepEqual(validatePilot(pilot, { baselines: [{}] }), []);
  pilot.tasks[1].id = pilot.tasks[0].id;
  assert.ok(validatePilot(pilot, { baselines: [{}] }).some((error) => error.includes('duplicate task id')));
});

test('each pilot task has a runnable offline reset and readback fixture', async () => {
  const { readFile } = await import('node:fs/promises');
  const { runOfflineFixture } = await import('./analysis/offline-task-harness.mjs');
  const pilot = JSON.parse(await readFile(new URL('./tasks/phase10-pilot.json', import.meta.url), 'utf8'));
  for (const task of pilot.tasks) {
    const result = runOfflineFixture({
      schema_version: 1,
      kind: 'offline-fixture-only',
      id: task.id,
      initial_state: task.initial_state,
      reset: { strategy: 'clone-initial-state' },
      before_actions: task.offline_fixture?.before_actions ?? [],
      actions: task.offline_fixture?.actions ?? [],
      predicates: task.offline_fixture?.predicates ?? [],
    });
    assert.equal(result.all_predicates_passed, true, `${task.id}: ${JSON.stringify(result.predicate_results)}`);
    if (['P14', 'P15', 'P16', 'P17'].includes(task.id)) assert.deepEqual(result.action_outcomes, [{ applied: false }], `${task.id} must withhold stale action`);
  }
});
