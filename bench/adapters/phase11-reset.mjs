#!/usr/bin/env node
const chunks = [];
let bytes = 0;
for await (const chunk of process.stdin) {
  bytes += chunk.length;
  if (bytes > 65536) throw new Error('reset request too large');
  chunks.push(chunk);
}
const input = JSON.parse(Buffer.concat(chunks).toString());
const base = process.env.PHASE11_FIXTURE_BASE_URL;
const token = process.env.PHASE11_ORACLE_TOKEN;
if (!base || !token) throw new Error('fixture base URL and privileged oracle token are required');
const response = await fetch(`${base}/__reset`, {
  method: 'POST',
  headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
  body: JSON.stringify(input),
  signal: AbortSignal.timeout(Math.min(input.deadline_ms ?? 60000, 60000)),
});
if (!response.ok) throw new Error(`fixture reset failed: ${response.status}`);
console.log(JSON.stringify(await response.json()));
