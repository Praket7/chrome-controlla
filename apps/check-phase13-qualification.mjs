import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const matrix = JSON.parse(await readFile(new URL('./qualification-matrix.json', import.meta.url), 'utf8'));
const required = [
  'traditional_forms','react','vue','angular','contenteditable','shadow_dom','iframes',
  'virtual_lists','infinite_scroll','dialogs_overlays','drag_drop','file_download','canvas',
  'spa_navigation','human_interference','renderer_replacement'
];
assert.equal(matrix.schema_version, 1);
assert.equal(matrix.generated_for, 'best-chrome-use-v2');
const byId = new Map(matrix.classes.map(entry => [entry.id, entry]));
for (const id of required) {
  assert.ok(byId.has(id), `missing difficult browser class ${id}`);
  const entry = byId.get(id);
  assert.ok(['covered','contract_only'].includes(entry.fixture_status), `${id} has invalid fixture status`);
  assert.ok(entry.evidence && entry.evidence.length > 8, `${id} must name evidence`);
}
assert.equal(byId.size, required.length, 'qualification matrix contains duplicate or unexpected classes');
for (const app of ['google_slides','canva','capcut_web']) {
  const entry = matrix.named_apps[app];
  assert.ok(entry, `missing named app ${app}`);
  assert.notEqual(entry.status, 'qualified', `${app} cannot be marked qualified without remote persistence evidence`);
  assert.ok(entry.required_evidence.includes('independent verification'));
}
assert.ok(matrix.narrow_local_evidence.excluded_claims.includes('remote application mutation'));
console.log(`phase13 qualification matrix valid: ${required.length} browser classes, live app claims remain fail-closed`);
