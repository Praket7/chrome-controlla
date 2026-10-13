import { createInterface } from 'node:readline';

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on('line', line => {
  let request;
  try { request = JSON.parse(line); } catch { return; }
  let result;
  if (request.method === 'tools/list') result = { tools: [{ name: 'browser', inputSchema: { type: 'object' } }] };
  else result = { content: [{ type: 'text', text: JSON.stringify({ status: 'verified', execution_route: { selected: 'qualified_skill' }, ...(process.env.TRACE_FIXTURE_NO_METRICS ? {} : { metrics: { browser: { cdp_commands: 3 }, internal_model_calls: 0 } }) }) }] };
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: request.id, result })}\n`);
});
