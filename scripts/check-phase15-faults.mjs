import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const faults = JSON.parse(await readFile(new URL('../bench/faults/phase15-fault-matrix.json', import.meta.url), 'utf8'));
assert.equal(faults.schema_version, 1);
assert.equal(faults.track, 'phase15-fault-injection');
const required = [
  'renderer_crash','delayed_cdp','dropped_extension','navigation_between_ops','process_restart',
  'human_edit','duplicate_node_replacement','network_loss','timeout_after_mutation_dispatch'
];
const ids = new Set(faults.faults.map(fault => fault.id));
for (const id of required) assert.ok(ids.has(id), `missing fault ${id}`);
for (const fault of faults.faults) {
  assert.equal(fault.automatic_retry, false, `${fault.id} must never request automatic retry`);
  assert.ok(fault.required_outcome);
  assert.ok(fault.safety);
}
assert.deepEqual(faults.skill_drift_rechecks_days, [1,7,30]);
for (const state of ['completed','failed','unknown']) assert.ok(faults.required_job_states.includes(state));
assert.ok(faults.invariants.some(value => value.includes('unknown delivery')));
console.log(`phase15 fault matrix valid: ${faults.faults.length} injected failures, automatic retry disabled`);
