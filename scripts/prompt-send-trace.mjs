import { mkdir, readFile, appendFile, stat } from 'node:fs/promises';
import { dirname } from 'node:path';

export const PROMPT_TRACE_EVENTS = Object.freeze([
  'prompt_sent', 'prompt_input_written', 'first_tool_decision', 'client_tool_decision', 'controlla_received', 'browser_operation',
  'verified_outcome', 'schema_delivered', 'tool_result_delivered', 'final_answer', 'model_turn',
  'model_visible_observation', 'process_start', 'process_end',
]);

const MAX_TRACE_FILE_BYTES = 8 * 1024 * 1024;
const MAX_TRACE_LINE_BYTES = 16 * 1024;
const ACCEPTED_PROMPT_BOUNDARIES = new Set(['actual_user_submit']);

export async function recordPromptTraceEvent(path, traceId, event, fields = {}, monotonicNs = process.hrtime.bigint()) {
  if (typeof path !== 'string' || !path || typeof traceId !== 'string' || !traceId
    || !PROMPT_TRACE_EVENTS.includes(event) || typeof monotonicNs !== 'bigint' || monotonicNs < 0n) {
    throw new Error('invalid prompt trace event');
  }
  const reserved = new Set(['schema_version', 'trace_id', 'event', 'monotonic_ns', 'span_id', 'parent_span_id', 'operation_id', 'clock_domain', 'provenance', 'build_id']);
  const safeFields = Object.fromEntries(Object.entries(fields).filter(([key, value]) => !reserved.has(key)
    && (typeof value === 'string' || typeof value === 'boolean'
      || Number.isSafeInteger(value) && value >= 0
      || key === 'clock_uncertainty_ms' && Number.isFinite(value) && value >= 0 && value <= 5)));
  const spanId = `${event}:${monotonicNs}`;
  const operationId = typeof fields.operation_id === 'string' ? fields.operation_id.slice(0, 128)
    : typeof fields.call_id === 'string' ? fields.call_id.slice(0, 128) : traceId.slice(0, 128);
  const envelope = {
    schema_version: 2,
    trace_id: traceId.slice(0, 128),
    span_id: spanId,
    parent_span_id: typeof fields.parent_span_id === 'string' ? fields.parent_span_id.slice(0, 128) : null,
    operation_id: operationId,
    clock_domain: 'local_host_monotonic',
    monotonic_ns: monotonicNs.toString(),
    boundary: event,
    provenance: typeof fields.provenance === 'string' ? fields.provenance.slice(0, 80) : 'client_adapter_hook',
    build_id: typeof fields.build_id === 'string' ? fields.build_id.slice(0, 128) : process.env.CONTROLLA_BUILD_ID ?? 'unknown',
    event,
    ...safeFields,
  };
  const line = `${JSON.stringify(envelope)}\n`;
  if (Buffer.byteLength(line) > MAX_TRACE_LINE_BYTES) throw new Error('prompt trace event exceeds line limit');
  await mkdir(dirname(path), { recursive: true });
  const currentBytes = await stat(path).then(info => info.size).catch(error => error.code === 'ENOENT' ? 0 : Promise.reject(error));
  if (currentBytes + Buffer.byteLength(line) > MAX_TRACE_FILE_BYTES) throw new Error('prompt trace exceeds file limit');
  await appendFile(path, line, { mode: 0o600 });
  return JSON.parse(line);
}

export async function readPromptTrace(path) {
  const text = await readFile(path, 'utf8');
  if (Buffer.byteLength(text) > MAX_TRACE_FILE_BYTES) throw new Error('prompt trace exceeds file limit');
  return text.split(/\r?\n/).filter(Boolean).map((line, index) => {
    if (Buffer.byteLength(line) > MAX_TRACE_LINE_BYTES) throw new Error(`prompt trace line ${index + 1} exceeds limit`);
    const event = JSON.parse(line);
    if (![1, 2].includes(event.schema_version) || !PROMPT_TRACE_EVENTS.includes(event.event)
      || typeof event.trace_id !== 'string' || !/^\d+$/.test(event.monotonic_ns)
      || event.schema_version === 2 && (typeof event.span_id !== 'string' || typeof event.operation_id !== 'string'
        || event.clock_domain !== 'local_host_monotonic' || event.boundary !== event.event
        || typeof event.provenance !== 'string' || typeof event.build_id !== 'string')) {
      throw new Error(`invalid prompt trace event at line ${index + 1}`);
    }
    return event;
  });
}

/** Read independently written traces and merge them in their shared monotonic clock domain. */
export async function readPromptTraceFiles(paths, expectedTraceId) {
  if (!Array.isArray(paths) || paths.length === 0 || paths.length > 2
    || paths.some(path => typeof path !== 'string' || !path)
    || typeof expectedTraceId !== 'string' || !expectedTraceId || expectedTraceId.length > 128) {
    throw new Error('prompt trace paths and expected trace ID are required and bounded');
  }
  const streams = await Promise.all(paths.map(async path => {
    try { return await readPromptTrace(path); }
    catch (error) {
      if (error.code === 'ENOENT') return [];
      throw error;
    }
  }));
  const events = streams.flatMap((stream, streamIndex) => stream.map((event, streamOrder) => ({
    event,
    streamIndex,
    streamOrder,
  })));
  const combinedBytes = events.reduce((total, item) => total + Buffer.byteLength(`${JSON.stringify(item.event)}\n`), 0);
  if (combinedBytes > MAX_TRACE_FILE_BYTES) throw new Error('combined prompt trace exceeds file limit');
  if (events.some(({ event }) => event.trace_id !== expectedTraceId)) {
    throw new Error('prompt trace contains an event from a different run');
  }
  events.sort((left, right) => {
    const leftTime = BigInt(left.event.monotonic_ns);
    const rightTime = BigInt(right.event.monotonic_ns);
    return leftTime < rightTime ? -1 : leftTime > rightTime ? 1
      : left.streamIndex - right.streamIndex || left.streamOrder - right.streamOrder;
  });
  return events.map(item => item.event);
}

export function summarizePromptTrace(events) {
  const ids = new Set(events.map(event => event.trace_id));
  const eventTimes = new Map();
  const allPromptEvents = events.filter(event => event.event === 'prompt_sent');
  const promptEvents = allPromptEvents.filter(event => ACCEPTED_PROMPT_BOUNDARIES.has(event.prompt_boundary));
  let traceValid = ids.size <= 1 && allPromptEvents.length <= 1;
  let previousTraceTime;
  for (const event of events) {
    if (!PROMPT_TRACE_EVENTS.includes(event.event) || !/^\d+$/.test(String(event.monotonic_ns))) {
      traceValid = false;
      continue;
    }
    const time = BigInt(event.monotonic_ns);
    if (previousTraceTime !== undefined && time < previousTraceTime) traceValid = false;
    previousTraceTime = time;
    const prior = eventTimes.get(event.event);
    if (event.event === 'browser_operation') {
      const list = prior ?? [];
      list.push(time);
      eventTimes.set(event.event, list);
    } else if (event.event === 'prompt_sent' && !ACCEPTED_PROMPT_BOUNDARIES.has(event.prompt_boundary)) {
      continue;
    } else if (prior === undefined || event.event === 'final_answer' && time > prior) {
      eventTimes.set(event.event, time);
    }
  }
  const prompt = eventTimes.get('prompt_sent');
  const promptEvent = promptEvents[0];
  const first = name => eventTimes.get(name);
  const lastBrowser = eventTimes.get('browser_operation');
  const callEvents = new Map();
  for (const event of events) {
    if (!['controlla_received', 'browser_operation', 'tool_result_delivered'].includes(event.event)
      || typeof event.call_id !== 'string') continue;
    const call = callEvents.get(event.call_id) ?? {};
    const time = BigInt(event.monotonic_ns);
    if (event.event === 'browser_operation') call.browser = time;
    else call[event.event] = time;
    if (event.event === 'tool_result_delivered') {
      for (const field of ['internal_model_calls', 'host_round_trips', 'observation_bytes', 'screenshot_bytes', 'cdp_commands', 'fallback_count']) {
        if (Number.isSafeInteger(event[field]) && event[field] >= 0) call[field] = event[field];
      }
      call.missing_counters = typeof event.missing_counters === 'string'
        ? event.missing_counters.split(',').filter(Boolean) : [];
    }
    callEvents.set(event.call_id, call);
  }
  if (callEvents.size) {
    const promptDecision = [first('prompt_sent'), first('first_tool_decision')].filter(time => time !== undefined);
    if (promptDecision.some((time, index) => index > 0 && time < promptDecision[index - 1])) traceValid = false;
    for (const call of callEvents.values()) {
      const ordered = [call.controlla_received, call.browser, call.tool_result_delivered].filter(time => time !== undefined);
      if (ordered.some((time, index) => index > 0 && time < ordered[index - 1])) traceValid = false;
      if (prompt !== undefined && ordered.some(time => time < prompt)) traceValid = false;
    }
    const lastResult = [...callEvents.values()].map(call => call.tool_result_delivered).filter(time => time !== undefined)
      .reduce((latest, time) => latest === undefined || time > latest ? time : latest, undefined);
    if (lastResult !== undefined && first('final_answer') !== undefined && first('final_answer') < lastResult) traceValid = false;
  } else {
    const orderedPipeline = [
      first('prompt_sent'), first('first_tool_decision'), first('controlla_received'),
      lastBrowser?.[0], first('tool_result_delivered'), first('final_answer'),
    ].filter(time => time !== undefined);
    if (orderedPipeline.some((time, index) => index > 0 && time < orderedPipeline[index - 1])) traceValid = false;
  }
  const timeOf = name => name === 'first_browser_op' ? lastBrowser?.[0]
    : name === 'last_browser_op' ? lastBrowser?.at(-1) : first(name);
  const elapsed = name => {
    const target = timeOf(name);
    if (!traceValid) return null;
    if (prompt === undefined || target === undefined) return null;
    if (target < prompt) { traceValid = false; return null; }
    return Number(target - prompt) / 1_000_000;
  };
  const sum = field => {
    const values = events.map(event => event[field]).filter(value => Number.isSafeInteger(value) && value >= 0);
    return values.length ? values.reduce((total, value) => total + value, 0) : null;
  };
  const distinctCalls = name => {
    const keyed = new Map();
    let unkeyed = 0;
    for (const event of events.filter(item => item.event === name)) {
      if (typeof event.call_id !== 'string') { unkeyed++; continue; }
      keyed.set(event.call_id, true);
    }
    return keyed.size + unkeyed;
  };
  const externalCalls = distinctCalls('controlla_received');
  const modelTurns = events.filter(event => event.event === 'model_turn').length;
  const observations = events.filter(event => event.event === 'model_visible_observation').length;
  const clientToolDecisions = events.filter(event => event.event === 'client_tool_decision'
    && typeof event.tool === 'string' && typeof event.client_call_id === 'string');
  const mcpToolCalls = events.filter(event => event.event === 'controlla_received'
    && typeof event.tool === 'string' && typeof event.call_id === 'string');
  const callCorrelations = [];
  const correlatedClientIds = new Set();
  const correlatedMcpIds = new Set();
  // Join only an unambiguous, same-name one-to-one pair. Tool-name/order guesses
  // are not promoted to call identity when either side has repeated candidates.
  for (const client of clientToolDecisions) {
    const candidates = mcpToolCalls.filter(mcp => mcp.tool === client.tool && !correlatedMcpIds.has(mcp.call_id));
    const sameNameClients = clientToolDecisions.filter(item => item.tool === client.tool
      && !correlatedClientIds.has(item.client_call_id));
    if (candidates.length !== 1 || sameNameClients.length !== 1) continue;
    const mcp = candidates[0];
    callCorrelations.push({ client_call_id: client.client_call_id, mcp_call_id: mcp.call_id,
      tool: client.tool, method: 'unique_tool_name_within_trace',
      client_span_id: typeof client.span_id === 'string' ? client.span_id : null,
      mcp_span_id: typeof mcp.span_id === 'string' ? mcp.span_id : null });
    correlatedClientIds.add(client.client_call_id);
    correlatedMcpIds.add(mcp.call_id);
  }
  const unmatchedClientCallIds = clientToolDecisions.map(event => event.client_call_id)
    .filter(id => !correlatedClientIds.has(id));
  const unmatchedMcpCallIds = mcpToolCalls.map(event => event.call_id)
    .filter(id => !correlatedMcpIds.has(id));
  const hasHostEvents = events.some(event => event.event === 'model_turn' || event.event === 'model_visible_observation');
  const toolResults = events.filter(event => event.event === 'tool_result_delivered');
  const perCallCounter = (sourceEvents, field, missingField = field) => {
    const grouped = new Map();
    const unkeyed = [];
    for (const event of sourceEvents) {
      const reportedMissing = typeof event.missing_counters === 'string'
        && event.missing_counters.split(',').includes(missingField);
      const value = !reportedMissing && Number.isSafeInteger(event[field]) && event[field] >= 0 ? event[field] : null;
      if (typeof event.call_id !== 'string') { unkeyed.push(value); continue; }
      const values = grouped.get(event.call_id) ?? [];
      values.push(value);
      grouped.set(event.call_id, values);
    }
    const perCall = [...grouped.values()].map(values => {
      const observed = [...new Set(values)];
      return observed.length === 1 && observed[0] !== null ? observed[0] : null;
    });
    const values = [...perCall, ...unkeyed];
    if (!values.length || values.some(value => value === null)) return null;
    const total = values.reduce((sum, value) => sum + value, 0);
    return Number.isSafeInteger(total) ? total : null;
  };
  const resultCounterNames = ['internal_model_calls', 'cdp_commands', 'host_round_trips',
    'observation_bytes', 'screenshot_bytes', 'fallback_count', 'output_bytes'];
  const resultCounters = Object.fromEntries(resultCounterNames.map(name => [name, perCallCounter(toolResults, name)]));
  const browserCommandEvents = events.filter(event => event.event === 'browser_operation');
  const browserCommands = perCallCounter(browserCommandEvents.some(event => Object.hasOwn(event, 'cdp_commands'))
    ? browserCommandEvents : toolResults, 'cdp_commands');
  const tier = events.map(event => event.selected_execution_tier).find(value => typeof value === 'string') ?? null;
  const warmth = events.map(event => event.cold_warm).find(value => value === 'cold' || value === 'warm') ?? 'unknown';
  const missingCounters = [...new Set(events.flatMap(event => typeof event.missing_counters === 'string'
    ? event.missing_counters.split(',').filter(Boolean) : []))].sort();
  for (const [name, value] of Object.entries(resultCounters)) {
    const reportedMissing = toolResults.some(event => typeof event.missing_counters === 'string'
      && event.missing_counters.split(',').includes(name));
    const conflictingOrIncomplete = toolResults.some(event => Object.hasOwn(event, name));
    const required = ['internal_model_calls', 'cdp_commands', 'host_round_trips',
      'observation_bytes', 'screenshot_bytes', 'fallback_count'].includes(name);
    if (value === null && (required || reportedMissing || conflictingOrIncomplete)) missingCounters.push(name);
  }
  if (browserCommands === null && browserCommandEvents.some(event => Object.hasOwn(event, 'cdp_commands'))) missingCounters.push('cdp_commands');
  const receivedInputBytes = perCallCounter(events.filter(event => event.event === 'controlla_received'), 'tool_input_bytes');
  if (receivedInputBytes === null && externalCalls > 0) missingCounters.push('tool_input_bytes');
  const normalizedMissingCounters = [...new Set(missingCounters)].sort();
  const partialCounters = events.some(event => event.partial_counters === true) || normalizedMissingCounters.length > 0;
  const promptToFirstToolDecisionMs = elapsed('first_tool_decision');
  const promptToControllaReceivedMs = elapsed('controlla_received');
  const promptToFirstBrowserOpMs = elapsed('first_browser_op');
  const promptToLastBrowserOpMs = elapsed('last_browser_op');
  const promptToVerifiedMs = elapsed('verified_outcome');
  const promptToToolResultDeliveredMs = elapsed('tool_result_delivered');
  const promptToFinalMs = elapsed('final_answer');
  const processStart = events.find(event => event.event === 'process_start');
  const processEnd = [...events].reverse().find(event => event.event === 'process_end');
  const processStartToExitMs = processStart && processEnd
    ? Number(BigInt(processEnd.monotonic_ns) - BigInt(processStart.monotonic_ns)) / 1_000_000 : null;
  return {
    schema_version: 1,
    trace_id: ids.size === 1 ? [...ids][0] : null,
    trace_valid: traceValid,
    prompt_sent_observed: promptEvent !== undefined,
    process_start_monotonic_ns: processStart?.monotonic_ns ?? null,
    process_end_monotonic_ns: processEnd?.monotonic_ns ?? null,
    process_start_to_exit_ms: processStartToExitMs,
    prompt_boundary: typeof promptEvent?.prompt_boundary === 'string' ? promptEvent.prompt_boundary : null,
    prompt_sent_monotonic_ns: typeof promptEvent?.monotonic_ns === 'string' && /^\d+$/.test(promptEvent.monotonic_ns)
      ? promptEvent.monotonic_ns : null,
    clock_domain: promptEvent?.clock_domain ?? null,
    clock_offset_ms: Number.isFinite(promptEvent?.clock_offset_ms) ? promptEvent.clock_offset_ms : null,
    clock_uncertainty_ms: Number.isFinite(promptEvent?.clock_uncertainty_ms) ? promptEvent.clock_uncertainty_ms : null,
    prompt_to_first_tool_decision_ms: promptToFirstToolDecisionMs,
    prompt_to_controlla_received_ms: promptToControllaReceivedMs,
    prompt_to_first_browser_op_ms: promptToFirstBrowserOpMs,
    prompt_to_last_browser_op_ms: promptToLastBrowserOpMs,
    prompt_to_verified_ms: promptToVerifiedMs,
    prompt_to_tool_result_delivered_ms: promptToToolResultDeliveredMs,
    prompt_to_final_ms: promptToFinalMs,
    final_answer_observed: first('final_answer') !== undefined,
    counters: {
      external_controlla_calls: externalCalls,
      model_turns: hasHostEvents ? modelTurns : null,
      model_visible_observations: hasHostEvents ? observations : null,
      internal_model_calls: resultCounters.internal_model_calls,
      browser_cdp_commands: browserCommands,
      host_round_trips: resultCounters.host_round_trips,
      observation_bytes: resultCounters.observation_bytes,
      screenshot_bytes: resultCounters.screenshot_bytes,
      partial_counters: partialCounters,
      missing_counters: normalizedMissingCounters.join(','),
      prompt_input_bytes: sum('prompt_input_bytes'),
      tool_input_bytes: receivedInputBytes,
      output_bytes: resultCounters.output_bytes,
      schema_bytes: sum('schema_bytes'),
      fallback_count: resultCounters.fallback_count,
    },
    cold_warm: warmth,
    execution_tier: tier,
    call_correlations: callCorrelations,
    unmatched_client_call_ids: unmatchedClientCallIds,
    unmatched_mcp_call_ids: unmatchedMcpCallIds,
  };
}
