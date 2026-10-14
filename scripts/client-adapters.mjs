// Built-in per-client stdio launch adapter. This verifies that each checked-in
// client configuration launches Controlla and exposes the exact V3 tool schema;
// it deliberately does not create behavioral/live-client qualification claims.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { resolveClientAdapterConfig } from './run-live-client-smokes.mjs';
import { canonicalV3ToolNames, fingerprintToolSchema } from './client-smoke-contract.mjs';
import { buildClientHostInvocation, prepareClientHostConfig, registeredClientHostAdapters, resolveClientHostAdapter, runClientHostTask } from './client-host-adapters.mjs';

const ADAPTERS = Object.freeze({
  freebuff: { id: 'freebuff-stdio', transport: 'stdio' },
  opencode: { id: 'opencode-stdio', transport: 'stdio' },
  'claude-code': { id: 'claude-code-stdio', transport: 'stdio' },
  codex: { id: 'codex-stdio', transport: 'stdio' },
  'chatgpt-desktop': { id: 'chatgpt-desktop-plugin', transport: 'stdio' },
  'generic-mcp': { id: 'generic-stdio', transport: 'stdio' },
});
// Contract pin for the six-tool V3 surface. A schema change requires an
// intentional update of this value and the client qualification fixtures.
export const EXPECTED_V3_TOOL_SCHEMA_SHA256 = 'a7ecd252cc41f2f65e88ddf7857509384bbc258cd84eba936b2a80c9c8797bc0';
export const EXPECTED_CLIENT_TOOL_SCHEMA_SHA256 = Object.freeze({
  'chatgpt-desktop': '0bab2c961ca1e052b8aadb0619b18f65b9a32383cda5af4480d3b19bf7d1abc9',
});

function waitForResponse(child, id, timeoutMs, pending) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP request ${id} timed out after ${timeoutMs}ms`)); }, timeoutMs);
    pending.set(id, message => {
      clearTimeout(timer);
      if (message.error) reject(new Error(`MCP request ${id} failed: ${message.error.message ?? JSON.stringify(message.error)}`));
      else resolve(message.result);
    });
  });
}

export async function runClientMcpContract(clientId, row, options = {}) {
  const adapter = ADAPTERS[clientId];
  if (!adapter) throw new Error(`no built-in MCP launch adapter is registered for ${clientId}`);
  const launch = options.launch ?? await resolveClientAdapterConfig(clientId, row, options);
  const spawnProcess = options.spawn ?? spawn;
  const child = spawnProcess(launch.command, launch.args, { env: { ...process.env, ...launch.env }, stdio: ['pipe', 'pipe', 'pipe'] });
  const pending = new Map();
  const errors = [];
  child.stderr.on('data', chunk => { if (errors.join('').length < 4096) errors.push(chunk.toString()); });
  const lines = createInterface({ input: child.stdout });
  lines.on('line', line => {
    let message;
    try { message = JSON.parse(line); } catch { return; }
    if (Number.isSafeInteger(message.id)) pending.get(message.id)?.(message);
  });
  let nextId = 1;
  const request = async (method, params) => {
    const id = nextId++;
    const response = waitForResponse(child, id, options.timeoutMs ?? 5000, pending);
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    return response;
  };
  try {
    const initialized = await request('initialize', {
      protocolVersion: '2025-06-18',
      capabilities: {},
      clientInfo: { name: `controlla-${adapter.id}`, version: '1.0.0' },
    });
    if (!initialized?.serverInfo?.name || !initialized?.capabilities) throw new Error(`${clientId} adapter received an invalid MCP initialize result`);
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
    const listed = await request('tools/list', {});
    const tools = Array.isArray(listed?.tools) ? listed.tools : [];
    const names = tools.map(tool => tool.name).sort();
    const expected = canonicalV3ToolNames().sort();
    if (JSON.stringify(names) !== JSON.stringify(expected)) throw new Error(`${clientId} adapter tool list mismatch: expected ${expected.join(', ')}, got ${names.join(', ')}`);
    if (tools.some(tool => !tool.inputSchema || typeof tool.inputSchema !== 'object')) throw new Error(`${clientId} adapter found a tool without an input schema`);
    const schemaSha256 = fingerprintToolSchema(tools);
    const expectedSchemaSha256 = options.expectedSchemaSha256 ?? EXPECTED_CLIENT_TOOL_SCHEMA_SHA256[clientId] ?? EXPECTED_V3_TOOL_SCHEMA_SHA256;
    if (schemaSha256 !== expectedSchemaSha256) {
      throw new Error(`${clientId} adapter schema fingerprint mismatch: expected ${expectedSchemaSha256}, got ${schemaSha256}`);
    }
    return {
      client: clientId,
      adapter: adapter.id,
      adapter_kind: row.adapterKind,
      status: 'contract_verified',
      qualification_status: 'not_run',
      server: initialized.serverInfo,
      protocol_version: initialized.protocolVersion,
      process_id: child.pid,
      tools: names,
      schema_sha256: schemaSha256,
      integration_config_sha256: launch.config_sha256,
      state_dir: launch.state_dir,
    };
  } catch (error) {
    const stderr = errors.join('').trim();
    throw new Error(`${clientId} built-in MCP adapter failed: ${error.message}${stderr ? `; stderr: ${stderr}` : ''}`);
  } finally {
    for (const [id, resolve] of pending) { pending.delete(id); resolve({ error: { message: 'adapter stopped' } }); }
    lines.close();
    child.kill('SIGTERM');
  }
}

export function registeredClientMcpAdapters() {
  return Object.fromEntries(Object.entries(ADAPTERS).map(([client, adapter]) => [client, { ...adapter }]));
}

export { buildClientHostInvocation, prepareClientHostConfig, registeredClientHostAdapters, resolveClientHostAdapter, runClientHostTask };
