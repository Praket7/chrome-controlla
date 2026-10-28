export const REQUIRED_METRIC_EVIDENCE_FIELDS = [
  'success','independently_verified_success','severe_wrong_target_events','authority_violations',
  'silent_stale_mutations','wall_ms','model_round_trips','model_tokens','mcp_calls','browser_ops',
  'prompt_to_verified_ms','prompt_to_final_ms','external_controlla_calls','model_turns',
  'model_visible_observations','internal_model_calls','browser_cdp_commands','host_round_trips',
  'native_host_messages_sent','native_host_messages_received','schema_bytes','prompt_input_bytes',
  'tool_input_bytes','output_bytes','observation_bytes','screenshot_bytes','unknown_delivery_count',
  'fallback_count','retries','timeout_count','cleanup_completed','cleanup_outcome','recovery_events',
  'cache_status','verification_ms','cpu_ms','peak_rss_bytes','focus_disruptions',
  'clipboard_disruptions','human_interventions','model_visible_calls','browser_commands',
  'native_host_commands','cdp_commands','full_ax_reads','frame_tree_reads','target_rebinds',
  'automatic_recoveries','user_visible_recoveries','application_accepted','input_commands',
  'verification_reads','auto_attach_calls'
];

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
  const missingMetricEvidence = [];
  for (const [rowIndex, row] of rows.entries()) for (const sideName of ['candidate', 'baseline']) {
    const side = row[sideName];
    for (const field of REQUIRED_METRIC_EVIDENCE_FIELDS) {
      const evidence = side?.metric_evidence?.[field];
      const observed = side && Object.hasOwn(side, field) && side[field] !== null && side[field] !== undefined
        && evidence && typeof evidence === 'object' && !Array.isArray(evidence)
        && evidence.s === 'observed' && typeof evidence.p === 'string' && evidence.p.trim().length > 0
        && !Object.hasOwn(evidence, 'r');
      if (!observed) missingMetricEvidence.push(`${sideName}[${rowIndex}].${field}`);
    }
  }
  const candidateVerifiedRows = rows.filter(row => effectiveSuccess(row.candidate)).map(row => row.candidate);
  const baselineVerifiedRows = rows.filter(row => effectiveSuccess(row.baseline)).map(row => row.baseline);
  const hasPromptToVerified = candidateVerifiedRows.length > 0 && baselineVerifiedRows.length > 0
    && [...candidateVerifiedRows, ...baselineVerifiedRows].every(result =>
      Number.isFinite(result.prompt_to_verified_ms) && result.prompt_to_verified_ms >= 0);
  const candidatePromptVerified = hasPromptToVerified ? candidateVerifiedRows.map(result => result.prompt_to_verified_ms) : [];
  const baselinePromptVerified = hasPromptToVerified ? baselineVerifiedRows.map(result => result.prompt_to_verified_ms) : [];
  const hasPromptToFinal = candidateVerifiedRows.length > 0 && baselineVerifiedRows.length > 0
    && [...candidateVerifiedRows, ...baselineVerifiedRows].every(result =>
      Number.isFinite(result.prompt_to_final_ms) && result.prompt_to_final_ms >= 0);
  const candidatePromptFinal = hasPromptToFinal ? candidateVerifiedRows.map(result => result.prompt_to_final_ms) : [];
  const baselinePromptFinal = hasPromptToFinal ? baselineVerifiedRows.map(result => result.prompt_to_final_ms) : [];
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
  const attemptTiming = side => {
    const values = rows.map(row => {
      const result = row[side];
      return effectiveSuccess(result) ? result.prompt_to_verified_ms : result.prompt_to_verifier_decision_ms;
    });
    const complete = values.every(value => Number.isFinite(value) && value >= 0);
    const successful = rows.filter(row => effectiveSuccess(row[side])).map(row => row[side].prompt_to_verified_ms);
    const successfulComplete = successful.length > 0 && successful.every(value => Number.isFinite(value) && value >= 0);
    return {
      allAttemptComplete: complete,
      allAttemptMedian: complete ? median(values) : null,
      allAttemptP95: complete ? quantile(values, 0.95) : null,
      successConditionedComplete: successfulComplete,
      successConditionedCount: successful.length,
      successConditionedMedian: successfulComplete ? median(successful) : null,
      successConditionedP95: successfulComplete ? quantile(successful, 0.95) : null,
    };
  };
  const candidateTiming = attemptTiming('candidate');
  const baselineTiming = attemptTiming('baseline');
  const allAttemptMedianReduction = candidateTiming.allAttemptComplete && baselineTiming.allAttemptComplete
    ? 1 - candidateTiming.allAttemptMedian / baselineTiming.allAttemptMedian : null;
  const allAttemptP95Reduction = candidateTiming.allAttemptComplete && baselineTiming.allAttemptComplete
    ? 1 - candidateTiming.allAttemptP95 / baselineTiming.allAttemptP95 : null;
  const reliability = side => {
    const groups = new Map();
    for (const [index, row] of rows.entries()) {
      const key = JSON.stringify([row.task_id ?? row.taskId ?? `row-${index}`, row.mode ?? null,
        row.execution_mode ?? null, row.concurrency ?? null]);
      const values = groups.get(key) ?? [];
      values.push(effectiveSuccess(row[side]));
      groups.set(key, values);
    }
    return [...groups.values()].filter(values => values.length > 1).length === 0 ? null
      : [...groups.values()].filter(values => values.length > 1 && values.every(Boolean)).length
        / [...groups.values()].filter(values => values.length > 1).length;
  };
  const falseSuccessCount = side => rows.every(row => typeof row[side]?.independently_verified_success === 'boolean')
    ? rows.filter(row => Boolean(row[side]?.success) && !row[side].independently_verified_success).length : null;
  const candidateReliability = reliability('candidate');
  const baselineReliability = reliability('baseline');
  return {
    paired_rows: rows.length,
    paired_task_clusters: clusters.size,
    metric_evidence_complete: missingMetricEvidence.length === 0,
    missing_metric_evidence: missingMetricEvidence,
    candidate_verified_successes: candidateVerified,
    baseline_verified_successes: baselineVerified,
    success_delta: (candidateVerified - baselineVerified) / rows.length,
    candidate_false_successes: falseSuccessCount('candidate'),
    baseline_false_successes: falseSuccessCount('baseline'),
    repeated_run_reliability_complete: candidateReliability !== null && baselineReliability !== null,
    candidate_repeated_run_reliability: candidateReliability,
    baseline_repeated_run_reliability: baselineReliability,
    all_attempt_time_to_outcome_complete: candidateTiming.allAttemptComplete && baselineTiming.allAttemptComplete,
    candidate_median_all_attempt_time_to_outcome_ms: candidateTiming.allAttemptMedian,
    baseline_median_all_attempt_time_to_outcome_ms: baselineTiming.allAttemptMedian,
    candidate_p95_all_attempt_time_to_outcome_ms: candidateTiming.allAttemptP95,
    baseline_p95_all_attempt_time_to_outcome_ms: baselineTiming.allAttemptP95,
    all_attempt_time_to_outcome_median_reduction: allAttemptMedianReduction,
    all_attempt_time_to_outcome_p95_reduction: allAttemptP95Reduction,
    success_conditioned_latency_complete: candidateTiming.successConditionedComplete && baselineTiming.successConditionedComplete,
    candidate_success_conditioned_attempts: candidateTiming.successConditionedCount,
    baseline_success_conditioned_attempts: baselineTiming.successConditionedCount,
    candidate_median_success_conditioned_prompt_to_verified_ms: candidateTiming.successConditionedMedian,
    baseline_median_success_conditioned_prompt_to_verified_ms: baselineTiming.successConditionedMedian,
    candidate_p95_success_conditioned_prompt_to_verified_ms: candidateTiming.successConditionedP95,
    baseline_p95_success_conditioned_prompt_to_verified_ms: baselineTiming.successConditionedP95,
    success_delta_lower_95: quantile(successDeltas, 0.025),
    success_delta_upper_95: quantile(successDeltas, 0.975),
    candidate_median_wall_ms: median(candidateWall),
    baseline_median_wall_ms: median(baselineWall),
    median_wall_time_reduction: 1 - median(candidateWall) / median(baselineWall),
    prompt_to_verified_evidence_complete: hasPromptToVerified,
    candidate_median_prompt_to_verified_ms: hasPromptToVerified ? candidateTiming.successConditionedMedian : null,
    baseline_median_prompt_to_verified_ms: hasPromptToVerified ? baselineTiming.successConditionedMedian : null,
    candidate_p95_prompt_to_verified_ms: hasPromptToVerified ? candidateTiming.successConditionedP95 : null,
    baseline_p95_prompt_to_verified_ms: hasPromptToVerified ? baselineTiming.successConditionedP95 : null,
    prompt_to_verified_median_reduction: hasPromptToVerified ? 1 - candidateTiming.successConditionedMedian / baselineTiming.successConditionedMedian : null,
    prompt_to_verified_p95_reduction: hasPromptToVerified ? 1 - candidateTiming.successConditionedP95 / baselineTiming.successConditionedP95 : null,
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
    minimum_median_all_attempt_time_reduction: 0,
    minimum_p95_all_attempt_time_reduction: 0,
    ...gates,
  };
  const checks = {
    metric_evidence: report.metric_evidence_complete === true,
    success_noninferior: report.success_delta_lower_95 > required.success_noninferiority_lower_bound,
    model_round_trips: report.model_round_trip_reduction >= required.minimum_model_round_trip_reduction,
    wall_time: report.median_wall_time_reduction >= required.minimum_median_wall_time_reduction,
    prompt_to_verified_evidence: report.prompt_to_verified_evidence_complete === true,
    all_attempt_time_to_outcome_evidence: report.all_attempt_time_to_outcome_complete === true,
    all_attempt_time_to_outcome_median: report.all_attempt_time_to_outcome_median_reduction >= required.minimum_median_all_attempt_time_reduction,
    all_attempt_time_to_outcome_p95: report.all_attempt_time_to_outcome_p95_reduction >= required.minimum_p95_all_attempt_time_reduction,
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
