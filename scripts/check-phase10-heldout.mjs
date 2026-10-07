import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifestPath = path.join(root, 'bench/tasks/phase10-heldout.json');
const lockPath = path.join(root, 'bench/baselines/versions.lock.json');

export function digestManifest(manifest) {
  const frozen = structuredClone(manifest);
  delete frozen.freeze.split_digest;
  return createHash('sha256').update(JSON.stringify(frozen)).digest('hex');
}

export function validateHeldout(manifest, lock) {
  const errors = [];
  const cats = ['generic', 'extraction', 'interference', 'multi_tab', 'design'];
  if (manifest?.schema_version !== 1 || manifest?.status !== 'frozen-preregistered-no-results') errors.push('manifest must be schema v1 and frozen-preregistered-no-results');
  if (manifest?.preregistration?.results !== null) errors.push('results must remain null');
  if (manifest?.preregistration?.templates_per_category !== 20 || manifest?.tasks?.length !== 100) errors.push('held-out set must contain 100 templates, 20 per category');
  if (manifest?.preregistration?.held_out_repetitions_per_template !== 5 || manifest?.preregistration?.critical_repetitions_per_template !== 10) errors.push('run counts must be 5 and 10 for critical workflows');
  if (manifest?.preregistration?.noninferiority_margin_percentage_points !== 2) errors.push('success noninferiority margin must remain 2 percentage points');
  if (manifest?.preregistration?.bootstrap_resamples !== 10000 || manifest?.preregistration?.confidence_level !== 0.95) errors.push('bootstrap plan must remain frozen at 10,000 draws and 95%');
  if (manifest?.preregistration?.candidate_order_randomization?.seed !== 20261008 || manifest?.preregistration?.controlled_config?.prompt_budget_tokens !== 20000 || manifest?.preregistration?.controlled_config?.per_run_time_budget_seconds !== 120) errors.push('candidate randomization and run budgets must be frozen');
  if (JSON.stringify(manifest?.preregistration?.categories) !== JSON.stringify(cats)) errors.push('category order must match preregistration');
  if (JSON.stringify(manifest?.preregistration?.excluded_ids) !== JSON.stringify(Array.from({ length: 30 }, (_, i) => `P${String(i + 1).padStart(2, '0')}`))) errors.push('all P01-P30 pilot IDs must be excluded');
  const counts = Object.fromEntries(cats.map((c) => [c, 0]));
  const ids = new Set(); let critical = 0;
  for (const task of manifest?.tasks ?? []) {
    if (!/^H\d{3}$/.test(task.id) || ids.has(task.id)) errors.push(`invalid or duplicate held-out ID: ${task.id}`);
    ids.add(task.id);
    if (!Object.hasOwn(counts, task.category)) errors.push(`unknown category: ${task.id}`); else counts[task.category]++;
    if (['prompt', 'reset', 'success_predicate'].some((k) => typeof task[k] !== 'string' || !task[k].trim()) || !task.initial_state) errors.push(`missing task definition field: ${task.id}`);
    if (typeof task.critical !== 'boolean') errors.push(`critical flag missing: ${task.id}`);
    if (task.critical) critical++;
  }
  for (const [category, count] of Object.entries(counts)) if (count !== 20) errors.push(`${category} has ${count} templates, expected 20`);
  if (critical !== 20) errors.push(`critical workflows=${critical}, expected 20`);
  for (const category of cats) { const n = (manifest?.tasks ?? []).filter((t) => t.category === category && t.critical).length; if (n !== 4) errors.push(`${category} must contain four critical workflows`); }
  if (manifest?.candidate_baselines !== '../baselines/versions.lock.json') errors.push('baseline lock reference changed');
  const lockHash = createHash('sha256').update(JSON.stringify(lock, null, 2) + '\n').digest('hex');
  if (manifest?.freeze?.baseline_lock_sha256 !== lockHash) errors.push('baseline lock digest mismatch');
  if (manifest?.freeze?.analysis_revision !== 'phase10-analysis-v1' || !/^[0-9a-f]{40}$/.test(manifest?.freeze?.controlla_revision ?? '')) errors.push('analysis or Controlla revision is not frozen');
  if (manifest?.freeze?.split_digest !== digestManifest(manifest)) errors.push('split/config digest mismatch');
  return errors;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    const lock = JSON.parse(await readFile(lockPath, 'utf8'));
    const errors = validateHeldout(manifest, lock);
    if (errors.length) { console.error(errors.map((e) => `- ${e}`).join('\n')); process.exitCode = 1; }
    else console.log(`Held-out manifest valid: ${manifest.tasks.length} templates, 20 critical; digest=${manifest.freeze.split_digest}`);
  } catch (error) { console.error(`Held-out manifest check failed: ${error.message}`); process.exitCode = 1; }
}
