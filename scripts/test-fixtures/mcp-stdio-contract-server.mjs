#!/usr/bin/env node
import { createInterface } from 'node:readline';
const tools = ['act', 'browser', 'extract', 'snapshot', 'verify', 'workflow'].map(name => ({
  name, inputSchema: { type: 'object', properties: {}, additionalProperties: false },
}));
for await (const line of createInterface({ input: process.stdin })) {
  let request;
  try { request = JSON.parse(line); } catch { continue; }
  if (!Object.hasOwn(request, 'id')) continue;
  const result = request.method === 'initialize'
    ? { protocolVersion: request.params.protocolVersion, capabilities: { tools: { listChanged: false } }, serverInfo: { name: 'controlla-test', version: '0.1.0' } }
    : request.method === 'tools/list' ? { tools } : {};
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: request.id, result })}\n`);
}
