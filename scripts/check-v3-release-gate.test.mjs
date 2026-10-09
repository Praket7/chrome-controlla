import test from 'node:test';
import assert from 'node:assert/strict';
import { evaluateV3Gate } from './check-v3-release-gate.mjs';

const complete = {
  browser_mode_parity: true,
  fast_keys_fixed_delay_ms: 0,
  block_typing_protocol_calls_per_1000_chars: 1,
  interference_wrong_target_mutations: 0,
  multi_client_wrong_target_mutations: 0,
  unknown_mutation_auto_retries: 0,
  default_tool_count: 6,
  default_schema_bytes: 40_000,
  client_contracts_qualified: true,
  headless_recovery_bounded: true,
  page_tools_fail_closed: true,
  severe_safety_failures: 0,
  real_competitor_evidence_complete: true,
  generated_rows_used_for_best_claim: false,
};

test('complete evidence passes', () => {
  assert.deepEqual(evaluateV3Gate(complete), { pass: true, failures: [] });
});

test('missing real competitor evidence blocks best claim', () => {
  const result = evaluateV3Gate({ ...complete, real_competitor_evidence_complete: false });
  assert.equal(result.pass, false);
  assert.ok(result.failures.includes('real controlled competitor evidence incomplete'));
});

test('fixed fast-key delay blocks release', () => {
  const result = evaluateV3Gate({ ...complete, fast_keys_fixed_delay_ms: 60 });
  assert.equal(result.pass, false);
  assert.ok(result.failures.some((failure) => failure.includes('fixed per-character delay')));
});

test('any wrong-target mutation blocks release', () => {
  const result = evaluateV3Gate({ ...complete, interference_wrong_target_mutations: 1 });
  assert.equal(result.pass, false);
});
