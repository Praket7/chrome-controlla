import { createInterface } from 'node:readline';

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', line => {
  let request;
  try { request = JSON.parse(line); } catch { return; }
  let result;
  if (request.method === 'tools/list') result = { tools: [{ name: 'browser', inputSchema: { type: 'object' } }] };
  else if (process.env.TRACE_FIXTURE_MULTI_CALL && request.params?.arguments?.action === 'read') result = { content: [{ type: 'text', text: JSON.stringify({ status: 'observed' }) }] };
  else if (process.env.TRACE_FIXTURE_MULTI_CALL) result = { content: [{ type: 'text', text: JSON.stringify({ status: 'completed', receipts: [{ step: 'act', status: 'verified' }] }) }] };
  else if (process.env.TRACE_FIXTURE_PARTIAL) result = { content: [{ type: 'text', text: JSON.stringify({ status: 'incomplete', workflow: { status: 'incomplete', receipts: [{ step: 'verify', status: 'passed' }, { step: 'later', status: 'unknown' }] } }) }] };
  else if (process.env.TRACE_FIXTURE_DUPLICATE_FALLBACK) {
    const receipt = { status: 'completed', fallback_reason: 'semantic_probe_incomplete', fallback_count: 1,
      metrics: { browser: { cdp_commands: 1 }, host_round_trips: 1, observation_bytes: 10, screenshot_bytes: 0,
        ...(process.env.TRACE_FIXTURE_NO_INTERNAL !== '1' ? { internal_model_calls: 0 } : {}),
        fallback_count: process.env.TRACE_FIXTURE_CONFLICTING_FALLBACK === '1' ? 2 : 1 } };
    result = { structuredContent: receipt, content: [{ type: 'text', text: JSON.stringify(receipt) }] };
  }
  else result = { content: [{ type: 'text', text: JSON.stringify({ status: 'verified', execution_route: { selected: 'qualified_skill', ...(process.env.TRACE_FIXTURE_NO_METRICS ? {} : { actual_cost: { reported_internal_model_calls: 0 } }) }, ...(process.env.TRACE_FIXTURE_NO_METRICS ? {} : { metrics: { operation_id: 'mcp:fixture:1', browser: { cdp_commands: 3 }, host_round_trips: 2, native_host_messages_sent: 2, native_host_messages_received: 2, first_browser_op_us: 30, last_browser_op_us: 80, observation_bytes: 700, screenshot_bytes: 0, fallback_count: 0, phases: { queue_us: 10, readiness_us: 20 } } }) }) }] };
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: request.id, result })}\n`);
});
