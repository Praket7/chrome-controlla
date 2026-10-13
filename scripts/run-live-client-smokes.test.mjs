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
    const codexCall = buildClientHostInvocation('codex', { executable: '/bin/codex', workDir: directory, mcpConfig: launch, prompt: 'read-only prompt' });
    assert.ok(codexCall.args.includes('--ephemeral'));
    assert.ok(codexCall.args.includes('--ignore-user-config'));
    assert.ok(codexCall.args.includes('read-only prompt'));
    const openCall = buildClientHostInvocation('opencode', { executable: '/bin/opencode', workDir: directory, mcpConfig: opencode, prompt: 'read-only prompt' });
    assert.ok(openCall.args.includes('--format') && openCall.args.includes('json'));
    const claudeCall = buildClientHostInvocation('claude-code', { executable: '/bin/claude', workDir: directory, mcpConfig: claude, prompt: 'read-only prompt' });
    assert.ok(claudeCall.args.includes('--strict-mcp-config'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('host task adapter records real client MCP tool events without promoting them to qualification evidence', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'controlla-host-task-'));
  try {
    const fakeCodex = join(directory, 'fake-codex.mjs');
    await writeFile(fakeCodex, `#!/usr/bin/env node\nconsole.log(JSON.stringify({type:'item.completed', item:{type:'mcp_tool_call', tool:'chrome-controlla.snapshot', call_id:'fixture-call'}}));\n`);
    const result = await runClientHostTask('codex', { executable: fakeCodex, workDir: directory, mcpConfig: { command: '/tmp/controlla', args: ['mcp'], env: {} }, prompt: 'inspect only', timeoutMs: 1000,
      spawnProcess: (_command, _args, options) => spawn(process.execPath, [fakeCodex], options) });
    assert.equal(result.status, 'host_tool_invocation_observed');
    assert.equal(result.qualification_status, 'not_run');
    assert.equal(result.tool_events[0].name, 'chrome-controlla.snapshot');
    assert.match(result.stdout_sha256, /^[a-f0-9]{64}$/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
