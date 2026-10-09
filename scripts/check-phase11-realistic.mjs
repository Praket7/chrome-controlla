import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const config = JSON.parse(await readFile(new URL('../bench/tasks/phase11-realistic.json', import.meta.url), 'utf8'));
assert.equal(config.schema_version, 1);
assert.equal(config.track, 'phase11-realistic');
assert.equal(config.frozen_phase10_unchanged, true, 'Phase 10 must remain frozen');
for (const mode of ['cold_unknown','warm_known','warm_skill']) assert.ok(config.modes.includes(mode));
for (const concurrency of [1,2,4,8]) assert.ok(config.concurrency.includes(concurrency));
const requiredClasses = [
  'forms','extraction','multitab','spa','visual_canvas','virtual_list','upload',
  'repeated_workflow','interference','stale_navigation','long_workflow','concurrent_tabs'
];
for (const taskClass of requiredClasses) assert.ok(config.task_classes.includes(taskClass), `missing task class ${taskClass}`);
assert.equal(new Set(config.tasks.map(task => task.id)).size, config.tasks.length, 'task IDs must be unique');
for (const task of config.tasks) {
  assert.ok(requiredClasses.includes(task.class));
  assert.equal(task.requires_independent_verification, true);
  assert.ok(task.fixture);
}
const adapterIds = new Set(config.adapters.map(adapter => adapter.id));
for (const id of ['controlla','playwright-mcp','chrome-devtools-mcp','agent-browser','stagehand','browser-use','computer-use']) {
  assert.ok(adapterIds.has(id), `missing adapter ${id}`);
}
assert.equal(config.adapters.find(adapter => adapter.id === 'computer-use').track, 'separate-vision');
for (const metric of [
  'success','independently_verified_success','severe_wrong_target_events','authority_violations',
  'silent_stale_mutations','wall_ms','model_round_trips','model_tokens','mcp_calls','browser_ops',
  'observation_bytes','screenshot_bytes','verification_ms','peak_rss_bytes','focus_disruptions','clipboard_disruptions'
]) assert.ok(config.required_metrics.includes(metric), `missing metric ${metric}`);
assert.equal(config.claim_gates.success_noninferiority_lower_bound, -0.02);
assert.equal(config.claim_gates.minimum_model_round_trip_reduction, 0.30);
assert.equal(config.claim_gates.minimum_median_wall_time_reduction, 0.25);
console.log(`phase11 realistic benchmark contract valid: ${config.tasks.length} tasks, ${config.adapters.length} adapters`);
