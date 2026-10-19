'use strict';

const readline = require('node:readline');
const { AgentUxRuntime, SIX_TOOLS } = require('./agent-ux-proxy.cjs');

function startAgentUxProxy(child, options = {}) {
  let activeChild = child;
  let serverLines;
  const clientLines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  let internalId = 1;
  let schemaStale = true;
  let schemaRevision = 0;
  let refreshPromise = null;
  let serverTools = [];
  let closed = false;
  let closingPromise = null;
  let restarting = false;
  let restartCount = 0;
  const pending = new Map();
  const inFlight = new Set();

  const sendServer = message => {
    if (closed || !activeChild?.stdin.writable) throw new Error('Controlla MCP process is closed');
    activeChild.stdin.write(`${JSON.stringify(message)}\n`);
  };
  const sendClient = message => process.stdout.write(`${JSON.stringify(message)}\n`);

  function rpc(method, params, timeoutMs = 60_000) {
    const id = `__controlla_proxy_${internalId++}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(Object.assign(new Error(`${method} timed out after ${timeoutMs} ms`), { code: -32000 }));
      }, timeoutMs);
      pending.set(id, { resolve, reject, timer });
      sendServer({ jsonrpc: '2.0', id, method, ...(params === undefined ? {} : { params }) });
    });
  }

  async function callServerTool(name, args) {
    async function callAtCurrentRevision() {
      const revision = schemaRevision;
      const result = await rpc('tools/call', { name, arguments: args });
      if (revision !== schemaRevision) {
        throw Object.assign(new Error('Controlla schema changed while tool call was pending; outcome is unknown'), { code: -32000 });
      }
      return result;
    }
    try {
      return await callAtCurrentRevision();
    } catch (error) {
      if (error?.code !== -32602) throw error;
      schemaStale = true;
      await refreshTools();
      return callAtCurrentRevision();
    }
  }

  const runtime = new AgentUxRuntime({
    callTool: callServerTool,
    stateDir: options.stateDir
  });

  function refreshTools() {
    if (refreshPromise) return refreshPromise;
    refreshPromise = (async () => {
      while (true) {
        const revision = schemaRevision;
        const tools = [];
        let cursor;
        do {
          const page = await rpc('tools/list', cursor ? { cursor } : {});
          tools.push(...(page?.tools ?? []));
          cursor = page?.nextCursor;
        } while (cursor);
        if (revision !== schemaRevision) continue;
        const names = new Set(tools.map(tool => tool.name));
        const missing = SIX_TOOLS.filter(name => !names.has(name));
        if (missing.length) throw new Error(`Controlla server is missing required V3 tools: ${missing.join(', ')}`);
        serverTools = tools;
        runtime.setServerTools(tools);
        schemaStale = false;
        return;
      }
    })().finally(() => { refreshPromise = null; });
    return refreshPromise;
  }

  async function ensureTools() {
    if (schemaStale || !serverTools.length) await refreshTools();
  }

  const handleServerLine = line => {
    let message;
    try { message = JSON.parse(line); }
    catch {
      process.stderr.write(`Ignored malformed Controlla MCP output: ${line.slice(0, 200)}\n`);
      return;
    }
    if (message.id != null && pending.has(message.id)) {
      const item = pending.get(message.id);
      pending.delete(message.id);
      clearTimeout(item.timer);
      if (message.error) item.reject(Object.assign(new Error(message.error.message ?? 'Controlla MCP error'), { code: message.error.code, data: message.error.data }));
      else item.resolve(message.result);
      return;
    }
    if (message.method === 'notifications/tools/list_changed') {
      schemaStale = true;
      schemaRevision++;
      return;
    }
    sendClient(message);
  };

  function rejectPending(reason) {
    for (const item of pending.values()) {
      clearTimeout(item.timer);
      item.reject(new Error(reason));
    }
    pending.clear();
  }

  function attachServer(nextChild) {
    activeChild = nextChild;
    serverLines = readline.createInterface({ input: nextChild.stdout, crlfDelay: Infinity });
    serverLines.on('line', handleServerLine);
    nextChild.on('exit', handleChildExit);
  }

  async function handleChildExit(code, signal) {
    if (closed) return;
    schemaRevision++;
    schemaStale = true;
    rejectPending(`Controlla MCP worker exited (${signal ?? code}); outcome is unknown and was not replayed`);
    if (typeof options.restart === 'function' && !restarting && restartCount < 2) {
      restarting = true;
      restartCount += 1;
      try {
        const replacement = await options.restart();
        if (closed) {
          replacement.kill?.('SIGTERM');
          return;
        }
        attachServer(replacement);
        process.stderr.write(`Chrome Controlla worker reloaded (generation ${restartCount}); pending mutations were not replayed.\n`);
      } catch (error) {
        process.stderr.write(`Chrome Controlla worker reload failed: ${error.message}\n`);
        closed = true;
        clientLines.close();
        process.stdin.pause();
      } finally {
        restarting = false;
      }
      return;
    }
    closed = true;
    clientLines.close();
    serverLines?.close();
    process.stdin.pause();
  }

  clientLines.on('line', line => {
    const task = (async () => {
      let request;
      try { request = JSON.parse(line); }
      catch {
        sendClient({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'invalid JSON' } });
        return;
      }
      if (request.method === 'tools/list' && request.id != null) {
        try {
          await ensureTools();
          sendClient({ jsonrpc: '2.0', id: request.id, result: { tools: runtime.publicTools() } });
        } catch (error) {
          sendClient({ jsonrpc: '2.0', id: request.id, error: { code: -32000, message: error.message } });
        }
        return;
      }
      if (request.method === 'tools/call' && request.id != null) {
        try {
          await ensureTools();
          const name = request.params?.name;
          if (!SIX_TOOLS.includes(name)) {
            sendClient({ jsonrpc: '2.0', id: request.id, error: { code: -32602, message: `unknown Controlla tool: ${String(name)}` } });
            return;
          }
          const result = await runtime.handleToolCall(name, request.params?.arguments ?? {});
          sendClient({ jsonrpc: '2.0', id: request.id, result });
        } catch (error) {
          sendClient({ jsonrpc: '2.0', id: request.id, error: { code: Number.isInteger(error?.code) ? error.code : -32000, message: error?.message ?? String(error), ...(error?.data === undefined ? {} : { data: error.data }) } });
        }
        return;
      }
      sendServer(request);
    })().catch(error => process.stderr.write(`Chrome Controlla proxy request failed: ${error.stack ?? error}\n`));
    inFlight.add(task);
    task.finally(() => inFlight.delete(task));
  });

  async function closeProxy() {
    if (closingPromise) return closingPromise;
    closingPromise = (async () => {
      await Promise.allSettled([...inFlight]);
      clientLines.close();
      closed = true;
      serverLines?.close();
      if (activeChild?.stdin.writable) activeChild.stdin.end();
    })();
    return closingPromise;
  }

  clientLines.on('close', () => {
    closeProxy().catch(error => process.stderr.write(`Chrome Controlla proxy close failed: ${error.message}\n`));
  });

  attachServer(child);

  return { runtime, close: closeProxy };
}

module.exports = { startAgentUxProxy };
