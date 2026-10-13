#!/usr/bin/env node
// Runs configured real-client adapters and accepts only current, independently bound evidence.
import { createHash, randomUUID } from 'node:crypto';
import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { invokeJsonCommand, parseCommand } from '../bench/runners/phase11-runner.mjs';
import { validateClientEvidence, validateClientSmoke } from './client-smoke-contract.mjs';
import { prepareClientHostConfig, resolveClientHostAdapter, runClientHostTask } from './client-host-adapters.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const defaultMatrixPath = resolve(root, 'apps/client-qualification-matrix.json');

function readJsonConfig(text, file) {
  try { return JSON.parse(text); }
  catch (error) { throw new Error(`invalid integration config ${file}: ${error.message}`); }
}

export async function resolveClientAdapterConfig(clientId, row, { env = process.env, stateRoot = join(tmpdir(), 'controlla-client-smokes') } = {}) {
  if (!row.integrationConfig || !['stdio_mcp', 'packaged_local_plugin'].includes(row.adapterKind)) {
    throw new Error(`${clientId} adapter is not bound to a supported integration config`);
  }
  const configPath = resolve(root, row.integrationConfig);
  const raw = await readFile(configPath, 'utf8');
  let command, args, configuredEnv = {};
  if (clientId === 'codex') {
    const commandMatch = raw.match(/\[mcp_servers\.chrome-controlla\][\s\S]*?command = "([^"]+)"[\s\S]*?args = \["([^"]+)"\]/);
    const stateMatch = raw.match(/CONTROLLA_STATE_DIR = "([^"]+)"/);
    if (!commandMatch || !stateMatch) throw new Error(`Codex adapter config is incomplete: ${configPath}`);
    command = commandMatch[1]; args = [commandMatch[2]];
    configuredEnv.CONTROLLA_STATE_DIR = stateMatch[1];
  } else {
    const config = readJsonConfig(raw, row.integrationConfig);
    const entry = clientId === 'freebuff' ? config : clientId === 'opencode' ? config.mcp?.['chrome-controlla']
      : clientId === 'claude-code' ? config.mcpServers?.['chrome-controlla']
      : clientId === 'generic-mcp' ? config.mcpServers?.['chrome-controlla']
      : config.mcpServers?.['chrome-controlla'];
    if (!entry) throw new Error(`${clientId} adapter has no chrome-controlla entry in ${configPath}`);
    const configuredCommand = entry.command;
    if (Array.isArray(configuredCommand)) [command, ...args] = configuredCommand;
    else { command = configuredCommand; args = [...(entry.args ?? [])]; }
    configuredEnv = entry.env ?? entry.environment ?? {};
    if (clientId === 'chatgpt-desktop') {
      command = process.execPath;
      args = [resolve(root, 'packages/chrome-controlla/plugin/launch.cjs')];
    }
  }
  const stateDir = resolve(stateRoot, clientId);
  const binary = env.CONTROLLA_BINARY_PATH ?? env.CONTROLLA_BIN;
  if (!binary || !isAbsolute(binary)) throw new Error(`CONTROLLA_BINARY_PATH must name an absolute built Controlla binary for the ${clientId} adapter`);
  await access(binary, constants.X_OK).catch(() => { throw new Error(`configured Controlla binary is not executable: ${binary}`); });
  if (clientId === 'chatgpt-desktop') command = process.execPath;
  else {
    command = binary;
    args = ['mcp'];
  }
  const launchEnv = { ...configuredEnv, CONTROLLA_AGENT_SURFACE: 'v3-compact', CONTROLLA_STATE_DIR: stateDir };
  if (clientId === 'chatgpt-desktop') launchEnv.CONTROLLA_BIN = binary;
  await mkdir(stateDir, { recursive: true });
  const configHash = createHash('sha256').update(raw).digest('hex');
  const adapter = {
    freebuff: 'freebuff-stdio', opencode: 'opencode-stdio', 'claude-code': 'claude-code-stdio',
    codex: 'codex-stdio', 'chatgpt-desktop': 'chatgpt-desktop-plugin', 'generic-mcp': 'generic-stdio',
  }[clientId];
  if (!adapter) throw new Error(`no concrete client adapter is registered for ${clientId}`);
  return { client: clientId, adapter, adapter_kind: row.adapterKind, integration_config: row.integrationConfig,
    command, args, env: launchEnv, state_dir: stateDir, config_sha256: configHash };
}

function normalizeCapabilities(value) {
  return Array.isArray(value) ? [...new Set(value.filter(item => typeof item === 'string'))].sort() : [];
}

export async function runLiveClientSmokes({
  matrixPath = defaultMatrixPath,
  outputPath = resolve(root, 'apps/client-smoke-results.json'),
  commands,
  hostTasks,
  hostTaskRunner = runClientHostTask,
  clientHostAdapterResolver = resolveClientHostAdapter,
  mcpContractRunner,
  env = process.env,
  allowSyntheticTestEvidence = false,
} = {}) {
  const clients = JSON.parse(await readFile(matrixPath, 'utf8')).clients;
  if (!commands || typeof commands !== 'object' || Array.isArray(commands)) {
    throw new Error('live client smoke adapters are missing: provide a JSON map from client ID to argv array; no client was run and no qualification evidence was created');
  }
  const ids = Object.keys(clients);
  const adapterIds = ids.map(id => clients[id].smokeAdapter ?? id);
  if (new Set(adapterIds).size !== adapterIds.length) throw new Error('client matrix contains duplicate smokeAdapter identifiers');
  const missing = ids.filter((id, index) => !Array.isArray(commands[adapterIds[index]]) || commands[adapterIds[index]].length === 0);
  if (missing.length) throw new Error(`missing real-client smoke adapters for: ${missing.join(', ')}; no client was run and no qualification evidence was created`);

  const results = [];
  for (const id of ids) {
    const adapterId = clients[id].smokeAdapter ?? id;
    const argv = parseCommand(JSON.stringify(commands[adapterId]), `client adapter ${adapterId}`);
    const required = normalizeCapabilities(clients[id].requiredEvidenceCategories);
    const runId = randomUUID();
    const startedAtMs = Date.now();
    const clientAdapter = await resolveClientAdapterConfig(id, clients[id], { env,
      stateRoot: env.CONTROLLA_CLIENT_SMOKE_STATE_ROOT ?? join(tmpdir(), `controlla-client-smoke-${runId}`) });
    const hostAdapter = await clientHostAdapterResolver(id, { env });
    const runContract = mcpContractRunner ?? (await import('./client-adapters.mjs')).runClientMcpContract;
    const clientSchemaContract = await runContract(id, clients[id], { launch: clientAdapter });
    if (clientSchemaContract.status !== 'contract_verified' || clientSchemaContract.qualification_status !== 'not_run') {
      throw new Error(`${id} client MCP schema contract did not verify during this run`);
    }
    let hostTaskRuns = [];
    if (hostAdapter.status === 'ready') {
      const tasks = hostTasks?.[id];
      const requiredTasks = ['read_only', 'verified_mutation', 'mode_specific'];
      if (!tasks || requiredTasks.some(task => typeof tasks[task] !== 'string' || !tasks[task].trim())) {
        throw new Error(`${id} host adapter is available, but CONTROLLA_CLIENT_HOST_TASKS_JSON must provide read_only, verified_mutation, and mode_specific prompts; no client evidence was written`);
      }
      const hostConfig = await prepareClientHostConfig(id, clientAdapter, join(clientAdapter.state_dir, 'host-config'));
      for (const task of requiredTasks) {
        const run = await hostTaskRunner(id, { task, executable: hostAdapter.executable, workDir: clientAdapter.state_dir,
          mcpConfig: { ...clientAdapter, ...hostConfig }, prompt: tasks[task], env });
        if (run.status !== 'host_tool_invocation_observed' || !Array.isArray(run.tool_events) || run.tool_events.length === 0) {
          throw new Error(`${id} ${task} host task did not invoke a Controlla MCP tool; qualification evidence was not written`);
        }
        const usedTools = new Set(run.tool_events.map(event => /(?:__|[._-])(act|browser|extract|snapshot|verify|workflow)$/.exec(event.name)?.[1]).filter(Boolean));
        const taskTools = task === 'read_only' ? ['snapshot', 'extract'] : task === 'verified_mutation' ? ['act', 'verify'] : ['browser'];
        if (!taskTools.every(tool => usedTools.has(tool))) throw new Error(`${id} ${task} host run did not exercise required Controlla tool(s): ${taskTools.join(', ')}`);
        if (task === 'mode_specific' && !run.tool_events.some(event => /(?:__|[._-])browser$/.test(event.name)
          && event.input?.action === 'open' && ['background', 'headless'].includes(event.input.mode))) {
          throw new Error(`${id} mode_specific host run did not request a supported background or headless browser mode`);
        }
        hostTaskRuns.push({ task, ...run });
      }
    } else if (hostAdapter.status === 'missing_dependency' || hostAdapter.status === 'interactive_only' || hostAdapter.status === 'manual_host_required') {
      if (!commands[adapterId]) throw new Error(`${id} client host is ${hostAdapter.status}: ${hostAdapter.reason}; provide an explicitly registered in-app driver; no qualification evidence was written`);
    }
    const rawResult = await invokeJsonCommand(argv, {
      client: id, run_id: runId, started_at: new Date(startedAtMs).toISOString(), required_capabilities: required,
      client_adapter: clientAdapter,
      client_host_adapter: hostAdapter,
      client_schema_contract: clientSchemaContract,
      host_task_runs: hostTaskRuns,
    }, 300_000, env);
    const hostTaskBinding = hostTaskRuns.map(({ task, process_id, tool_events, stdout_sha256 }) => ({ task, process_id, tool_events, stdout_sha256 }));
    const matchesHostTaskBinding = value => Array.isArray(value) && JSON.stringify(value.map(({ task, process_id, tool_events, stdout_sha256 }) => ({ task, process_id, tool_events, stdout_sha256 }))) === JSON.stringify(hostTaskBinding);
    if (!matchesHostTaskBinding(rawResult.host_task_runs)) {
      throw new Error(`${id} evidence driver did not return the exact observed host-task process IDs, tool calls, and output hashes; no qualification result was written`);
    }
    const result = validateClientSmoke(id, rawResult, required, runId, clientAdapter.config_sha256);
    const finishedAtMs = Date.now();
    const bytes = await readFile(result.evidence_path);
    const digest = createHash('sha256').update(bytes).digest('hex');
    if (digest !== result.evidence_sha256) throw new Error(`${id} evidence SHA-256 does not match the file contents`);
    let evidence;
    try { evidence = JSON.parse(bytes.toString('utf8')); }
    catch (error) { throw new Error(`${id} evidence file must contain valid JSON: ${error.message}`); }
    if (!matchesHostTaskBinding(evidence.host_task_runs)) {
      throw new Error(`${id} evidence file is not bound to the host-task processes observed by this run`);
    }
    const verified = validateClientEvidence(id, result, evidence, required, { runId, startedAtMs, finishedAtMs },
      { allowSyntheticTestEvidence, expectedConfigHash: clientAdapter.config_sha256 });
    results.push({ client: id, status: 'verified', live: true, verified_capabilities: verified.verified_capabilities,
      evidence_path: result.evidence_path, evidence_hash: digest, client_version: result.client_version,
      integration_config_sha256: clientAdapter.config_sha256,
      controlla_version: result.controlla_version });
  }

  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, `${JSON.stringify({ schema_version: 1, live_client_smokes_complete: true, live_client_smokes: results }, null, 2)}\n`, { flag: 'wx' });
  return { clients: results.length, outputPath };
}

async function main() {
  let commands, hostTasks;
  try {
    commands = JSON.parse(process.env.CONTROLLA_CLIENT_SMOKE_COMMANDS_JSON ?? 'null');
  } catch (error) {
    throw new Error(`CONTROLLA_CLIENT_SMOKE_COMMANDS_JSON must be valid JSON: ${error.message}`);
  }
  try {
    hostTasks = JSON.parse(process.env.CONTROLLA_CLIENT_HOST_TASKS_JSON ?? 'null');
  } catch (error) {
    throw new Error(`CONTROLLA_CLIENT_HOST_TASKS_JSON must be valid JSON: ${error.message}`);
  }
  const stamp = new Date().toISOString().replaceAll(/[:.]/g, '-');
  const outputPath = resolve(root, process.env.CONTROLLA_CLIENT_SMOKE_OUTPUT ?? `apps/client-smoke-results-${stamp}.json`);
  const result = await runLiveClientSmokes({ commands, hostTasks, outputPath });
  console.log(`Verified live smoke evidence for ${result.clients} configured clients: ${result.outputPath}`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main().catch(error => { console.error(error.stack ?? error.message); process.exit(2); });
}
