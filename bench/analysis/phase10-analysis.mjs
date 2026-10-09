import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const fields = ['wall_time_ms', 'input_tokens', 'output_tokens', 'model_calls', 'mcp_calls', 'browser_operations', 'retries', 'human_interventions', 'focus_disruptions', 'clipboard_disruptions', 'cleanup_failures', 'cache_preparation_ms', 'cache_maintenance_ms'];
const integerFields = fields;

export function validateRows(rows, manifest, candidates) {
  const errors = [];
  const tasks = new Map(manifest.tasks.map((t) => [t.id, t]));
  const expected = new Set();
  for (const task of manifest.tasks) for (let rep = 1; rep <= (task.critical ? 10 : 5); rep++) for (const candidate of candidates) expected.add(`${task.id}|${rep}|${candidate}`);
  const seen = new Set();
  for (const [index, row] of (rows ?? []).entries()) {
    const key = `${row.task_id}|${row.repetition}|${row.candidate}`;
    if (!tasks.has(row.task_id) || !candidates.includes(row.candidate) || !expected.has(key)) errors.push(`row ${index + 1}: unexpected task/repetition/candidate`);
    if (seen.has(key)) errors.push(`row ${index + 1}: duplicate run ${key}`);
    seen.add(key);
    if (row.run_status !== 'complete' || row.verification_status !== 'independently_verified') errors.push(`row ${index + 1}: incomplete or unverified result`);
    if (typeof row.verified_success !== 'boolean' || typeof row.severe_wrong_target_or_authority_violation !== 'boolean') errors.push(`row ${index + 1}: missing primary outcome`);
    if (typeof row.verification_evidence !== 'string' || !row.verification_evidence.trim()) errors.push(`row ${index + 1}: missing independent evidence`);
    if (typeof row.cleanup_complete !== 'boolean') errors.push(`row ${index + 1}: missing cleanup outcome`);
    if (!['cold', 'warm'].includes(row.cache_state)) errors.push(`row ${index + 1}: cache_state must be cold or warm`);
    for (const field of integerFields) if (!Number.isSafeInteger(row[field]) || row[field] < 0 || (field === 'wall_time_ms' && row[field] === 0)) errors.push(`row ${index + 1}: invalid ${field}`);
    if (row.task_id && Number.isInteger(row.repetition) && (row.repetition % 2 === 1) !== (row.cache_state === 'cold')) errors.push(`row ${index + 1}: cache_state must alternate cold on odd, warm on even repetitions`);
    if (row.severe_wrong_target_or_authority_violation && row.verified_success) errors.push(`row ${index + 1}: severe violation cannot be a success`);
  }
  for (const key of expected) if (!seen.has(key)) errors.push(`missing required run ${key}`);
  return errors;
}

export function validateRunManifest(input, manifest) {
  const errors = [];
  if (input?.manifest_digest !== manifest.freeze.split_digest) errors.push('result file manifest digest does not match frozen split');
  if (input?.controlla_revision !== manifest.freeze.controlla_revision) errors.push('result file Controlla revision does not match frozen candidate revision');
  return errors;
}

function quantile(values, p) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const x = (sorted.length - 1) * p, lo = Math.floor(x), hi = Math.ceil(x);
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (x - lo);
}
function mean(xs) { return xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null; }
function rng(seed) {
  let x = seed >>> 0;
  return () => { x = (x + 0x6D2B79F5) >>> 0; let t = x; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}
function taskMeans(rows, candidate, metric, transform = (x) => x) {
  const groups = new Map();
  for (const r of rows.filter((x) => x.candidate === candidate)) {
    const key = r.task_id;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(transform(metric(r)));
  }
  return groups;
}
function pairedClusterInterval(rows, treatment, control, metric, seed, draws) {
  const a = taskMeans(rows, treatment, metric), b = taskMeans(rows, control, metric);
  const ids = [...a.keys()].filter((id) => b.has(id));
  const diffs = ids.map((id) => mean(a.get(id)) - mean(b.get(id)));
  const observed = mean(diffs), random = rng(seed), boot = [];
  for (let n = 0; n < draws; n++) {
    const sample = [];
    for (let j = 0; j < ids.length; j++) sample.push(diffs[Math.floor(random() * diffs.length)]);
    boot.push(mean(sample));
  }
  return { estimate: observed, lower: quantile(boot, 0.025), upper: quantile(boot, 0.975), task_clusters: ids.length };
}
function metricSummary(rows, candidate) {
  const own = rows.filter((r) => r.candidate === candidate);
  const summary = { runs: own.length, success_rate: mean(own.map((r) => Number(r.verified_success))), severe_events: own.filter((r) => r.severe_wrong_target_or_authority_violation).length, cleanup_failures: own.reduce((s, r) => s + r.cleanup_failures, 0), cleanup_incomplete_runs: own.filter((r) => !r.cleanup_complete).length, p50_wall_time_ms: quantile(own.map((r) => r.wall_time_ms), .5), p95_wall_time_ms: quantile(own.map((r) => r.wall_time_ms), .95) };
  for (const field of fields.filter((f) => f !== 'wall_time_ms')) summary[field] = { total: own.reduce((s, r) => s + r[field], 0), mean_per_run: mean(own.map((r) => r[field])) };
  const criticalTasks = new Set(own.filter((r) => r.repetition === 10).map((r) => r.task_id));
  const r10 = [...criticalTasks].map((task_id) => { const outcomes = own.filter((r) => r.task_id === task_id && r.repetition <= 10); return { task_id, all_ten_correct: outcomes.length === 10 && outcomes.every((r) => r.verified_success && !r.severe_wrong_target_or_authority_violation) }; });
  summary.critical_r10 = { workflows: r10.length, passed: r10.filter((x) => x.all_ten_correct).length, outcomes: r10 };
  summary.by_cache_state = {};
  for (const state of ['cold', 'warm']) {
    const subset = own.filter((r) => r.cache_state === state);
    summary.by_cache_state[state] = { runs: subset.length, p50_wall_time_ms: quantile(subset.map((r) => r.wall_time_ms), .5), p95_wall_time_ms: quantile(subset.map((r) => r.wall_time_ms), .95), cache_preparation_ms: subset.reduce((s, r) => s + r.cache_preparation_ms, 0), cache_maintenance_ms: subset.reduce((s, r) => s + r.cache_maintenance_ms, 0), model_calls: subset.reduce((s, r) => s + r.model_calls, 0), mcp_calls: subset.reduce((s, r) => s + r.mcp_calls, 0), browser_operations: subset.reduce((s, r) => s + r.browser_operations, 0), human_interventions: subset.reduce((s, r) => s + r.human_interventions, 0), cleanup_failures: subset.reduce((s, r) => s + r.cleanup_failures, 0) };
  }
  return summary;
}

export function analyze(rows, manifest, candidates) {
  const errors = validateRows(rows, manifest, candidates);
  if (errors.length) throw new Error(`Rejected ${errors.length} invalid result row/set: ${errors.slice(0, 12).join('; ')}`);
  const draws = manifest.preregistration.bootstrap_resamples, seed = manifest.preregistration.bootstrap_seed;
  const controls = candidates.filter((c) => c !== 'Controlla');
  const comparisons = Object.fromEntries(controls.map((control, i) => [control, {
    success_difference: pairedClusterInterval(rows, 'Controlla', control, (r) => Number(r.verified_success), seed + i, draws),
    success_noninferiority_margin: -manifest.preregistration.noninferiority_margin_percentage_points / 100,
    success_noninferiority_pass: undefined,
    paired_log_wall_time_ratio: pairedClusterInterval(rows, 'Controlla', control, (r) => Math.log(r.wall_time_ms), seed + 100 + i, draws),
    paired_log_model_call_ratio: pairedClusterInterval(rows, 'Controlla', control, (r) => Math.log(Math.max(1, r.model_calls)), seed + 200 + i, draws),
  }]));
  const byCandidate = Object.fromEntries(candidates.map((c) => [c, metricSummary(rows, c)]));
  for (const [control, comparison] of Object.entries(comparisons)) {
    comparison.success_noninferiority_pass = comparison.success_difference.lower > comparison.success_noninferiority_margin;
    const controlModelCalls = byCandidate[control].model_calls.total;
    const modelCallReduction = controlModelCalls === 0 ? null : 1 - byCandidate.Controlla.model_calls.total / controlModelCalls;
    comparison.efficiency_targets = {
      model_call_reduction: modelCallReduction,
      model_call_reduction_target: manifest.preregistration.target_model_round_trip_reduction,
      model_call_reduction_pass: modelCallReduction === null ? null : modelCallReduction >= manifest.preregistration.target_model_round_trip_reduction,
      paired_geometric_model_call_ratio_ci: { lower: Math.exp(comparison.paired_log_model_call_ratio.lower), upper: Math.exp(comparison.paired_log_model_call_ratio.upper) },
      median_wall_time_reduction: 1 - byCandidate.Controlla.p50_wall_time_ms / byCandidate[control].p50_wall_time_ms,
      median_wall_time_reduction_target: manifest.preregistration.target_median_wall_time_reduction,
      median_wall_time_reduction_pass: 1 - byCandidate.Controlla.p50_wall_time_ms / byCandidate[control].p50_wall_time_ms >= manifest.preregistration.target_median_wall_time_reduction,
      paired_geometric_wall_time_ratio_ci: { lower: Math.exp(comparison.paired_log_wall_time_ratio.lower), upper: Math.exp(comparison.paired_log_wall_time_ratio.upper) },
      severe_event_guard_pass: byCandidate.Controlla.severe_events === 0,
    };
  }
  return { manifest_digest: manifest.freeze.split_digest, rows: rows.length, bootstrap_resamples: draws, cluster_unit: 'task template', comparisons, candidates: byCandidate, interpretation: 'Intervals are task-clustered paired bootstrap intervals. Positive success difference favors Controlla; negative log ratios favor Controlla. Proposed targets remain targets, not conclusions.' };
}

async function main() {
  const inputPath = process.argv[2];
  const outputPath = process.argv[3];
  if (!inputPath || !outputPath) throw new Error('usage: node bench/analysis/phase10-analysis.mjs RESULTS.json OUTPUT.json');
  const manifest = JSON.parse(await readFile(path.join(root, 'bench/tasks/phase10-heldout.json'), 'utf8'));
  const lock = JSON.parse(await readFile(path.join(root, 'bench/baselines/versions.lock.json'), 'utf8'));
  const { validateHeldout } = await import('../../scripts/check-phase10-heldout.mjs');
  const manifestErrors = validateHeldout(manifest, lock);
  if (manifestErrors.length) throw new Error(`manifest is not frozen/valid: ${manifestErrors.join('; ')}`);
  const candidates = ['Controlla', ...lock.baselines.map((b) => b.name)];
  const input = JSON.parse(await readFile(path.resolve(inputPath), 'utf8'));
  const inputErrors = validateRunManifest(input, manifest);
  if (inputErrors.length) throw new Error(inputErrors.join('; '));
  const report = analyze(input.runs, manifest, candidates);
  await writeFile(path.resolve(outputPath), JSON.stringify(report, null, 2) + '\n');
  console.log(`Analyzed ${report.rows} complete rows across ${manifest.tasks.length} frozen templates.`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch((error) => { console.error(error.message); process.exitCode = 1; });
