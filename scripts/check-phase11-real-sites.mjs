import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { liveTaskUrl } from '../bench/runners/phase11-runner.mjs';

const matrix = JSON.parse(await readFile(new URL('../bench/tasks/phase11-real-sites.json', import.meta.url), 'utf8'));
const required = [
  'nested_dropdown','multi_select_checkbox','searchable_combobox','date_picker_range','time_picker',
  'slider','tree_select','rich_editor','file_upload','hover_popover','virtualized_infinite_list',
  'tabs_carousel_collapse','drag_drop','canvas_chart_reading','cross_origin_iframe_oopif','spa_rerender',
  'cross_site_multitab','background_nonactivation','user_interference',
];
assert.equal(matrix.track, 'phase11-live-sites');
assert.equal(matrix.qualification_status, 'unqualified');
assert.deepEqual([...matrix.task_classes].sort(), [...required].sort());
assert.equal(new Set(matrix.tasks.map(task => task.class)).size, required.length);
assert.equal(matrix.tasks.length, required.length);
for (const task of matrix.tasks) {
  assert.ok(task.id && task.action_points?.length && task.perception_points?.length && task.independent_verifier);
  assert.equal(task.requires_independent_verification, true);
  liveTaskUrl(task);
  if (task.site_urls) {
    assert.equal(task.site_urls.length, task.site_allowlists.length);
    for (const [index, raw] of task.site_urls.entries()) {
      const site = new URL(raw);
      assert.equal(site.protocol, 'https:');
      assert.deepEqual(task.site_allowlists[index], [site.hostname]);
    }
  }
  if (task.class === 'file_upload') assert.equal(task.site_mode, 'public_test_fixture_only');
  if (task.class === 'background_nonactivation' || task.class === 'user_interference') {
    assert.ok(task.independent_verifier.startsWith('browser_'));
  }
}
assert.equal(matrix.adapters.find(adapter => adapter.id === 'controlla-1')?.source_ref, '62e6407d0e75b83a1be8d69468476601a6e6da37');
assert.ok(matrix.claim_policy.fixture_or_synthetic_evidence_cannot_qualify_live_site);
console.log(`phase11 live-site matrix valid: ${matrix.tasks.length} action/perception classes; qualification remains unverified`);
