'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const readline = require('node:readline');
const test = require('node:test');
const browserSkill = fs.readFileSync(path.join(__dirname, 'plugin/skills/browser-control/SKILL.md'), 'utf8');
const advancedBrowserSkill = fs.readFileSync(path.join(__dirname, 'plugin/skills/browser-control/references/advanced.md'), 'utf8');
const {
  AgentUxRuntime,
  ContextRegistry,
  OpenKeyCache,
  RouteMemory,
  SIX_TOOLS,
  compactTools,
  compileNewestWorkflow,
  routeKey
} = require('./plugin/agent-ux-proxy.cjs');
const { startAgentUxProxy } = require('./plugin/mcp-agent-ux-launch.cjs');

function fakeTools() {
  return SIX_TOOLS.map(name => ({
    name,
    description: `legacy ${name}`,
    inputSchema: {
      type: 'object',
      additionalProperties: false,
      properties: {
        action: { type: 'string' },
        url: { type: 'string' },
        mode: { type: 'string' },
        session_id: { type: 'string' },
        chrome_tab_id: { type: 'string' },
        idempotency_key: { type: 'string' },
        goal: { type: 'object' },
        href_hint: { type: 'string' },
        max_items: { type: 'integer' },
        max_text_chars: { type: 'integer' },
        max_scan_nodes: { type: 'integer' },
        steps: { type: 'array', items: { type: 'object' } },
        program: { type: 'array', items: { type: 'object' } },
        selector: { type: 'string' },
        fields: { type: 'object' },
        reference: { type: 'string' },
        expected_value: { type: 'string' },
        value: { type: 'string' },
        outcome: { type: 'object' },
        predicate: { type: 'object' }
      },
      required: name === 'browser' ? ['action', 'idempotency_key'] : ['session_id', 'chrome_tab_id']
    }
  }));
}

test('headless guidance keeps browser lifecycle mode separate from page tools', () => {
  for (const content of [browserSkill, advancedBrowserSkill]) {
    assert.match(content, /action=task.*mode=.*headless/s);
    assert.match(content, /no `session_id` or `chrome_tab_id`|no session or tab IDs/);
    assert.match(content, /launches.*navigates.*verifies.*releases/s);
    assert.match(content, /read-only `Research` task/);
    assert.match(content, /Form drafts.*existing session/);
    assert.match(content, /`action=launch`/);
    assert.match(content, /`target_ids\[0\]` directly.*`action=task`/s);
    assert.match(content, /skip `list` and standalone `snapshot`/);
    assert.match(content, /`release_after_task: true`/);
    assert.match(content, /accepts.*`foreground`.*`background`/s);
    assert.match(content, /do not pass `mode` to `snapshot`|page tools take `session_id` and do not accept `mode`/);
  }
});

function tempDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'controlla-agent-ux-'));
}

function startProxyFixture() {
  const dir = tempDir();
  const serverPath = path.join(dir, 'server.cjs');
  const driverPath = path.join(dir, 'driver.cjs');
  fs.writeFileSync(serverPath, `
    const readline = require('node:readline');
    const rl = readline.createInterface({ input: process.stdin });
    const tools = ${JSON.stringify(fakeTools())};
    let heldWorkflow;
    let activeSnapshots = 0;
    let maxSnapshots = 0;
    rl.on('line', line => {
      const request = JSON.parse(line);
      if (request.method === 'tools/list') {
        process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { tools } }) + '\\n');
        return;
      }
      if (request.method !== 'tools/call') return;
      const { name, arguments: args = {} } = request.params;
      if (name === 'act' && args.invalidate === true) {
        process.stdout.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/tools/list_changed' }) + '\\n');
        process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { structuredContent: { status: 'completed' } } }) + '\\n');
        return;
      }
      if (name === 'browser') {
        const tab = args.url.endsWith('/a') ? '11' : '22';
        process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { structuredContent: { status: 'verified', session_id: 'session-' + tab, tab_id: tab, url: args.url } } }) + '\\n');
        return;
      }
      if (name === 'workflow' && args.cancel === true) {
        heldWorkflow?.();
        process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { structuredContent: { status: 'cancelled', session_id: args.session_id, chrome_tab_id: args.chrome_tab_id } } }) + '\\n');
        return;
      }
      if (name === 'workflow' && args.hold === true) {
        heldWorkflow = () => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { structuredContent: { status: 'cancelled' } } }) + '\\n');
        return;
      }
      if (name === 'snapshot') {
        activeSnapshots++;
        maxSnapshots = Math.max(maxSnapshots, activeSnapshots);
        setTimeout(() => {
          activeSnapshots--;
          process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { structuredContent: { status: 'observed', session_id: args.session_id, chrome_tab_id: args.chrome_tab_id, max_snapshots: maxSnapshots } } }) + '\\n');
        }, 60);
        return;
      }
      process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: { structuredContent: { status: 'ok' } } }) + '\\n');
    });
  `);
  fs.writeFileSync(driverPath, `
    const { spawn } = require('node:child_process');
    const { startAgentUxProxy } = require(${JSON.stringify(path.resolve(__dirname, 'plugin/mcp-agent-ux-launch.cjs'))});
    const child = spawn(process.execPath, [${JSON.stringify(serverPath)}], { stdio: ['pipe', 'pipe', 'inherit'] });
    startAgentUxProxy(child, { stateDir: ${JSON.stringify(dir)} });
  `);
  const driver = spawn(process.execPath, [driverPath], { stdio: ['pipe', 'pipe', 'pipe'] });
  const lines = readline.createInterface({ input: driver.stdout });
  const waiters = new Map();
  lines.on('line', line => {
    const message = JSON.parse(line);
    const waiter = waiters.get(message.id);
    if (!waiter) return;
    waiters.delete(message.id);
    waiter(message);
  });
  let nextId = 1;
  return {
    dir,
    async request(method, params) {
      const id = nextId++;
      let timeout;
      const response = new Promise((resolve, reject) => {
        waiters.set(id, message => { clearTimeout(timeout); resolve(message); });
        timeout = setTimeout(() => {
          waiters.delete(id);
          reject(new Error(`proxy request ${id} timed out`));
        }, 1500);
      });
      driver.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
      return response;
    },
    close() {
      driver.stdin.end();
      driver.kill();
      lines.close();
      fs.rmSync(dir, { recursive: true, force: true });
    }
  };
}

test('proxy dispatches cancellation and distinct contexts while a child workflow is pending', async t => {
  const proxy = startProxyFixture();
  t.after(() => proxy.close());
  const listed = await proxy.request('tools/list', {});
  assert.equal(listed.error, undefined);
  const openA = await proxy.request('tools/call', { name: 'browser', arguments: { action: 'open', url: 'https://example.test/a' } });
  const openB = await proxy.request('tools/call', { name: 'browser', arguments: { action: 'open', url: 'https://example.test/b' } });
  const contextA = openA.result.structuredContent.context;
  const contextB = openB.result.structuredContent.context;

  const held = proxy.request('tools/call', { name: 'workflow', arguments: { context: contextA, advanced: { hold: true } } });
  await new Promise(resolve => setTimeout(resolve, 30));
  const cancel = await proxy.request('tools/call', { name: 'workflow', arguments: { context: contextA, advanced: { cancel: true } } });
  assert.equal(cancel.result.structuredContent.status, 'cancelled', 'cancel must reach child before held workflow completes');
  assert.equal((await held).result.structuredContent.status, 'cancelled');

  const [snapshotA, snapshotB] = await Promise.all([
    proxy.request('tools/call', { name: 'snapshot', arguments: { context: contextA } }),
    proxy.request('tools/call', { name: 'snapshot', arguments: { context: contextB } })
  ]);
  assert.equal(snapshotA.result.structuredContent.session_id, 'session-11');
  assert.equal(snapshotA.result.structuredContent.chrome_tab_id, '11');
  assert.equal(snapshotB.result.structuredContent.session_id, 'session-22');
  assert.equal(snapshotB.result.structuredContent.chrome_tab_id, '22');
  assert.equal(snapshotA.result.structuredContent.max_snapshots, 2, 'independent child calls should overlap');
});

test('proxy fails closed when the child schema changes during a pending mutation', async t => {
  const proxy = startProxyFixture();
  t.after(() => proxy.close());
  await proxy.request('tools/list', {});
  const result = await proxy.request('tools/call', { name: 'act', arguments: { advanced: {
    session_id: 'schema-test-session', chrome_tab_id: 'schema-test-tab', invalidate: true
  } } });
  assert.equal(result.error.code, -32000);
  assert.match(result.error.message, /outcome is unknown/);
  assert.equal((await proxy.request('tools/list', {})).error, undefined, 'the next request should use refreshed schemas');
});

test('public plugin surface keeps six eager tools but hides routine bookkeeping', () => {
  const tools = compactTools(fakeTools());
  assert.deepEqual(tools.map(tool => tool.name), SIX_TOOLS);
  const browser = tools.find(tool => tool.name === 'browser');
  assert.match(browser.description, /PRIMARY ENTRY POINT/);
  assert.ok(browser.inputSchema.properties.goal);
  assert.ok(browser.inputSchema.properties.context);
  assert.ok(browser.inputSchema.properties.advanced);
  assert.equal(browser.inputSchema.properties.session_id, undefined);
  assert.equal(browser.inputSchema.properties.chrome_tab_id, undefined);
  assert.equal(browser.inputSchema.properties.idempotency_key, undefined);
  assert.deepEqual(browser.inputSchema.required, ['action']);
  for (const tool of tools.filter(tool => tool.name !== 'browser')) {
    assert.ok(tool.inputSchema.properties.context, `${tool.name} should accept context`);
    assert.equal(tool.inputSchema.properties.session_id, undefined);
    assert.equal(tool.inputSchema.properties.chrome_tab_id, undefined);
    assert.ok(!tool.inputSchema.required.includes('session_id'));
    assert.ok(!tool.inputSchema.required.includes('chrome_tab_id'));
  }
});

test('automatic open keys are retry-stable briefly but do not pin future opens forever', () => {
  let now = 1_000;
  const cache = new OpenKeyCache(() => now);
  const args = { action: 'open', mode: 'foreground', url: 'https://example.test/page' };
  const first = cache.keyFor(args);
  assert.equal(cache.keyFor(args), first);
  now += 60_001;
  assert.notEqual(cache.keyFor(args), first);
});

test('one opaque context replaces repeated session and tab bookkeeping', () => {
  const contexts = new ContextRegistry();
  const context = contexts.create('session-1', '42');
  assert.match(context, /^ctx_/);
  assert.deepEqual(contexts.resolve(context), { session_id: 'session-1', chrome_tab_id: '42' });
  assert.deepEqual(contexts.resolve(), { session_id: 'session-1', chrome_tab_id: '42' });
  contexts.create('session-2', '43');
  assert.equal(contexts.resolve(), null, 'implicit context must fail closed when ambiguous');
});

test('newest-item compiler emits one targeted snapshot and no homepage detour', () => {
  const goal = { kind: 'open_newest_item', site: 'espn.com', entity: 'Buffalo Bills', item_type: 'story' };
  const profile = {
    destination_url: 'https://www.espn.com/nfl/team/_/name/buf/buffalo-bills',
    link_query: 'Bills',
    href_hint: '/nfl/story/',
    verify_url_prefix: 'https://www.espn.com/nfl/'
  };
  const steps = compileNewestWorkflow(goal, profile);
  assert.equal(steps.filter(step => step.kind === 'snapshot').length, 1);
  assert.deepEqual(steps.map(step => step.kind), ['snapshot', 'find_all', 'choose', 'click', 'verify']);
  assert.equal(steps[0].goal.exhaustive, true);
  assert.equal(steps[0].goal.href_hint, '/nfl/story/');
  assert.equal(steps[1].query, undefined, 'choose(datetime) must compare every story in the scoped collection, including headlines that omit the team name');
  assert.equal(steps[1].href_hint, '/nfl/story/');
  assert.equal(steps[0].max_text_chars, 0);
  assert.equal(steps[3].reference, '$latest');
});

test('qualified route memory requires verification evidence and quarantines structural drift', () => {
  const dir = tempDir();
  const memory = new RouteMemory(dir, () => 100_000);
  const goal = {
    kind: 'open_newest_item',
    site: 'example.test',
    entity: 'Example Team',
    item_type: 'story',
    destination_url: 'https://example.test/team/example'
  };
  const profile = {
    site_scope: 'https://example.test',
    destination_url: goal.destination_url,
    link_query: 'Example Team',
    href_hint: '/story/',
    verify_url_prefix: 'https://example.test/story/',
    structural_signatures: ['Example Team'],
    source: 'explicit_destination'
  };
  memory.recordSuccess(goal, profile, 'run-a');
  assert.equal(memory.resolve({ ...goal, destination_url: undefined }), null, 'one run is only a candidate');
  memory.recordSuccess(goal, profile, 'run-b');
  assert.equal(memory.resolve({ ...goal, destination_url: undefined }).destination_url, goal.destination_url);
  memory.recordFailure(goal, { structuralDrift: true });
  assert.equal(memory.resolve({ ...goal, destination_url: undefined }), null);
  fs.rmSync(dir, { recursive: true, force: true });
});

test('ESPN Bills goal compiles to one browser call with an inline verified workflow', async () => {
  const calls = [];
  const dir = tempDir();
  const runtime = new AgentUxRuntime({
    stateDir: dir,
    callTool: async (name, args) => {
      calls.push({ name, args: structuredClone(args) });
      if (name === 'browser') {
        return {
          structuredContent: {
            status: 'verified', session_id: 's1', tab_id: '77', url: args.url,
            status: 'completed',
            workflow: { status: 'completed', receipts: [
              { step: 'snapshot', status: 'collected' },
              { step: 'find_all', status: 'collected' },
              { step: 'choose', status: 'selected' },
              { step: 'click', status: 'verified' },
              { step: 'verify', status: 'passed' }
            ] }
          }
        };
      }
      throw new Error(`unexpected tool ${name}`);
    }
  });
  runtime.setServerTools(fakeTools());
  const result = await runtime.handleToolCall('browser', {
    action: 'open',
    goal: { kind: 'open_newest_item', site: 'espn.com', entity: 'Buffalo Bills', item_type: 'story' }
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].name, 'browser');
  assert.equal(calls[0].args.url, 'https://www.espn.com/nfl/team/_/name/buf/buffalo-bills');
  assert.deepEqual(calls[0].args.ready_when, { selector: 'a[href*="/nfl/story/"]' },
    'the direct newest-story route should start after its first story link appears, not wait for the full document');
  assert.notEqual(calls[0].args.url, 'https://www.espn.com/');
  assert.match(calls[0].args.idempotency_key, /^auto_/);
  assert.deepEqual(calls[0].args.steps.map(step => step.kind), ['snapshot', 'find_all', 'choose', 'click', 'verify']);
  assert.equal(calls[0].args.steps.filter(step => step.kind === 'snapshot').length, 1);
  assert.equal(result.structuredContent.status, 'verified');
  assert.match(result.structuredContent.context, /^ctx_/);
  assert.deepEqual(result.structuredContent.orchestration, {
    external_tool_calls: 1,
    internal_tool_calls: 1,
    schema_discovery_calls: 0,
    repo_reads_required: 0,
    targeted_snapshots: 1,
    homepage_detours: 0,
    direct_route: true
  });
  assert.equal(routeKey(result.structuredContent.goal), 'espn.com|open_newest_item|buffalo bills|story');
  fs.rmSync(dir, { recursive: true, force: true });
});

test('background newest-story goals navigate to the selected href without activating pointer input', async () => {
  const calls = [];
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async (name, args) => {
    calls.push({ name, args: structuredClone(args) });
    return { structuredContent: { status: 'completed', session_id: 's1', tab_id: '77',
      workflow: { status: 'completed', receipts: [{ step: 'verify', status: 'passed' }] } } };
  }});
  runtime.setServerTools(fakeTools());
  await runtime.handleToolCall('browser', { action: 'open', mode: 'background', goal: {
    kind: 'open_newest_item', site: 'espn.com', entity: 'Buffalo Bills', item_type: 'story'
  }});
  assert.equal(calls.length, 1);
  assert.equal(calls[0].args.mode, 'background');
  assert.deepEqual(calls[0].args.ready_when, { selector: 'a[href*="/nfl/story/"]' });
  assert.deepEqual(calls[0].args.steps.map(step => step.kind), ['snapshot', 'find_all', 'choose', 'navigate_selected', 'verify']);
});

test('runtime instructions forbid schema probing for ordinary MCP browser tasks', () => {
  const runtimeSource = fs.readFileSync(path.resolve(__dirname, '../../crates/controlla-runtime/src/mcp/v3_full.rs'), 'utf8');
  const instructions = runtimeSource.match(/\.with_instructions\(r#"([\s\S]*?)"#\)/)?.[1] ?? '';
  assert.match(instructions, /(MCP already supplies the six tool schemas|schemas are already supplied)/i);
  assert.match(instructions, /Do not call action=schema/i);
  assert.match(instructions, /browser action=open with an exact URL|call browser action=open directly/i);
  assert.match(instructions, /stale_watcher_revision/i);
  assert.match(instructions, /no_safe_tab_match/i);
  assert.match(instructions, /presentation_warning=chrome_window_not_focused/i);
  assert.match(instructions, /unknown means dispatch may have happened/i);
  assert.doesNotMatch(instructions, /Read these tool schemas once per session/i);
  assert.doesNotMatch(instructions, /"kind":"find"[^}]*"exhaustive"/i,
    'canonical find recipe must contain only fields accepted by the live workflow schema');
  assert.match(instructions, /no safe existing tab matches.*no_safe_tab_match/i);
});

test('packaged launcher retries only startup failures before proxy dispatch', () => {
  const launchSource = fs.readFileSync(path.resolve(__dirname, 'plugin/launch.cjs'), 'utf8');
  assert.match(launchSource, /maxAttempts = 3/);
  assert.match(launchSource, /setTimeout\(resolve, 100 \* attempt\)/);
  assert.match(launchSource, /startAgentUxProxy\(child/);
});

test('follow-up tools can use only the opaque context handle', async () => {
  const calls = [];
  const runtime = new AgentUxRuntime({
    stateDir: tempDir(),
    callTool: async (name, args) => {
      calls.push({ name, args: structuredClone(args) });
      if (name === 'browser') return { structuredContent: { status: 'verified', session_id: 's1', tab_id: '8' } };
      return { structuredContent: { status: 'ok' } };
    }
  });
  runtime.setServerTools(fakeTools());
  const opened = await runtime.handleToolCall('browser', { action: 'open', url: 'https://example.test/' });
  const context = opened.structuredContent.context;
  await runtime.handleToolCall('snapshot', { context, max_items: 1, max_text_chars: 0 });
  assert.equal(calls[1].args.session_id, 's1');
  assert.equal(calls[1].args.chrome_tab_id, '8');
  assert.equal(calls[1].args.context, undefined);
});

test('context-bound tools infer the sole active context when context is omitted', async () => {
  const calls = [];
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async (name, args) => {
    calls.push({ name, args: structuredClone(args) });
    if (name === 'browser') return { structuredContent: { status: 'verified', session_id: 's1', tab_id: '8' } };
    return { structuredContent: { status: 'ok' } };
  }});
  runtime.setServerTools(fakeTools());
  await runtime.handleToolCall('browser', { action: 'open', url: 'https://example.test/' });
  await runtime.handleToolCall('snapshot', { max_items: 1 });
  assert.equal(calls[1].args.session_id, 's1');
  assert.equal(calls[1].args.chrome_tab_id, '8');
});

test('context-bound tools reject omitted context when multiple contexts are active', async () => {
  const calls = [];
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async (name, args) => {
    calls.push({ name, args: structuredClone(args) });
    return { structuredContent: { status: 'verified', session_id: `s${calls.length}`, tab_id: String(calls.length) } };
  }});
  runtime.setServerTools(fakeTools());
  await runtime.handleToolCall('browser', { action: 'open', url: 'https://example.test/a' });
  await runtime.handleToolCall('browser', { action: 'open', url: 'https://example.test/b' });
  await assert.rejects(runtime.handleToolCall('snapshot', {}), /context is required when multiple/);
  assert.equal(calls.length, 2, 'ambiguous call must not reach child');
});

test('context-bound tools reject omitted context when no context or explicit target IDs exist', async () => {
  let calls = 0;
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async () => { calls++; return {}; } });
  runtime.setServerTools(fakeTools());
  await assert.rejects(runtime.handleToolCall('snapshot', {}), /context or explicit session_id and chrome_tab_id are required/);
  assert.equal(calls, 0, 'unbound call must not reach child');
});

test('context-bound tools allow a complete explicit target pair with no registered context', async () => {
  let dispatched;
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async (_name, args) => { dispatched = args; return {}; } });
  runtime.setServerTools(fakeTools());
  await runtime.handleToolCall('snapshot', { advanced: { session_id: 's-explicit', chrome_tab_id: 'tab-explicit' } });
  assert.equal(dispatched.session_id, 's-explicit');
  assert.equal(dispatched.chrome_tab_id, 'tab-explicit');
});

test('explicit context rejects conflicting direct and advanced target IDs', async () => {
  const calls = [];
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async (name, args) => {
    calls.push({ name, args: structuredClone(args) });
    if (name === 'browser') return { structuredContent: { status: 'verified', session_id: 's1', tab_id: '8' } };
    return { structuredContent: { status: 'ok' } };
  }});
  runtime.setServerTools(fakeTools());
  const opened = await runtime.handleToolCall('browser', { action: 'open', url: 'https://example.test/' });
  const context = opened.structuredContent.context;
  await assert.rejects(runtime.handleToolCall('snapshot', { context, session_id: 'other-session' }), /conflicts with context/);
  await assert.rejects(runtime.handleToolCall('snapshot', { context, advanced: { chrome_tab_id: '99' } }), /conflicts with context/);
  assert.equal(calls.length, 1, 'conflicting IDs must not reach child');
});

test('an unknown explicit context fails closed without dispatching an unbound call', async () => {
  let calls = 0;
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async () => { calls++; return {}; } });
  runtime.setServerTools(fakeTools());
  await assert.rejects(runtime.handleToolCall('snapshot', { context: 'ctx_expired' }), /unknown Controlla context/);
  assert.equal(calls, 0);
});

test('structured failed snapshots trigger context recovery instead of false success', async () => {
  const calls = [];
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async (name, args) => {
    calls.push({ name, args: structuredClone(args) });
    if (name === 'browser' && args.action === 'open') return { structuredContent: { status: 'verified', session_id: 's1', tab_id: '8', url: 'https://example.test/' } };
    if (name === 'snapshot') return { structuredContent: { status: 'failed', reason_code: 'target_lifecycle_changed' } };
    if (name === 'browser' && args.action === 'discover_shared_tabs') return { structuredContent: { snapshot: { host_id: 'h1', tabs: [{ id: 8, url: 'https://example.test/' }] } } };
    if (name === 'browser' && args.action === 'pair_shared') return { structuredContent: { session_id: 's2' } };
    if (name === 'browser' && args.action === 'accept_shared') return { structuredContent: { accepted: true } };
    throw new Error(`unexpected tool ${name}`);
  }});
  runtime.setServerTools(fakeTools());
  const opened = await runtime.handleToolCall('browser', { action: 'open', url: 'https://example.test/' });
  const recovered = await runtime.handleToolCall('browser', { action: 'open', url: 'https://example.test/', context: opened.structuredContent.context });
  assert.equal(recovered.structuredContent.status, 'reused');
  assert.ok(calls.some(call => call.name === 'browser' && call.args.action === 'discover_shared_tabs'));
});

test('open reuses the bound ESPN tab after a focus or session transition', async () => {
  const calls = [];
  let stale = false;
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async (name, args) => {
    calls.push({ name, args: structuredClone(args) });
    if (name === 'browser' && args.action === 'open') {
      return { structuredContent: { status: 'verified', session_id: 's1', tab_id: '77', url: args.url, mode: args.mode ?? 'foreground' } };
    }
    if (name === 'snapshot') {
      if (stale) throw new Error('session was not created through the V3 browser route');
      return { structuredContent: { status: 'observed', url: 'https://www.espn.com/' } };
    }
    if (name === 'browser' && args.action === 'discover_shared_tabs') {
      return { structuredContent: { snapshot: { host_id: 'h1', tabs: [{ id: 88, url: 'https://www.espn.com/', window_id: 4 }] } } };
    }
    if (name === 'browser' && args.action === 'pair_shared') {
      return { structuredContent: { session_id: 's2' } };
    }
    if (name === 'browser' && args.action === 'accept_shared') {
      return { structuredContent: { accepted: true } };
    }
    throw new Error(`unexpected tool ${name}`);
  }});
  runtime.setServerTools(fakeTools());
  const opened = await runtime.handleToolCall('browser', { action: 'open', url: 'https://www.espn.com/' });
  const context = opened.structuredContent.context;
  stale = true;
  const reused = await runtime.handleToolCall('browser', { action: 'open', url: 'https://www.espn.com/', context });
  assert.equal(reused.structuredContent.status, 'reused');
  assert.equal(reused.structuredContent.same_target, true);
  assert.equal(reused.structuredContent.tab_id, '88');
  assert.deepEqual(calls.map(call => `${call.name}:${call.args.action ?? ''}`), [
    'browser:open',
    'snapshot:',
    'browser:discover_shared_tabs',
    'browser:pair_shared',
    'browser:accept_shared'
  ]);
});

test('reuse_existing false preserves explicit new-tab behavior', async () => {
  const calls = [];
  const runtime = new AgentUxRuntime({ stateDir: tempDir(), callTool: async (name, args) => {
    calls.push({ name, args: structuredClone(args) });
    if (name === 'browser') return { structuredContent: { status: 'verified', session_id: `s${calls.length}`, tab_id: String(70 + calls.length), url: args.url, mode: args.mode ?? 'foreground' } };
    if (name === 'snapshot') return { structuredContent: { status: 'observed', url: 'https://www.espn.com/' } };
    throw new Error(`unexpected tool ${name}`);
  }});
  runtime.setServerTools(fakeTools());
  await runtime.handleToolCall('browser', { action: 'open', url: 'https://www.espn.com/' });
  await runtime.handleToolCall('browser', { action: 'open', url: 'https://www.espn.com/', reuse_existing: false });
  assert.equal(calls.filter(call => call.name === 'browser').length, 2);
  assert.equal(calls[1].args.action, 'open');
});
