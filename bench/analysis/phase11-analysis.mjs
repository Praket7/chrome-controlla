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
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.floor(probability * sorted.length)))];
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
  const random = mulberry32(seed);
  const successDeltas = [];
  for (let iteration = 0; iteration < iterations; iteration += 1) {
    let delta = 0;
    for (let draw = 0; draw < rows.length; draw += 1) {
      const row = rows[Math.floor(random() * rows.length)];
      delta += Number(effectiveSuccess(row.candidate)) - Number(effectiveSuccess(row.baseline));
    }
    successDeltas.push(delta / rows.length);
  }
  const candidateWall = rows.map(row => row.candidate.wall_ms);
  const baselineWall = rows.map(row => row.baseline.wall_ms);
  const candidateRounds = rows.reduce((sum, row) => sum + row.candidate.model_round_trips, 0);
  const baselineRounds = rows.reduce((sum, row) => sum + row.baseline.model_round_trips, 0);
  const severe = rows.reduce((sum, row) => sum + row.candidate.severe_wrong_target_events, 0);
  const authority = rows.reduce((sum, row) => sum + row.candidate.authority_violations, 0);
  const stale = rows.reduce((sum, row) => sum + row.candidate.silent_stale_mutations, 0);
  const candidateVerified = rows.reduce((sum, row) => sum + Number(effectiveSuccess(row.candidate)), 0);
  const baselineVerified = rows.reduce((sum, row) => sum + Number(effectiveSuccess(row.baseline)), 0);
  return {
    paired_rows: rows.length,
    candidate_verified_successes: candidateVerified,
    baseline_verified_successes: baselineVerified,
    success_delta: (candidateVerified - baselineVerified) / rows.length,
    success_delta_lower_95: quantile(successDeltas, 0.025),
    success_delta_upper_95: quantile(successDeltas, 0.975),
    candidate_median_wall_ms: median(candidateWall),
    baseline_median_wall_ms: median(baselineWall),
    median_wall_time_reduction: 1 - median(candidateWall) / median(baselineWall),
    model_round_trip_reduction: baselineRounds === 0 ? 0 : 1 - candidateRounds / baselineRounds,
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
    maximum_severe_wrong_target_events: 0,
    maximum_authority_violations: 0,
    maximum_silent_stale_mutations: 0,
    ...gates,
  };
  const checks = {
    success_noninferior: report.success_delta_lower_95 > required.success_noninferiority_lower_bound,
    model_round_trips: report.model_round_trip_reduction >= required.minimum_model_round_trip_reduction,
    wall_time: report.median_wall_time_reduction >= required.minimum_median_wall_time_reduction,
    severe_wrong_target: report.severe_wrong_target_events <= required.maximum_severe_wrong_target_events,
    authority: report.authority_violations <= required.maximum_authority_violations,
    stale_mutation: report.silent_stale_mutations <= required.maximum_silent_stale_mutations,
  };
  return { eligible: Object.values(checks).every(Boolean), checks };
}
