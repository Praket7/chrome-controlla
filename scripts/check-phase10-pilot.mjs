import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifestPath = path.join(root, 'bench/tasks/phase10-pilot.json');
const baselinesPath = path.join(root, 'bench/baselines/versions.lock.json');

export function validatePilot(pilot, baselines) {
  const errors = [];
  const categories = ['generic', 'extraction', 'interference', 'multi_tab', 'design'];
  if (pilot?.schema_version !== 1) errors.push('schema_version must be 1');
  if (pilot?.status !== 'preregistered-no-results') errors.push('status must remain preregistered-no-results');
  if (!pilot?.preregistration?.primary_outcomes?.includes('verified_success')) errors.push('verified_success must be a primary outcome');
  if (pilot?.preregistration?.pilot_repetitions_per_template !== 3) errors.push('pilot must specify three repetitions per template');
  if (pilot?.preregistration?.order_randomization?.seed !== 20261006) errors.push('order randomization seed must be fixed');
  if (JSON.stringify(pilot?.preregistration?.categories) !== JSON.stringify(categories)) errors.push('categories must match the five preregistered workload categories');
  if (!Array.isArray(pilot?.tasks) || pilot.tasks.length !== 30) errors.push('pilot must contain exactly 30 templates');

  const ids = new Set();
  const counts = Object.fromEntries(categories.map((category) => [category, 0]));
  for (const task of pilot?.tasks ?? []) {
    if (ids.has(task.id)) errors.push(`duplicate task id: ${task.id}`);
    ids.add(task.id);
    if (!Object.hasOwn(counts, task.category)) errors.push(`unknown category for ${task.id}`);
    else counts[task.category]++;
    for (const field of ['prompt', 'initial_state', 'reset', 'success_predicate']) {
      if (!task[field] || (typeof task[field] === 'string' && !task[field].trim())) errors.push(`${task.id} requires ${field}`);
    }
    if (task.category === 'design' && task.review_sample_disclosure !== 'included in six design-review templates; report separately') {
      errors.push(`${task.id} must disclose design-review sampling`);
    }
  }
  for (const [category, count] of Object.entries(counts)) if (count !== 6) errors.push(`${category} must contain six templates`);

  if (pilot?.results !== null) errors.push('results must be null until runs are executed');
  if (!Array.isArray(baselines?.baselines) || baselines.baselines.length < 1) errors.push('candidate baseline lock is empty');
  if (pilot?.candidate_baselines !== '../baselines/versions.lock.json') errors.push('candidate baseline reference must use the checked-in version lock');
  return errors;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const pilot = JSON.parse(await readFile(manifestPath, 'utf8'));
    const baselines = JSON.parse(await readFile(baselinesPath, 'utf8'));
    const errors = validatePilot(pilot, baselines);
    if (errors.length) {
      console.error(errors.map((error) => `- ${error}`).join('\n'));
      process.exitCode = 1;
    } else {
      console.log(`Phase 10 pilot manifest valid: ${pilot.tasks.length} templates, 5 categories × 6; status=${pilot.status}.`);
    }
  } catch (error) {
    console.error(`Phase 10 pilot check failed: ${error.message}`);
    process.exitCode = 1;
  }
}
