#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');
const readline = require('node:readline');
const path = require('node:path');

function parseArgs(argv) {
  const out = { serverArgs: [], env: {}, timeout: 60_000 };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i], value = argv[++i];
    if (!value) throw new Error(`${flag} requires a value`);
    if (flag === '--server') out.server = value;
    else if (flag === '--server-arg') out.serverArgs.push(value);
    else if (flag === '--env') {
      const at = value.indexOf('=');
      if (at < 1) throw new Error('--env requires NAME=VALUE');
      out.env[value.slice(0, at)] = value.slice(at + 1);
    } else if (flag === '--state-dir') out.env.CONTROLLA_STATE_DIR = value;
    else if (flag === '--timeout-ms') out.timeout = Number(value);
    else throw new Error(`unknown option: ${flag}`);
  }
  if (!out.server) {
    out.server = path.join(__dirname, `controlla-core${process.platform === 'win32' ? '.exe' : ''}`);
    out.serverArgs = ['mcp', ...out.serverArgs];
  }
  if (!Number.isInteger(out.timeout) || out.timeout < 100 || out.timeout > 600_000) throw new Error('--timeout-ms must be 100..600000');
  // Preserve server ownership policy: concurrent clients must choose a state directory explicitly.
  return out;
}

function validate(schema, value, root, at = '$') {
  if (!schema || typeof schema !== 'object') return null;
  if (schema.$ref) {
    const resolved = schema.$ref.split('/').slice(1).reduce((v, key) => v?.[key.replace(/~1/g, '/').replace(/~0/g, '~')], root);
    return resolved ? validate(resolved, value, root, at) : `${at}: unresolved schema reference ${schema.$ref}`;
  }
  if (schema.allOf) for (const part of schema.allOf) { const e = validate(part, value, root, at); if (e) return e; }
  for (const key of ['oneOf', 'anyOf']) if (schema[key]) {
    const matches = schema[key].filter(part => !validate(part, value, root, at));
    if (key === 'oneOf' ? matches.length !== 1 : matches.length === 0) return `${at}: does not match ${key}`;
  }
  if (schema.enum && !schema.enum.some(v => JSON.stringify(v) === JSON.stringify(value))) return `${at}: must be one of ${schema.enum.join(', ')}`;
  if ('const' in schema && schema.const !== value) return `${at}: must equal ${JSON.stringify(schema.const)}`;
  const type = Array.isArray(schema.type) ? schema.type : [schema.type];
  const ok = t => ({
    object: value !== null && typeof value === 'object' && !Array.isArray(value),
    array: Array.isArray(value), string: typeof value === 'string',
    number: typeof value === 'number' && Number.isFinite(value), integer: Number.isInteger(value),
    boolean: typeof value === 'boolean', null: value === null,
  })[t] ?? true;
  if (schema.type && !type.some(ok)) return `${at}: expected ${type.join('|')}`;
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const key of schema.required ?? []) if (!(key in value)) return `${at}.${key}: required property is missing`;
    for (const [key, item] of Object.entries(value)) {
      if (schema.properties?.[key]) { const e = validate(schema.properties[key], item, root, `${at}.${key}`); if (e) return e; }
      else if (schema.additionalProperties === false || (schema.properties && schema.additionalProperties === undefined)) return `${at}.${key}: unknown property`;
      else if (schema.additionalProperties && typeof schema.additionalProperties === 'object') { const e = validate(schema.additionalProperties, item, root, `${at}.${key}`); if (e) return e; }
    }
  }
  if (Array.isArray(value) && schema.items) for (let i = 0; i < value.length; i++) { const e = validate(schema.items, value[i], root, `${at}[${i}]`); if (e) return e; }
  if (typeof value === 'string' && schema.minLength != null && value.length < schema.minLength) return `${at}: string is too short`;
  if (typeof value === 'string' && schema.maxLength != null && value.length > schema.maxLength) return `${at}: string is too long`;
  if (typeof value === 'number' && schema.minimum != null && value < schema.minimum) return `${at}: must be >= ${schema.minimum}`;
  if (typeof value === 'number' && schema.maximum != null && value > schema.maximum) return `${at}: must be <= ${schema.maximum}`;
  if (Array.isArray(value) && schema.minItems != null && value.length < schema.minItems) return `${at}: array is too short`;
  if (Array.isArray(value) && schema.maxItems != null && value.length > schema.maxItems) return `${at}: array is too long`;
  return null;
}

async function main() {
  let options;
  try { options = parseArgs(process.argv.slice(2)); }
  catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 2; return; }
  const child = spawn(options.server, options.serverArgs, { stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, ...options.env } });
  // Capture requests immediately, including piped input arriving during initialization.
  const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  const queued = [];
  let receive = line => queued.push(line), inputEnded = false;
  input.on('line', line => receive(line));
  input.on('close', () => { inputEnded = true; });
  child.stderr.pipe(process.stderr);
  const lines = readline.createInterface({ input: child.stdout });
  let nextId = 1, closed = false;
  let stopping = false, killTimer;
  const stopChild = () => {
    if (stopping || closed) return;
    stopping = true;
    child.stdin.end();
    killTimer = setTimeout(() => {
      child.kill('SIGTERM');
      killTimer = setTimeout(() => child.kill('SIGKILL'), 1500);
    }, 1500);
  };
  for (const signal of ['SIGINT','SIGTERM']) process.once(signal, () => { input.close(); process.stdin.pause(); stopChild(); });
  const pending = new Map();
  child.stdin.on('error', () => {}); // write callbacks and child close settle pending requests.
  function rpc(method, params, timeout = options.timeout) {
    if (closed || !child.stdin.writable) return Promise.reject(new Error('MCP server process is closed'));
    const id = nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP ${method} timed out after ${timeout} ms${method === 'tools/call' ? '; outcome unknown: inspect state before retrying' : ''}`)); }, timeout);
      pending.set(id, { resolve, reject, timer });
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, ...(params === undefined ? {} : { params }) })}\n`, error => {
        if (error) { clearTimeout(timer); pending.delete(id); reject(error); }
      });
    });
  }
  lines.on('line', line => {
    let message;
    try { message = JSON.parse(line); } catch { process.stderr.write(`Ignored malformed MCP server output: ${line.slice(0, 200)}\n`); return; }
    if (message.method) {
      if (message.id !== undefined) child.stdin.write(`${JSON.stringify({jsonrpc:'2.0',id:message.id,error:{code:-32601,message:'Client does not support server requests'}})}\n`);
      else if (message.method === 'notifications/tools/list_changed') schemasStale = true;
      return;
    }
    const item = pending.get(message.id);
    if (!item) return;
    pending.delete(message.id); clearTimeout(item.timer);
    if (message.error) item.reject(Object.assign(new Error(message.error.message ?? 'MCP server error'), { code: message.error.code, data: message.error.data }));
    else item.resolve(message.result);
  });
  child.on('error', error => { process.stderr.write(`Could not start MCP server: ${error.message}\n`); process.exitCode = 1; });
  child.on('close', (code, signal) => {
    closed = true;
    clearTimeout(killTimer);
    if (!stopping && code !== 0) process.exitCode = 1;
    input.close(); process.stdin.pause();
    for (const { reject, timer } of pending.values()) { clearTimeout(timer); reject(new Error(`MCP server exited (${signal ?? code})`)); }
    pending.clear();
  });
  let tools;
  let schemasStale = false;
  try {
    const initialized = await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'chrome-controlla-client', version: '0.1.0' } });
    if (!initialized?.protocolVersion || !initialized?.serverInfo) throw new Error('invalid initialize result');
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
    tools = [];
    let cursor;
    do {
      const page = await rpc('tools/list', cursor ? {cursor} : {});
      tools.push(...(page.tools ?? [])); cursor = page.nextCursor;
    } while (cursor);
  } catch (error) {
    process.stderr.write(`Chrome Controlla client initialization failed: ${error.message}\n`);
    input.close(); process.stdin.pause(); stopChild(); process.exitCode = 1; return;
  }
  const schemas = new Map(tools.map(tool => [tool.name, tool.inputSchema]));
  let chain = Promise.resolve();
  receive = line => { chain = chain.then(async () => {
    let request;
    try { request = JSON.parse(line); } catch { process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'invalid JSON line' } })}\n`); return; }
    if (!request || typeof request !== 'object' || request.jsonrpc !== '2.0' || !('id' in request) || typeof request.method !== 'string') {
      process.stdout.write(`${JSON.stringify({jsonrpc:'2.0',id:request?.id??null,error:{code:-32600,message:'Expected a JSON-RPC request with id and method'}})}\n`); return;
    }
    const respond = (result, error) => process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: request.id, ...(error ? { error } : { result }) })}\n`);
    if (schemasStale) { respond(null, {code:-32000,message:'Tool schemas changed; restart this client before further calls'}); return; }
    if (request.method === 'tools/list') {
      const name = request.params?.name;
      respond({ tools: name ? tools.filter(t => t.name === name) : tools.map(t => ({name:t.name,description:t.description})) }, null); return;
    }
    if (request.method !== 'tools/call') { respond(null, { code: -32601, message: `unsupported client method: ${request.method}` }); return; }
    const { name, arguments: args } = request.params ?? {};
    const schema = schemas.get(name);
    if (!schema) { respond(null, { code: -32602, message: `unknown tool: ${String(name)}` }); return; }
    const issue = validate(schema, args ?? {}, schema);
    if (issue) { respond(null, { code: -32602, message: `invalid arguments: ${issue}` }); return; }
    try {
      const result = await rpc('tools/call', { name, arguments: args ?? {} });
      if (result.structuredContent !== undefined) delete result.content;
      respond(result, null);
    }
    catch (error) { respond(null, { code: error.code ?? -32000, message: error.message, ...(error.data === undefined ? {} : { data: error.data }) }); }
  }); };
  for (const line of queued) receive(line);
  process.stderr.write('Controlla client ready; schemas cached. Keep this process open for the task.\n');
  const finish = async () => {
    await chain;
    stopChild();
  };
  input.on('close', finish);
  if (inputEnded) await finish();
}

main().catch(error => { process.stderr.write(`${error.stack ?? error}\n`); process.exitCode = 1; });
