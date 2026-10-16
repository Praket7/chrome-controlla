import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { REQUIRED_METRIC_EVIDENCE_FIELDS } from '../bench/analysis/phase11-analysis.mjs';

const config = JSON.parse(await readFile(new URL('../bench/tasks/phase11-realistic.json', import.meta.url), 'utf8'));
assert.equal(config.schema_version, 1);
assert.equal(config.track, 'phase11-realistic');
assert.equal(config.frozen_phase10_unchanged, true, 'Phase 10 must remain frozen');
assert.deepEqual(config.modes, ['cold_browser','warm_browser_cold_site','warm_qualified_route','reconnect'],
  'qualification must keep cold browser, warm browser/cold site, qualified route, and reconnect cohorts separate');
assert.deepEqual(config.execution_modes, ['foreground','background','headless'], 'qualification must cross all supported execution modes');
for (const concurrency of [1,2,4,8]) assert.ok(config.concurrency.includes(concurrency));
assert.ok(Number.isSafeInteger(config.random_seed) && config.random_seed >= 0, 'paired trial random seed must be pinned');
assert.ok(config.repetitions >= 30, 'release evidence needs at least 30 measured repetitions');
assert.ok(config.warmup_repetitions >= 1, 'warmup runs must be declared separately');
assert.equal(config.claim_gates.maximum_false_success_regression, 0,
  'a false-success regression must block release qualification');
const requiredClasses = [
  'forms','extraction','multitab','spa','visual_canvas','virtual_list','upload',
  'repeated_workflow','interference','stale_navigation','long_workflow','concurrent_tabs',
  'cross_origin_oopif','download_upload'
];
for (const taskClass of requiredClasses) assert.ok(config.task_classes.includes(taskClass), `missing task class ${taskClass}`);
assert.equal(new Set(config.tasks.map(task => task.id)).size, config.tasks.length, 'task IDs must be unique');
for (const task of config.tasks) {
  assert.ok(requiredClasses.includes(task.class));
  assert.equal(task.requires_independent_verification, true);
  assert.ok(task.fixture);
}
const adapterIds = new Set(config.adapters.map(adapter => adapter.id));
for (const id of ['controlla','controlla-1','controlla-2-pre-stage2','playwright-mcp','chrome-devtools-mcp','agent-browser','stagehand','browser-use','computer-use']) {
  assert.ok(adapterIds.has(id), `missing adapter ${id}`);
}
const requiredRegressionTasks = [
  'slow-media-early-form','50k-node-targeted-control','rich-editor-1000-char-fastkeys',
  'five-field-single-workflow','virtual-list-1000-rows','newest-ranking-50-candidates',
  'artifact-register-4mb','artifact-register-10mb','react-rerender-target','on-demand-frame-escalation',
  'cross-site-oopif-target','download-register-upload','seven-slide-capture-manifest'
];
for (const id of requiredRegressionTasks) {
  const task = config.tasks.find(candidate => candidate.id === id);
  assert.ok(task, `missing Stage 2 regression ${id}`);
  assert.ok(task.scenario, `Stage 2 regression ${id} needs an explicit scenario`);
}
assert.equal(config.adapters.find(adapter => adapter.id === 'controlla-1').source_ref, '62e6407d0e75b83a1be8d69468476601a6e6da37');
assert.equal(config.adapters.find(adapter => adapter.id === 'controlla-2-pre-stage2').source_ref, '406b9d9794d6cb59dc75d22c7e6fe7a8554b1e67');
assert.equal(config.adapters.find(adapter => adapter.id === 'computer-use').track, 'separate-vision');
const minimumRequiredMetrics = [
  'success','independently_verified_success','severe_wrong_target_events','authority_violations',
  'silent_stale_mutations','wall_ms','model_round_trips','model_tokens','mcp_calls','browser_ops',
  'prompt_to_verified_ms','prompt_to_final_ms','external_controlla_calls','model_turns',
  'model_visible_observations','internal_model_calls','browser_cdp_commands','host_round_trips',
  'native_host_messages_sent','native_host_messages_received','schema_bytes','prompt_input_bytes',
  'tool_input_bytes','output_bytes','observation_bytes','screenshot_bytes','unknown_delivery_count',
  'fallback_count','retries','timeout_count','cleanup_completed','cleanup_outcome','verification_ms',
  'peak_rss_bytes','focus_disruptions','clipboard_disruptions'
];
for (const metric of minimumRequiredMetrics) {
  assert.ok(config.required_metrics.includes(metric), `missing metric ${metric}`);
}

const metricContract = config.metric_measurement_contract;
assert.equal(metricContract?.missing_value, null, 'unavailable measurements must remain explicit nulls');
assert.ok(Array.isArray(metricContract?.missingness_fields)
  && metricContract.missingness_fields.includes('missing_reason')
  && metricContract.missingness_fields.includes('provenance'),
'missing measurements need both a reason and provenance');
assert.ok(metricContract.rules.some(rule => /zero is an observed measurement only/i.test(rule)),
  'zero must not stand in for unavailable or incomplete evidence');
for (const metric of config.required_metrics) {
  assert.equal(typeof metricContract.provenance_by_metric?.[metric], 'string',
    `missing provenance definition for ${metric}`);
  assert.ok(metricContract.provenance_by_metric[metric].trim(),
    `empty provenance definition for ${metric}`);
}
const adapterContract = JSON.parse(await readFile(new URL('../bench/runners/adapter-contract.json', import.meta.url), 'utf8'));
assert.deepEqual([...adapterContract.measurement_metric_fields].sort(), [...config.required_metrics].sort(),
  'adapter measurement-evidence contract must cover every and only required Phase 11 metric');
assert.deepEqual([...REQUIRED_METRIC_EVIDENCE_FIELDS].sort(), [...config.required_metrics].sort(),
  'public claim gate must evaluate every and only required Phase 11 metric');
const adapterMetricOutputs = new Set([
  ...(adapterContract.output_required ?? []), ...(adapterContract.runner_enriched_output ?? [])
]);
for (const metric of adapterContract.measurement_metric_fields ?? []) {
  assert.ok(config.required_metrics.includes(metric), `adapter contract metric ${metric} is not required by the task set`);
  assert.ok(adapterMetricOutputs.has(metric), `required metric ${metric} is absent from adapter and runner output contracts`);
  assert.ok(Object.hasOwn(adapterContract.measurement_metric_types ?? {}, metric),
    `adapter contract metric ${metric} has no type`);
  assert.equal(typeof adapterContract.measurement_metric_provenance?.[metric], 'string',
    `adapter contract metric ${metric} has no provenance`);
  assert.ok(adapterContract.measurement_metric_provenance[metric].trim(),
    `adapter contract metric ${metric} has empty provenance`);
}
for (const metric of minimumRequiredMetrics.filter(name => name.startsWith('prompt_to_')
  || ['external_controlla_calls','model_turns','model_visible_observations','internal_model_calls',
    'browser_cdp_commands','host_round_trips','native_host_messages_sent','native_host_messages_received',
    'schema_bytes','prompt_input_bytes','tool_input_bytes','output_bytes','unknown_delivery_count',
    'fallback_count','timeout_count','cleanup_completed','cleanup_outcome'].includes(name))) {
  assert.ok(adapterContract.measurement_metric_fields.includes(metric),
    `adapter contract does not validate metric ${metric}`);
}
assert.equal(config.claim_gates.success_noninferiority_lower_bound, -0.01);
assert.equal(config.claim_gates.minimum_model_round_trip_reduction, 0.30);
assert.equal(config.claim_gates.minimum_median_wall_time_reduction, 0.25);
console.log(`phase11 realistic benchmark contract valid: ${config.tasks.length} tasks, ${config.adapters.length} adapters`);
