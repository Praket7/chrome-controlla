import fs from 'node:fs';
import process from 'node:process';

const DEFAULT_LIMITS = Object.freeze({
  max_prompt_to_first_browser_ms: 5_000,
  max_pre_browser_metadata_calls: 0,
  max_schema_discovery_calls: 0,
  max_repo_reads: 0,
  max_documentation_reads: 0,
  max_describe_calls: 0,
  max_broad_snapshots: 0,
  max_navigation_detours: 0,
  max_duplicate_observations: 0,
  max_model_visible_observations_before_first_mutation: 1,
});

const BROWSER_KINDS = new Set([
  'browser_open',
  'browser_task',
  'snapshot',
  'workflow',
  'act',
  'extract',
  'verify',
]);
const METADATA_KINDS = new Set([
  'mcp_discovery',
  'tools_list',
  'schema_lookup',
  'browser_describe',
  'repo_read',
  'documentation_read',
  'memory_read',
  'source_search',
]);
const SCHEMA_KINDS = new Set(['mcp_discovery', 'tools_list', 'schema_lookup']);

function count(events, predicate) {
  return events.reduce((total, event) => total + Number(Boolean(predicate(event))), 0);
}

function firstTime(events, predicate) {
  return events.find(predicate)?.at_ms ?? null;
}

function nonNegativeDelta(end, start, label) {
  if (!Number.isFinite(end) || !Number.isFinite(start) || end < start) {
    throw new Error(`${label} timestamps are invalid`);
  }
  return end - start;
}

export function summarizeAgentUx(trace, limits = {}) {
  if (!trace || typeof trace !== 'object' || !Array.isArray(trace.events)) {
    throw new Error('trace.events must be an array');
  }
  const effective = { ...DEFAULT_LIMITS, ...limits, ...(trace.limits ?? {}) };
  const promptStarted = trace.prompt_started_ms ?? 0;
  const firstBrowser = trace.first_browser_command_ms
    ?? firstTime(trace.events, event => BROWSER_KINDS.has(event.kind));
  if (firstBrowser == null) throw new Error('trace has no browser command');
  const verifiedComplete = trace.verified_complete_ms;
  if (!Number.isFinite(verifiedComplete)) throw new Error('verified_complete_ms is required');

  const firstMutation = firstTime(trace.events, event => event.mutation === true);
  const preBrowser = trace.events.filter(event => Number.isFinite(event.at_ms) && event.at_ms < firstBrowser);
  const beforeMutation = firstMutation == null
    ? trace.events
    : trace.events.filter(event => Number.isFinite(event.at_ms) && event.at_ms < firstMutation);

  const metrics = {
    prompt_to_first_browser_ms: nonNegativeDelta(firstBrowser, promptStarted, 'prompt-to-first-browser'),
    total_prompt_to_verified_ms: nonNegativeDelta(verifiedComplete, promptStarted, 'prompt-to-verified'),
    pre_browser_metadata_calls: count(preBrowser, event => METADATA_KINDS.has(event.kind)),
    schema_discovery_calls: count(trace.events, event => SCHEMA_KINDS.has(event.kind)),
    repo_reads: count(trace.events, event => event.kind === 'repo_read' || event.kind === 'source_search'),
    documentation_reads: count(trace.events, event => event.kind === 'documentation_read' || event.kind === 'memory_read'),
    describe_calls: count(trace.events, event => event.kind === 'browser_describe'),
    broad_snapshots: count(trace.events, event => event.kind === 'snapshot' && event.targeted !== true),
    targeted_snapshots: count(trace.events, event => event.kind === 'snapshot' && event.targeted === true),
    navigation_detours: count(trace.events, event => event.kind === 'navigation' && event.detour === true),
    duplicate_observations: count(trace.events, event => event.duplicate === true),
    model_visible_observations_before_first_mutation: count(
      beforeMutation,
      event => event.model_visible_observation === true,
    ),
    browser_commands: count(trace.events, event => BROWSER_KINDS.has(event.kind)),
  };
  metrics.orchestration_overhead_ms = Math.max(
    0,
    metrics.total_prompt_to_verified_ms - Number(trace.measured_browser_work_ms ?? 0),
  );

  const checks = [
    ['prompt_to_first_browser_ms', 'max_prompt_to_first_browser_ms'],
    ['pre_browser_metadata_calls', 'max_pre_browser_metadata_calls'],
    ['schema_discovery_calls', 'max_schema_discovery_calls'],
    ['repo_reads', 'max_repo_reads'],
    ['documentation_reads', 'max_documentation_reads'],
    ['describe_calls', 'max_describe_calls'],
    ['broad_snapshots', 'max_broad_snapshots'],
    ['navigation_detours', 'max_navigation_detours'],
    ['duplicate_observations', 'max_duplicate_observations'],
    ['model_visible_observations_before_first_mutation', 'max_model_visible_observations_before_first_mutation'],
  ];
  const failures = checks
    .filter(([metric, limit]) => metrics[metric] > effective[limit])
    .map(([metric, limit]) => ({
      metric,
      observed: metrics[metric],
      limit: effective[limit],
    }));

  if (trace.expected_first_destination && trace.first_destination !== trace.expected_first_destination) {
    failures.push({
      metric: 'first_destination',
      observed: trace.first_destination ?? null,
      limit: trace.expected_first_destination,
    });
  }

  return {
    schema_version: 1,
    task_id: trace.task_id ?? null,
    pass: failures.length === 0,
    metrics,
    limits: effective,
    failures,
  };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const file = process.argv[2];
  if (!file) {
    process.stderr.write('usage: node stage2.5-agent-ux-score.mjs TRACE.json\n');
    process.exit(2);
  }
  const trace = JSON.parse(fs.readFileSync(file, 'utf8'));
  const result = summarizeAgentUx(trace);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  if (!result.pass) process.exitCode = 1;
}
