'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
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

function tempDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'controlla-agent-ux-'));
}

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

test('ESPN Bills goal compiles to one external call, direct team URL, and one internal workflow', async () => {
  const calls = [];
  const dir = tempDir();
  const runtime = new AgentUxRuntime({
    stateDir: dir,
    callTool: async (name, args) => {
      calls.push({ name, args: structuredClone(args) });
      if (name === 'browser') {
        return { structuredContent: { status: 'verified', session_id: 's1', tab_id: '77', url: args.url } };
      }
      if (name === 'workflow') {
        return {
          structuredContent: {
            status: 'completed',
            receipts: [
              { step: 'snapshot', status: 'collected' },
              { step: 'find_all', status: 'collected' },
              { step: 'choose', status: 'selected' },
              { step: 'click', status: 'verified' },
              { step: 'verify', status: 'passed' }
            ]
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
  assert.equal(calls.length, 2);
  assert.equal(calls[0].name, 'browser');
  assert.equal(calls[0].args.url, 'https://www.espn.com/nfl/team/_/name/buf/buffalo-bills');
  assert.notEqual(calls[0].args.url, 'https://www.espn.com/');
  assert.match(calls[0].args.idempotency_key, /^auto_/);
  assert.equal(calls[1].name, 'workflow');
  assert.deepEqual(calls[1].args.steps.map(step => step.kind), ['snapshot', 'find_all', 'choose', 'click', 'verify']);
  assert.equal(calls[1].args.steps.filter(step => step.kind === 'snapshot').length, 1);
  assert.equal(result.structuredContent.status, 'verified');
  assert.match(result.structuredContent.context, /^ctx_/);
  assert.deepEqual(result.structuredContent.orchestration, {
    external_tool_calls: 1,
    internal_tool_calls: 2,
    schema_discovery_calls: 0,
    repo_reads_required: 0,
    targeted_snapshots: 1,
    homepage_detours: 0,
    direct_route: true
  });
  assert.equal(routeKey(result.structuredContent.goal), 'espn.com|open_newest_item|buffalo bills|story');
  fs.rmSync(dir, { recursive: true, force: true });
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
