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
  else result = { content: [{ type: 'text', text: JSON.stringify({ status: 'verified', execution_route: { selected: 'qualified_skill', ...(process.env.TRACE_FIXTURE_NO_METRICS ? {} : { actual_cost: { reported_internal_model_calls: 0 } }) }, ...(process.env.TRACE_FIXTURE_NO_METRICS ? {} : { metrics: { browser: { cdp_commands: 3 } } }) }) }] };
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: request.id, result })}\n`);
});
