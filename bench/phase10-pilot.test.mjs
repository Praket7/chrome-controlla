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
      ...(index >= 24 ? { review_sample_disclosure: 'included in six design-review templates; report separately' } : {}),
    })),
  };
  assert.deepEqual(validatePilot(pilot, { baselines: [{}] }), []);
  pilot.tasks[1].id = pilot.tasks[0].id;
  assert.ok(validatePilot(pilot, { baselines: [{}] }).some((error) => error.includes('duplicate task id')));
});
