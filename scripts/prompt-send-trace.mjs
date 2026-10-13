import { mkdir, readFile, appendFile, stat } from 'node:fs/promises';
import { dirname } from 'node:path';

export const PROMPT_TRACE_EVENTS = Object.freeze([
  'prompt_sent', 'first_tool_decision', 'controlla_received', 'browser_operation',
  'verified_outcome', 'tool_result_delivered', 'final_answer', 'model_turn',
  'model_visible_observation',
]);

const MAX_TRACE_FILE_BYTES = 8 * 1024 * 1024;
const MAX_TRACE_LINE_BYTES = 16 * 1024;

export async function recordPromptTraceEvent(path, traceId, event, fields = {}, monotonicNs = process.hrtime.bigint()) {
  if (typeof path !== 'string' || !path || typeof traceId !== 'string' || !traceId
    || !PROMPT_TRACE_EVENTS.includes(event) || typeof monotonicNs !== 'bigint' || monotonicNs < 0n) {
    throw new Error('invalid prompt trace event');
  }
  const reserved = new Set(['schema_version', 'trace_id', 'event', 'monotonic_ns']);
  const safeFields = Object.fromEntries(Object.entries(fields).filter(([key, value]) => !reserved.has(key)
    && (typeof value === 'string' || typeof value === 'boolean' || Number.isSafeInteger(value) && value >= 0)));
  const line = `${JSON.stringify({ schema_version: 1, trace_id: traceId, event, monotonic_ns: monotonicNs.toString(), ...safeFields })}\n`;
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
    if (event.schema_version !== 1 || !PROMPT_TRACE_EVENTS.includes(event.event)
      || typeof event.trace_id !== 'string' || !/^\d+$/.test(event.monotonic_ns)) {
      throw new Error(`invalid prompt trace event at line ${index + 1}`);
    }
    return event;
  });
}

export function summarizePromptTrace(events) {
  const ids = new Set(events.map(event => event.trace_id));
  const eventTimes = new Map();
  let traceValid = ids.size <= 1;
  for (const event of events) {
    if (!PROMPT_TRACE_EVENTS.includes(event.event) || !/^\d+$/.test(String(event.monotonic_ns))) {
      traceValid = false;
      continue;
    }
    const time = BigInt(event.monotonic_ns);
    const prior = eventTimes.get(event.event);
    if (event.event === 'browser_operation') {
      const list = prior ?? [];
      list.push(time);
      eventTimes.set(event.event, list);
    } else if (prior === undefined || event.event === 'final_answer' && time > prior) {
      eventTimes.set(event.event, time);
    }
  }
  const prompt = eventTimes.get('prompt_sent');
  const first = name => eventTimes.get(name);
  const lastBrowser = eventTimes.get('browser_operation');
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
  const externalCalls = events.filter(event => event.event === 'controlla_received').length;
  const modelTurns = events.filter(event => event.event === 'model_turn').length;
  const observations = events.filter(event => event.event === 'model_visible_observation').length;
  const hasHostEvents = events.some(event => event.event === 'model_turn' || event.event === 'model_visible_observation');
  const modelCalls = events.map(event => event.internal_model_calls).find(Number.isSafeInteger);
  const tier = events.map(event => event.selected_execution_tier).find(value => typeof value === 'string') ?? null;
  const warmth = events.map(event => event.cold_warm).find(value => value === 'cold' || value === 'warm') ?? 'unknown';
  return {
    schema_version: 1,
    trace_id: ids.size === 1 ? [...ids][0] : null,
    trace_valid: traceValid,
    prompt_to_first_tool_decision_ms: elapsed('first_tool_decision'),
    prompt_to_controlla_received_ms: elapsed('controlla_received'),
    prompt_to_first_browser_op_ms: elapsed('first_browser_op'),
    prompt_to_last_browser_op_ms: elapsed('last_browser_op'),
    prompt_to_verified_ms: elapsed('verified_outcome'),
    prompt_to_tool_result_delivered_ms: elapsed('tool_result_delivered'),
    prompt_to_final_ms: elapsed('final_answer'),
    counters: {
      external_controlla_calls: externalCalls,
      model_turns: hasHostEvents ? modelTurns : null,
      model_visible_observations: hasHostEvents ? observations : null,
      internal_model_calls: modelCalls ?? null,
      browser_cdp_commands: sum('cdp_commands'),
      input_bytes: sum('input_bytes'),
      output_bytes: sum('output_bytes'),
      schema_bytes: sum('schema_bytes'),
      fallback_count: sum('fallback_count'),
    },
    cold_warm: warmth,
    execution_tier: tier,
  };
}
