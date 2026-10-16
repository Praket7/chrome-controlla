import test from 'node:test';
import assert from 'node:assert/strict';
import { fingerprintToolSchema, validateClientEvidence, validateClientSmoke } from './client-smoke-contract.mjs';
import { resolveClientAdapterConfig, runLiveClientSmokes } from './run-live-client-smokes.mjs';
import { EXPECTED_V3_TOOL_SCHEMA_SHA256, registeredClientMcpAdapters, runClientMcpContract } from './client-adapters.mjs';
import { buildClientHostInvocation, prepareClientHostConfig, registeredClientHostAdapters, resolveClientHostAdapter, runClientHostTask } from './client-host-adapters.mjs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const required = ['startup', 'persistent_session'];
const valid = {
  schema_version: 1,
  run_id: 'run-1',
  client: 'codex',
  status: 'passed',
  live: true,
  evidence_path: '/tmp/codex-smoke.json',
  evidence_sha256: 'a'.repeat(64),
  client_version: 'codex-test',
  controlla_version: 'controlla-test',
  verified_capabilities: required,
};
const startedAt = Date.now();
const evidence = {
  schema_version: 1,
  run_id: 'run-1',
  client: 'codex',
  client_version: 'codex-test',
  controlla_version: 'controlla-test',
  cases: [
    { capability: 'startup', status: 'passed', live: true, observer: 'codex-mcp-client', observed_at: new Date(startedAt + 10).toISOString(), proof: { tools: ['browser', 'snapshot', 'act', 'workflow', 'extract', 'verify'], tool_schema: ['browser', 'snapshot', 'act', 'workflow', 'extract', 'verify'].map(name => ({ name, inputSchema: { type: 'object' } })), schema_sha256: fingerprintToolSchema(['browser', 'snapshot', 'act', 'workflow', 'extract', 'verify'].map(name => ({ name, inputSchema: { type: 'object' } }))), process_id: 1234, isolation: 'disposable' } },
    { capability: 'persistent_session', status: 'passed', live: true, observer: 'codex-mcp-client', observed_at: new Date(startedAt + 20).toISOString(), proof: { session_id: 'test-session', session_reused: true } },
  ],
};

test('client smoke contract accepts live evidence covering every required capability', () => {
  assert.equal(validateClientSmoke('codex', valid, required, 'run-1'), valid);
  assert.throws(() => validateClientSmoke('codex', valid, required, 'run-2'), /current run/);
  assert.throws(() => validateClientSmoke('codex', { ...valid, integration_config_sha256: 'b'.repeat(64) }, required, 'run-1', 'a'.repeat(64)), /resolved integration config/);
});

test('client smoke contract rejects synthetic, mismatched, and incomplete evidence', () => {
  assert.throws(() => validateClientSmoke('codex', { ...valid, live: false }, required, 'run-1'), /live=true/);
  assert.throws(() => validateClientSmoke('claude-code', valid, required, 'run-1'), /matching client/);
  assert.throws(() => validateClientSmoke('codex', { ...valid, verified_capabilities: ['startup'] }, required, 'run-1'), /persistent_session/);
  assert.throws(() => validateClientSmoke('codex', { ...valid, evidence_sha256: 'not-a-digest' }, required, 'run-1'), /SHA-256/);
});

test('client evidence validates version binding, live observations, required coverage, and exact six-tool schema', () => {
  const run = { runId: 'run-1', startedAtMs: startedAt, finishedAtMs: startedAt + 100 };
  assert.deepEqual(validateClientEvidence('codex', valid, evidence, required, run).verified_capabilities, [...required].sort());
  assert.throws(() => validateClientEvidence('codex', valid, evidence, required, run, { expectedConfigHash: 'a'.repeat(64) }), /resolved integration config/);
  assert.throws(() => validateClientEvidence('codex', valid, { ...evidence, synthetic: true }, required, run), /synthetic fixture evidence cannot qualify/);
  assert.throws(() => validateClientEvidence('codex', valid, evidence, required, { ...run, runId: 'run-2' }), /run_id/);
  assert.throws(() => validateClientEvidence('codex', valid, { ...evidence, cases: [{ ...evidence.cases[0], observed_at: new Date(startedAt - 10_000).toISOString() }, evidence.cases[1]] }, required, run), /current run/);
  assert.throws(() => validateClientEvidence('codex', valid, { ...evidence, client_version: 'other' }, required, run), /client_version/);
  assert.throws(() => validateClientEvidence('codex', valid, { ...evidence, cases: evidence.cases.slice(0, 1) }, required, run), /persistent_session/);
  assert.throws(() => validateClientEvidence('codex', valid, { ...evidence, cases: [{ ...evidence.cases[0], live: false }, evidence.cases[1]] }, required, run), /live/);
  assert.throws(() => validateClientEvidence('codex', valid, { ...evidence, cases: [{ ...evidence.cases[0], proof: { ...evidence.cases[0].proof, tools: ['browser'] } }, evidence.cases[1]] }, required, run), /six-tool/);
  const proof = evidence.cases[0].proof;
  assert.throws(() => validateClientEvidence('codex', valid, { ...evidence, cases: [{ ...evidence.cases[0], proof: { ...proof, schema_sha256: 'b'.repeat(64) } }, evidence.cases[1]] }, required, run), /does not match the tool schemas/);
});

test('mutation qualification requires an independent post-action observer', () => {
  const mutationResult = { ...valid, verified_capabilities: [...required, 'verified_mutations'] };
  const mutationCase = {
    capability: 'verified_mutations', status: 'passed', live: true, observer: 'codex-mcp-client', observed_at: new Date(Date.now()).toISOString(),
    proof: { actor_id: 'controlla-action', action_receipt: { status: 'verified' }, independent_verification: { status: 'passed', observer_id: 'fresh-browser-snapshot' } },
  };
  const mutationEvidence = { ...evidence, cases: [...evidence.cases, mutationCase] };
  assert.ok(validateClientEvidence('codex', mutationResult, mutationEvidence, [...required, 'verified_mutations'], { runId: 'run-1', startedAtMs: startedAt, finishedAtMs: Date.now() }));
  const coupled = { ...mutationCase, proof: { ...mutationCase.proof, independent_verification: { status: 'passed', observer_id: 'controlla-action' } } };
  assert.throws(() => validateClientEvidence('codex', mutationResult, { ...evidence, cases: [...evidence.cases, coupled] }, [...required, 'verified_mutations'], { runId: 'run-1', startedAtMs: startedAt, finishedAtMs: Date.now() }), /independent observer/);
});

test('client adapter orchestrator runs each configured client and stores only current hashed evidence', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-client-smoke-'));
  try {
    const matrixPath = join(directory, 'matrix.json');
    const outputPath = join(directory, 'results.json');
    const codexRow = { requiredEvidenceCategories: required, integrationConfig: 'integrations/codex/config.toml.example', adapterKind: 'stdio_mcp' };
    await writeFile(matrixPath, JSON.stringify({ clients: { codex: codexRow } }));
    const adapter = resolve(fileURLToPath(new URL('./test-fixtures/client-smoke-adapter.mjs', import.meta.url)));
    const env = { ...process.env, CONTROLLA_TEST_EVIDENCE_ROOT: directory, CONTROLLA_BINARY_PATH: process.execPath,
      CONTROLLA_CLIENT_SMOKE_STATE_ROOT: directory };
    const context = await resolveClientAdapterConfig('codex', codexRow, { env, stateRoot: directory });
    assert.deepEqual(context.args, ['mcp']);
    assert.equal(context.command, process.execPath);
    assert.ok(isAbsolute(context.command));
    assert.equal(context.integration_config, codexRow.integrationConfig);
    assert.ok(context.config_sha256);
    const result = await runLiveClientSmokes({ matrixPath, outputPath, commands: { codex: [process.execPath, adapter] },
      hostTasks: { codex: { read_only: 'read', verified_mutation: 'mutate fixture', mode_specific: 'open background' } },
      hostTaskRunner: async (client, { task }) => ({ client, status: 'host_tool_invocation_observed', qualification_status: 'not_run', process_id: 321,
        tool_events: (task === 'read_only' ? ['snapshot', 'extract'] : task === 'verified_mutation' ? ['act', 'verify'] : ['browser']).map(name => ({ name: `chrome-controlla.${name}`, type: 'mcp_tool_call', ...(task === 'mode_specific' ? { input: { action: 'open', mode: 'background' } } : {}) })), stdout_sha256: 'a'.repeat(64) }),
      mcpContractRunner: async () => ({ status: 'contract_verified', qualification_status: 'not_run', schema_sha256: EXPECTED_V3_TOOL_SCHEMA_SHA256 }),
      env, allowSyntheticTestEvidence: true });
    assert.equal(result.clients, 1);
    const report = JSON.parse(await readFile(outputPath, 'utf8'));
    assert.equal(report.live_client_smokes_complete, true);
    assert.equal(report.live_client_smokes[0].status, 'verified');
    assert.equal(report.live_client_smokes[0].live, true);
    assert.ok(report.live_client_smokes[0].integration_config_sha256);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('client adapter orchestrator fails explicitly when a configured client adapter is absent', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-client-smoke-missing-'));
  try {
    const matrixPath = join(directory, 'matrix.json');
    await writeFile(matrixPath, JSON.stringify({ clients: { codex: { requiredEvidenceCategories: required }, claude: { requiredEvidenceCategories: required } } }));
    await assert.rejects(runLiveClientSmokes({ matrixPath, outputPath: join(directory, 'results.json'), commands: {} }), /missing real-client smoke adapters for: codex, claude/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('public client smoke path requires and invokes its configured host task adapter before evidence collection', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-client-host-wiring-'));
  try {
    const matrixPath = join(directory, 'matrix.json');
    const row = { requiredEvidenceCategories: required, integrationConfig: 'integrations/codex/config.toml.example', adapterKind: 'stdio_mcp' };
    await writeFile(matrixPath, JSON.stringify({ clients: { codex: row } }));
    const env = { ...process.env, CONTROLLA_BINARY_PATH: process.execPath, CONTROLLA_CLIENT_SMOKE_STATE_ROOT: directory };
    const mcpContractRunner = async () => ({ status: 'contract_verified', qualification_status: 'not_run', schema_sha256: EXPECTED_V3_TOOL_SCHEMA_SHA256 });
    const clientHostAdapterResolver = async () => ({ client: 'codex', kind: 'cli', status: 'ready', qualification_status: 'not_run', executable: process.execPath });
    await assert.rejects(runLiveClientSmokes({ matrixPath, outputPath: join(directory, 'missing.json'), commands: { codex: [process.execPath, 'unused'] }, clientHostAdapterResolver, mcpContractRunner, env }), /CONTROLLA_CLIENT_HOST_TASKS_JSON must provide/);
    await assert.rejects(runLiveClientSmokes({ matrixPath, outputPath: join(directory, 'not-invoked.json'), commands: { codex: [process.execPath, 'unused'] },
      hostTasks: { codex: { read_only: 'read', verified_mutation: 'mutate', mode_specific: 'mode' } },
      clientHostAdapterResolver, hostTaskRunner: async () => ({ status: 'no_host_tool_invocation_observed', tool_events: [] }), mcpContractRunner, env }), /did not invoke a Controlla MCP tool/);
    await assert.rejects(runLiveClientSmokes({ matrixPath, outputPath: join(directory, 'wrong-mode.json'), commands: { codex: [process.execPath, 'unused'] },
      hostTasks: { codex: { read_only: 'read', verified_mutation: 'mutate', mode_specific: 'mode' } },
      hostTaskRunner: async (client, { task }) => ({ status: 'host_tool_invocation_observed', process_id: 321,
        tool_events: (task === 'read_only' ? ['snapshot', 'extract'] : task === 'verified_mutation' ? ['act', 'verify'] : ['browser']).map(name => ({ name: `chrome-controlla.${name}`, input: { action: 'open', mode: 'foreground' } })) }), clientHostAdapterResolver, mcpContractRunner, env }), /did not request a supported background or headless browser mode/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('each named client adapter resolves its checked-in integration config into an isolated launch context', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-client-config-adapters-'));
  try {
    const matrix = JSON.parse(await readFile(resolve('apps/client-qualification-matrix.json'), 'utf8'));
    const ids = Object.keys(matrix.clients);
    const configs = await Promise.all(ids.map(id => resolveClientAdapterConfig(id, matrix.clients[id], {
      env: { ...process.env, CONTROLLA_BINARY_PATH: process.execPath }, stateRoot: directory,
    })));
    assert.equal(configs.length, 6);
    assert.equal(new Set(configs.map(config => config.state_dir)).size, 6);
    for (const config of configs) {
      assert.ok(config.integration_config);
      assert.match(config.config_sha256, /^[a-f0-9]{64}$/);
      assert.equal(config.env.CONTROLLA_AGENT_SURFACE, 'v3-compact');
      assert.equal(config.env.CONTROLLA_STATE_DIR, config.state_dir);
      if (config.client === 'chatgpt-desktop') {
        assert.equal(config.command, process.execPath);
        assert.ok(config.args[0].endsWith(join('packages', 'chrome-controlla', 'plugin', 'launch.cjs')));
      } else {
        assert.equal(config.command, process.execPath);
        assert.deepEqual(config.args, ['mcp']);
      }
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('built-in client launch adapters handshake with stdio MCP and verify the exact V3 tools without claiming qualification', async () => {
  const adapterIds = registeredClientMcpAdapters();
  assert.deepEqual(Object.keys(adapterIds).sort(), ['chatgpt-desktop', 'claude-code', 'codex', 'freebuff', 'generic-mcp', 'opencode'].sort());
  const directory = await mkdtemp(join(tmpdir(), 'controlla-client-mcp-contract-'));
  try {
    const fixture = resolve(fileURLToPath(new URL('./test-fixtures/mcp-stdio-contract-server.mjs', import.meta.url)));
    const result = await runClientMcpContract('codex', { adapterKind: 'stdio_mcp' }, {
      launch: { command: process.execPath, args: [fixture], env: {}, config_sha256: 'a'.repeat(64), state_dir: directory },
      expectedSchemaSha256: fingerprintToolSchema(['act', 'browser', 'extract', 'snapshot', 'verify', 'workflow'].map(name => ({ name, inputSchema: { type: 'object', properties: {}, additionalProperties: false } }))),
    });
    assert.equal(result.status, 'contract_verified');
    assert.equal(result.qualification_status, 'not_run');
    assert.deepEqual(result.tools, ['act', 'browser', 'extract', 'snapshot', 'verify', 'workflow']);
    assert.equal(result.schema_sha256, fingerprintToolSchema(['act', 'browser', 'extract', 'snapshot', 'verify', 'workflow'].map(name => ({ name, inputSchema: { type: 'object', properties: {}, additionalProperties: false } }))));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('client host adapters distinguish runnable CLI hosts from interactive and protocol-only clients', async () => {
  const hosts = registeredClientHostAdapters();
  assert.deepEqual(Object.keys(hosts).sort(), ['chatgpt-desktop', 'claude-code', 'codex', 'freebuff', 'generic-mcp', 'opencode'].sort());
  assert.equal((await resolveClientHostAdapter('generic-mcp')).status, 'protocol_only');
  assert.equal((await resolveClientHostAdapter('chatgpt-desktop')).status, 'manual_host_required');
  assert.equal((await resolveClientHostAdapter('freebuff', { env: { PATH: '' } })).status, 'interactive_only');
  assert.equal((await resolveClientHostAdapter('codex', { env: { PATH: '', CONTROLLA_CODEX_BIN: '/definitely/missing/codex' } })).status, 'missing_dependency');
});

test('host launch adapters create isolated MCP-only configs and use supported non-interactive entrypoints', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-client-host-config-'));
  try {
    const launch = { command: '/tmp/controlla', args: ['mcp'], env: { CONTROLLA_STATE_DIR: `${directory}/state` } };
    const opencode = await prepareClientHostConfig('opencode', launch, join(directory, 'opencode'));
    const openConfig = JSON.parse(await readFile(join(opencode.configDir, 'opencode.json'), 'utf8'));
    assert.deepEqual(openConfig.mcp['chrome-controlla'].command, ['/tmp/controlla', 'mcp']);
    assert.deepEqual(openConfig.permission, { edit: 'deny', bash: 'deny' });
    const claude = await prepareClientHostConfig('claude-code', launch, join(directory, 'claude'));
    assert.deepEqual(JSON.parse(await readFile(claude.configPath, 'utf8')).mcpServers['chrome-controlla'].args, ['mcp']);
    const codexConfig = { ...launch, env: { CONTROLLA_STATE_DIR: `${directory}/state`, CONTROLLA_AGENT_SURFACE: 'v3-compact' } };
    const tracePath = join(directory, 'client-trace.jsonl');
    const tracedCodexConfig = await prepareClientHostConfig('codex', launch, join(directory, 'codex-traced'), {
      traceFile: tracePath, traceId: 'trace-1', coldWarm: 'warm',
    });
    assert.equal(tracedCodexConfig.env.CONTROLLA_PROMPT_TRACE_FILE, `${tracePath}.mcp`);
    const codexCall = buildClientHostInvocation('codex', { executable: '/bin/codex', workDir: directory, mcpConfig: codexConfig, prompt: 'read-only prompt' });
    assert.ok(codexCall.args.includes('--ephemeral'));
    assert.ok(codexCall.args.includes('--ignore-user-config'));
    assert.ok(codexCall.args.includes('--approve-for-me'), 'isolated Codex host tasks need the supported scoped approval path for MCP tool use');
    assert.ok(!codexCall.args.includes('read-only prompt'), 'Codex prompt text must not appear in OS process arguments');
    assert.equal(codexCall.args.at(-1), '-');
    assert.equal(codexCall.stdin, 'read-only prompt');
    assert.ok(codexCall.args.includes(`mcp_servers.chrome-controlla.env={CONTROLLA_AGENT_SURFACE = "v3-compact", CONTROLLA_STATE_DIR = ${JSON.stringify(codexConfig.env.CONTROLLA_STATE_DIR)}}`), 'Codex config override must encode only allowlisted nonsecret env as a TOML inline table');
    const secret = 'do-not-put-this-in-argv';
    assert.throws(() => buildClientHostInvocation('codex', { executable: '/bin/codex', workDir: directory,
      mcpConfig: { ...codexConfig, env: { ...codexConfig.env, API_TOKEN: secret } }, prompt: 'read-only prompt' }), /unsupported Codex MCP environment key/);
    assert.throws(() => buildClientHostInvocation('codex', { executable: '/bin/codex', workDir: directory,
      mcpConfig: { ...codexConfig, env: { ...codexConfig.env, CONTROLLA_STATE_DIR: secret } }, prompt: 'read-only prompt' }), /unsafe Codex MCP environment value/);
    assert.ok(!codexCall.args.some(argument => argument.includes(secret)));
    assert.throws(() => buildClientHostInvocation('opencode', { executable: '/bin/opencode', workDir: directory, mcpConfig: opencode, prompt: 'read-only prompt' }), /no documented stdin prompt channel/);
    const claudeCall = buildClientHostInvocation('claude-code', { executable: '/bin/claude', workDir: directory, mcpConfig: claude, prompt: 'read-only prompt' });
    assert.ok(claudeCall.args.includes('--strict-mcp-config'));
    assert.ok(claudeCall.args.includes('--input-format') && claudeCall.args.includes('stream-json'));
    assert.ok(!claudeCall.args.includes('read-only prompt'), 'Claude prompt text must not appear in OS process arguments');
    assert.deepEqual(JSON.parse(claudeCall.stdin), { type: 'user', message: { role: 'user', content: [{ type: 'text', text: 'read-only prompt' }] } });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('host task adapter records real client MCP tool events without promoting them to qualification evidence', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-host-task-'));
  try {
    const fakeCodex = join(directory, 'fake-codex.mjs');
    await writeFile(fakeCodex, `#!/usr/bin/env node\nlet prompt='';for await(const chunk of process.stdin)prompt+=chunk;if(prompt!=='inspect only')process.exitCode=2;process.stderr.write(prompt);\nconsole.log(JSON.stringify({type:'item.started', item:{id:'call-1',type:'mcp_tool_call',server:'chrome-controlla',tool:'browser',arguments:{action:'open',mode:'background'},status:'in_progress'}}));\nconsole.log(JSON.stringify({type:'item.completed', item:{id:'call-1',type:'mcp_tool_call',server:'chrome-controlla',tool:'browser',arguments:{action:'open',mode:'background'},status:'completed'}}));\nconsole.log(JSON.stringify({type:'item.completed', item:{type:'mcp_tool_call',tool:'chrome-controlla.snapshot',call_id:'fixture-call'}}));\nconsole.log(JSON.stringify({type:'item.started', item:{id:'call-2',type:'mcp_tool_call',server:'playwright',tool:'browser',arguments:{action:'navigate'},status:'in_progress'}}));\nconsole.log(JSON.stringify({type:'item.completed', item:{id:'call-2',type:'mcp_tool_call',server:'playwright',tool:'browser',arguments:{action:'navigate'},status:'completed'}}));\n`);
    const result = await runClientHostTask('codex', { executable: fakeCodex, workDir: directory, mcpConfig: { command: '/tmp/controlla', args: ['mcp'], env: {} }, prompt: 'inspect only', timeoutMs: 1000,
      spawnProcess: (_command, _args, options) => spawn(process.execPath, [fakeCodex], options) });
    assert.equal(result.status, 'host_tool_invocation_observed');
    assert.equal(result.qualification_status, 'not_run');
    assert.equal(result.tool_events.length, 2, 'bare Codex tool names must be attributed by server while competitor browser calls stay excluded');
    assert.equal(result.tool_events[0].name, 'browser');
    assert.equal(result.tool_events[0].server, 'chrome-controlla');
    assert.deepEqual(result.tool_events[0].input, { action: 'open', mode: 'background' });
    assert.equal(result.tool_events[1].name, 'chrome-controlla.snapshot');
    assert.match(result.stdout_sha256, /^[a-f0-9]{64}$/);
    assert.equal(result.stderr, undefined, 'raw client stderr is never returned');
    assert.equal(result.stderr_bytes_captured, Buffer.byteLength('inspect only'));
    assert.equal(result.stderr_truncated, false);
    assert.match(result.stderr_prefix_sha256, /^[a-f0-9]{64}$/);
    assert.doesNotMatch(JSON.stringify(result), /inspect only/, 'prompt text echoed to stderr is not retained');
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('host task adapter hashes only a bounded stderr prefix and reports truncation', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-host-stderr-'));
  try {
    const fakeCodex = join(directory, 'fake-codex.mjs');
    await writeFile(fakeCodex, `#!/usr/bin/env node\nfor await (const chunk of process.stdin) {}\nprocess.stderr.write('s'.repeat(70_000));\n`);
    const result = await runClientHostTask('codex', { executable: fakeCodex, workDir: directory,
      mcpConfig: { command: '/tmp/controlla', args: ['mcp'], env: {} }, prompt: 'safe fixture', timeoutMs: 1000,
      spawnProcess: (_command, _args, options) => spawn(process.execPath, [fakeCodex], options) });
    assert.equal(result.stderr, undefined);
    assert.equal(result.stderr_bytes_captured, 64_000);
    assert.equal(result.stderr_truncated, true);
    assert.match(result.stderr_prefix_sha256, /^[a-f0-9]{64}$/);
    assert.doesNotMatch(JSON.stringify(result), /s{128}/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('client host trace keeps process-start timing separate from prompt send and records failures before first tool', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-host-trace-'));
  try {
    const fakeCodex = join(directory, 'fake-codex.mjs');
    const traceFile = join(directory, 'trace.jsonl');
    await writeFile(fakeCodex, `#!/usr/bin/env node\nlet prompt='';for await (const chunk of process.stdin)prompt+=chunk;process.stderr.write(prompt);\nawait new Promise(resolve => setTimeout(resolve, 40));\nprocess.exitCode = 7;\n`);
    const failure = await runClientHostTask('codex', { executable: fakeCodex, workDir: directory,
      mcpConfig: { command: '/tmp/controlla', args: ['mcp'], env: {} }, prompt: 'fixture prompt', timeoutMs: 1000,
      traceFile, traceId: 'delayed-failure', spawnProcess: (_command, _args, options) => spawn(process.execPath, [fakeCodex], options) })
      .then(() => assert.fail('nonzero host exit should reject'), error => error);
    assert.match(failure.message, /exited with code 7/);
    assert.doesNotMatch(failure.message, /fixture prompt/);
    assert.equal(failure.stderr_bytes_captured, Buffer.byteLength('fixture prompt'));
    assert.equal(failure.stderr_truncated, false);
    assert.match(failure.stderr_prefix_sha256, /^[a-f0-9]{64}$/);
    assert.equal(failure.exit_code, 7);
    assert.equal(failure.prompt_trace.prompt_sent_observed, false);
    assert.equal(failure.prompt_trace.process_start_monotonic_ns !== null, true);
    assert.equal(failure.prompt_trace.process_end_monotonic_ns !== null, true);
    assert.equal(failure.prompt_trace.prompt_to_verified_ms, null, 'a process exit is not a verified outcome');
    assert.equal(failure.prompt_trace.prompt_to_final_ms, null, 'a missing final answer remains missing');
    const { readPromptTrace, summarizePromptTrace } = await import('./prompt-send-trace.mjs');
    const events = await readPromptTrace(traceFile);
    assert.ok(events.some(event => event.event === 'process_start'));
    assert.ok(events.some(event => event.event === 'process_end' && event.exit_code === 7));
    assert.equal(events.some(event => event.event === 'first_tool_decision'), false);
    const promptWritten = events.find(event => event.event === 'prompt_input_written');
    assert.equal(promptWritten.prompt_boundary, 'client_stdin_write_finished');
    assert.equal(promptWritten.provenance, 'child_stdin_finish');
    const summary = summarizePromptTrace(events);
    assert.equal(summary.prompt_sent_observed, false, 'a local stdin flush is not client-consumption evidence');
    assert.equal(summary.prompt_to_verified_ms, null);
    assert.equal(summary.process_start_to_exit_ms >= 30, true);
    assert.equal(summary.process_start_to_outcome_ms, undefined, 'process exit must not be labeled as a verified outcome');
    assert.equal(summary.prompt_to_first_tool_decision_ms, null);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('client tool-decision trace retains its observed call ID and tool name', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-host-call-id-'));
  try {
    const fakeCodex = join(directory, 'fake-codex.mjs');
    const traceFile = join(directory, 'trace.jsonl');
    await writeFile(fakeCodex, `#!/usr/bin/env node\nfor await (const chunk of process.stdin) {}\nprocess.stdout.write(JSON.stringify({type:'mcp_tool_call',server:'chrome-controlla',name:'snapshot',id:'client-call-7'})+'\\n');\n`);
    const result = await runClientHostTask('codex', { executable: fakeCodex, workDir: directory,
      mcpConfig: { command: '/tmp/controlla', args: ['mcp'], env: {} }, prompt: 'read fixture', timeoutMs: 1000,
      traceFile, traceId: 'call-id-trace',
      spawnProcess: (_command, _args, options) => spawn(process.execPath, [fakeCodex], options) });
    assert.equal(result.status, 'host_tool_invocation_observed');
    const { readPromptTrace } = await import('./prompt-send-trace.mjs');
    const events = await readPromptTrace(traceFile);
    const event = events.find(item => item.event === 'first_tool_decision');
    assert.equal(event.client_call_id, 'client-call-7');
    assert.equal(event.tool, 'snapshot');
    const call = events.find(item => item.event === 'client_tool_decision');
    assert.equal(call.client_call_id, 'client-call-7');
    assert.equal(call.tool, 'snapshot');
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('client and MCP proxy traces use separate files and merge by monotonic event time', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-host-trace-merge-'));
  try {
    const traceFile = join(directory, 'trace.jsonl');
    const mcpTraceFile = `${traceFile}.mcp`;
    const { recordPromptTraceEvent, readPromptTrace, readPromptTraceFiles, summarizePromptTrace } = await import('./prompt-send-trace.mjs');
    await recordPromptTraceEvent(traceFile, 'ordered', 'process_start', {}, 20n);
    await recordPromptTraceEvent(traceFile, 'ordered', 'prompt_input_written', { prompt_boundary: 'client_stdin_write_finished' }, 40n);
    await recordPromptTraceEvent(mcpTraceFile, 'ordered', 'controlla_received', { call_id: 'call-1' }, 30n);
    await recordPromptTraceEvent(mcpTraceFile, 'ordered', 'tool_result_delivered', { call_id: 'call-1', missing_counters: 'internal_model_calls' }, 50n);
    const events = await readPromptTraceFiles([traceFile, mcpTraceFile], 'ordered');
    assert.deepEqual(events.map(event => BigInt(event.monotonic_ns)), [20n, 30n, 40n, 50n]);
    assert.equal(summarizePromptTrace(events).trace_valid, true);
    await recordPromptTraceEvent(mcpTraceFile, 'old-run', 'controlla_received', { call_id: 'stale-call' }, 45n);
    await assert.rejects(readPromptTraceFiles([traceFile, mcpTraceFile], 'ordered'), /different run/);
    assert.equal((await readPromptTrace(traceFile)).length, 2, 'each process owns a separate append-only trace file');
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('client host timeout rejection retains partial trace without inventing outcome timings', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-host-timeout-'));
  try {
    const fakeCodex = join(directory, 'fake-codex.mjs');
    const traceFile = join(directory, 'trace.jsonl');
    await writeFile(fakeCodex, `#!/usr/bin/env node\nfor await (const chunk of process.stdin) {}\nawait new Promise(resolve => setInterval(resolve, 10000));\n`);
    const failure = await runClientHostTask('codex', { executable: fakeCodex, workDir: directory,
      mcpConfig: { command: '/tmp/controlla', args: ['mcp'], env: {} }, prompt: 'fixture prompt', timeoutMs: 50,
      traceFile, traceId: 'timed-out', spawnProcess: (_command, _args, options) => spawn(process.execPath, [fakeCodex], options) })
      .then(() => assert.fail('timed out host should reject'), error => error);
    assert.equal(failure.timed_out, true);
    assert.equal(failure.exit_code, null);
    assert.equal(failure.prompt_trace.prompt_sent_observed, false);
    assert.equal(failure.prompt_trace.process_end_monotonic_ns !== null, true);
    assert.equal(failure.prompt_trace.prompt_to_verified_ms, null);
    assert.equal(failure.prompt_trace.prompt_to_final_ms, null);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
