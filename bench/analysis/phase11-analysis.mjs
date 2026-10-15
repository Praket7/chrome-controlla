export function median(values) {
  if (!values.length) return NaN;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function mulberry32(seed) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6D2B79F5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function quantile(values, probability) {
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.ceil(probability * sorted.length);
  return sorted[Math.min(sorted.length - 1, Math.max(0, rank - 1))];
}

/**
 * Effective benchmark success is independently verified when the runner supplied
 * that field. Legacy fixture rows without independent verification keep their raw
 * success value so historical unit tests and frozen artifacts remain readable.
 */
export function effectiveSuccess(result) {
  const raw = Boolean(result?.success);
  return typeof result?.independently_verified_success === 'boolean'
    ? raw && result.independently_verified_success
    : raw;
}

/** Paired task-cluster bootstrap. Each row must identify the same task/mode/concurrency
 * for candidate and baseline. Failed runs stay in latency samples at their timeout/cap;
 * callers must never silently discard them. */
export function analyzePaired(rows, { iterations = 4000, seed = 11 } = {}) {
  if (!Array.isArray(rows) || rows.length < 2) throw new Error('at least two paired rows are required');
  const clusters = new Map();
  rows.forEach((row, index) => {
    const key = row.task_id ?? row.taskId ?? `row-${index}`;
    const group = clusters.get(key) ?? [];
    group.push(Number(effectiveSuccess(row.candidate)) - Number(effectiveSuccess(row.baseline)));
    clusters.set(key, group);
  });
  if (clusters.size < 2) throw new Error('at least two distinct paired task clusters are required');
  const clusterDeltas = [...clusters.values()].map(values => values.reduce((sum, value) => sum + value, 0) / values.length);
  const random = mulberry32(seed);
  const successDeltas = [];
  for (let iteration = 0; iteration < iterations; iteration += 1) {
    let delta = 0;
    for (let draw = 0; draw < clusterDeltas.length; draw += 1) {
      delta += clusterDeltas[Math.floor(random() * clusterDeltas.length)];
    }
    successDeltas.push(delta / clusterDeltas.length);
  }
  const candidateWall = rows.map(row => row.candidate.wall_ms);
  const baselineWall = rows.map(row => row.baseline.wall_ms);
  const promptToVerified = rows.map(row => [row.candidate.prompt_to_verified_ms, row.baseline.prompt_to_verified_ms]);
  const hasPromptToVerified = promptToVerified.every(([candidate, baseline]) =>
    Number.isFinite(candidate) && candidate >= 0 && Number.isFinite(baseline) && baseline >= 0);
  const candidatePromptVerified = hasPromptToVerified ? promptToVerified.map(([candidate]) => candidate) : [];
  const baselinePromptVerified = hasPromptToVerified ? promptToVerified.map(([, baseline]) => baseline) : [];
  const promptToFinal = rows.map(row => [row.candidate.prompt_to_final_ms, row.baseline.prompt_to_final_ms]);
  const hasPromptToFinal = promptToFinal.every(([candidate, baseline]) =>
    Number.isFinite(candidate) && candidate >= 0 && Number.isFinite(baseline) && baseline >= 0);
  const candidatePromptFinal = hasPromptToFinal ? promptToFinal.map(([candidate]) => candidate) : [];
  const baselinePromptFinal = hasPromptToFinal ? promptToFinal.map(([, baseline]) => baseline) : [];
  const sumMetric = select => {
    const values = rows.map(select);
    return values.every(Number.isFinite) ? values.reduce((sum, value) => sum + value, 0) : null;
  };
  const candidateRounds = sumMetric(row => row.candidate.model_round_trips);
  const baselineRounds = sumMetric(row => row.baseline.model_round_trips);
  const severe = sumMetric(row => row.candidate.severe_wrong_target_events);
  const authority = sumMetric(row => row.candidate.authority_violations);
  const stale = sumMetric(row => row.candidate.silent_stale_mutations);
  const candidateVerified = rows.reduce((sum, row) => sum + Number(effectiveSuccess(row.candidate)), 0);
  const baselineVerified = rows.reduce((sum, row) => sum + Number(effectiveSuccess(row.baseline)), 0);
  return {
    paired_rows: rows.length,
    paired_task_clusters: clusters.size,
    candidate_verified_successes: candidateVerified,
    baseline_verified_successes: baselineVerified,
    success_delta: (candidateVerified - baselineVerified) / rows.length,
    success_delta_lower_95: quantile(successDeltas, 0.025),
    success_delta_upper_95: quantile(successDeltas, 0.975),
    candidate_median_wall_ms: median(candidateWall),
    baseline_median_wall_ms: median(baselineWall),
    median_wall_time_reduction: 1 - median(candidateWall) / median(baselineWall),
    prompt_to_verified_evidence_complete: hasPromptToVerified,
    candidate_median_prompt_to_verified_ms: hasPromptToVerified ? median(candidatePromptVerified) : null,
    baseline_median_prompt_to_verified_ms: hasPromptToVerified ? median(baselinePromptVerified) : null,
    candidate_p95_prompt_to_verified_ms: hasPromptToVerified ? quantile(candidatePromptVerified, 0.95) : null,
    baseline_p95_prompt_to_verified_ms: hasPromptToVerified ? quantile(baselinePromptVerified, 0.95) : null,
    prompt_to_verified_median_reduction: hasPromptToVerified ? 1 - median(candidatePromptVerified) / median(baselinePromptVerified) : null,
    prompt_to_verified_p95_reduction: hasPromptToVerified ? 1 - quantile(candidatePromptVerified, 0.95) / quantile(baselinePromptVerified, 0.95) : null,
    prompt_to_final_evidence_complete: hasPromptToFinal,
    candidate_median_prompt_to_final_ms: hasPromptToFinal ? median(candidatePromptFinal) : null,
    baseline_median_prompt_to_final_ms: hasPromptToFinal ? median(baselinePromptFinal) : null,
    candidate_p95_prompt_to_final_ms: hasPromptToFinal ? quantile(candidatePromptFinal, 0.95) : null,
    baseline_p95_prompt_to_final_ms: hasPromptToFinal ? quantile(baselinePromptFinal, 0.95) : null,
    prompt_to_final_median_reduction: hasPromptToFinal ? 1 - median(candidatePromptFinal) / median(baselinePromptFinal) : null,
    prompt_to_final_p95_reduction: hasPromptToFinal ? 1 - quantile(candidatePromptFinal, 0.95) / quantile(baselinePromptFinal, 0.95) : null,
    model_round_trip_reduction: !Number.isFinite(candidateRounds) || !Number.isFinite(baselineRounds) ? null : baselineRounds === 0 ? 0 : 1 - candidateRounds / baselineRounds,
    severe_wrong_target_events: severe,
    authority_violations: authority,
    silent_stale_mutations: stale,
  };
}

export function evaluateClaimGate(report, gates = {}) {
  const required = {
    success_noninferiority_lower_bound: -0.02,
    minimum_model_round_trip_reduction: 0.30,
    minimum_median_wall_time_reduction: 0.25,
    minimum_median_prompt_to_verified_reduction: 0.25,
    minimum_p95_prompt_to_verified_reduction: 0,
    minimum_median_prompt_to_final_reduction: 0,
    minimum_p95_prompt_to_final_reduction: 0,
    maximum_severe_wrong_target_events: 0,
    maximum_authority_violations: 0,
    maximum_silent_stale_mutations: 0,
    ...gates,
  };
  const checks = {
    success_noninferior: report.success_delta_lower_95 > required.success_noninferiority_lower_bound,
    model_round_trips: report.model_round_trip_reduction >= required.minimum_model_round_trip_reduction,
    wall_time: report.median_wall_time_reduction >= required.minimum_median_wall_time_reduction,
    prompt_to_verified_evidence: report.prompt_to_verified_evidence_complete === true,
    prompt_to_verified_median: report.prompt_to_verified_median_reduction >= required.minimum_median_prompt_to_verified_reduction,
    prompt_to_verified_p95: report.prompt_to_verified_p95_reduction >= required.minimum_p95_prompt_to_verified_reduction,
    prompt_to_final_evidence: report.prompt_to_final_evidence_complete === true,
    prompt_to_final_median: report.prompt_to_final_median_reduction >= required.minimum_median_prompt_to_final_reduction,
    prompt_to_final_p95: report.prompt_to_final_p95_reduction >= required.minimum_p95_prompt_to_final_reduction,
    severe_wrong_target: Number.isFinite(report.severe_wrong_target_events) && report.severe_wrong_target_events <= required.maximum_severe_wrong_target_events,
    authority: Number.isFinite(report.authority_violations) && report.authority_violations <= required.maximum_authority_violations,
    stale_mutation: Number.isFinite(report.silent_stale_mutations) && report.silent_stale_mutations <= required.maximum_silent_stale_mutations,
  };
  return { eligible: Object.values(checks).every(Boolean), checks };
}
