import test from 'node:test';
import assert from 'node:assert/strict';
import { summarizeAgentUx } from './stage2.5-agent-ux-score.mjs';

test('direct Bills fast path passes without setup discovery or detours', () => {
  const result = summarizeAgentUx({
    task_id: 'espn-newest-bills-story',
    prompt_started_ms: 0,
    first_browser_command_ms: 900,
    verified_complete_ms: 2_900,
    measured_browser_work_ms: 1_450,
    expected_first_destination: 'bills-specific-page',
    first_destination: 'bills-specific-page',
    events: [
      { kind: 'browser_open', at_ms: 900 },
      { kind: 'snapshot', at_ms: 1_550, targeted: true, model_visible_observation: true },
      { kind: 'workflow', at_ms: 1_800, mutation: true },
      { kind: 'verify', at_ms: 2_700 },
    ],
  });
  assert.equal(result.pass, true);
  assert.equal(result.metrics.schema_discovery_calls, 0);
  assert.equal(result.metrics.repo_reads, 0);
  assert.equal(result.metrics.broad_snapshots, 0);
  assert.equal(result.metrics.navigation_detours, 0);
  assert.equal(result.metrics.orchestration_overhead_ms, 1_450);
});

test('the observed three-minute source-reading route fails every relevant orchestration gate', () => {
  const result = summarizeAgentUx({
    task_id: 'espn-newest-bills-story',
    prompt_started_ms: 0,
    first_browser_command_ms: 70_000,
    verified_complete_ms: 175_000,
    measured_browser_work_ms: 1_400,
    expected_first_destination: 'bills-specific-page',
    first_destination: 'espn-homepage',
    events: [
      { kind: 'mcp_discovery', at_ms: 2_000 },
      { kind: 'memory_read', at_ms: 4_000 },
      { kind: 'repo_read', at_ms: 7_000 },
      { kind: 'source_search', at_ms: 10_000 },
      { kind: 'documentation_read', at_ms: 13_000 },
      { kind: 'schema_lookup', at_ms: 20_000 },
      { kind: 'browser_describe', at_ms: 30_000 },
      { kind: 'browser_open', at_ms: 70_000 },
      { kind: 'navigation', at_ms: 71_000, detour: true },
      { kind: 'snapshot', at_ms: 72_000, targeted: false, model_visible_observation: true },
      { kind: 'snapshot', at_ms: 74_000, targeted: false, model_visible_observation: true, duplicate: true },
      { kind: 'navigation', at_ms: 90_000, detour: true },
      { kind: 'snapshot', at_ms: 91_000, targeted: true, model_visible_observation: true },
      { kind: 'act', at_ms: 150_000, mutation: true },
      { kind: 'verify', at_ms: 174_000 },
    ],
  });
  assert.equal(result.pass, false);
  const failed = new Set(result.failures.map(failure => failure.metric));
  for (const metric of [
    'prompt_to_first_browser_ms',
    'pre_browser_metadata_calls',
    'schema_discovery_calls',
    'repo_reads',
    'documentation_reads',
    'describe_calls',
    'broad_snapshots',
    'navigation_detours',
    'duplicate_observations',
    'model_visible_observations_before_first_mutation',
    'first_destination',
  ]) {
    assert.equal(failed.has(metric), true, `expected ${metric} to fail`);
  }
});

test('schema recovery after an explicit mismatch can be given a recovery-only budget', () => {
  const result = summarizeAgentUx({
    task_id: 'schema-recovery',
    prompt_started_ms: 0,
    first_browser_command_ms: 1_000,
    verified_complete_ms: 4_000,
    limits: {
      max_schema_discovery_calls: 1,
      max_pre_browser_metadata_calls: 0,
      max_repo_reads: 0,
      max_documentation_reads: 0,
      max_describe_calls: 0,
    },
    events: [
      { kind: 'browser_open', at_ms: 1_000 },
      { kind: 'schema_lookup', at_ms: 1_200 },
      { kind: 'browser_open', at_ms: 1_500 },
      { kind: 'verify', at_ms: 3_900 },
    ],
  });
  assert.equal(result.pass, true);
  assert.equal(result.metrics.schema_discovery_calls, 1);
});
