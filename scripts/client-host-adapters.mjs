import { access, mkdir, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { delimiter, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { recordPromptTraceEvent, readPromptTrace, summarizePromptTrace } from './prompt-send-trace.mjs';

const HOSTS = Object.freeze({
  codex: { executable: 'codex', kind: 'cli', support: 'non_interactive' },
  opencode: { executable: 'opencode', kind: 'cli', support: 'non_interactive' },
  'claude-code': { executable: 'claude', kind: 'cli', support: 'non_interactive' },
  freebuff: { executable: 'freebuff', kind: 'cli', support: 'interactive_only' },
  'chatgpt-desktop': { executable: null, kind: 'desktop', support: 'desktop_only' },
  'generic-mcp': { executable: null, kind: 'protocol', support: 'stdio_protocol' },
});

async function findExecutable(name, env) {
  for (const directory of (env.PATH ?? '').split(delimiter)) {
    const candidate = join(directory, name);
    try { await access(candidate, constants.X_OK); return candidate; } catch { /* continue */ }
  }
  return null;
}

export async function resolveClientHostAdapter(clientId, { env = process.env } = {}) {
  const host = HOSTS[clientId];
  if (!host) throw new Error(`no client host adapter is registered for ${clientId}`);
  if (host.kind === 'protocol') return { client: clientId, kind: host.kind, status: 'protocol_only', qualification_status: 'not_run', reason: 'Generic MCP has no model-client host; run the stdio contract adapter.' };
  if (host.kind === 'desktop') return { client: clientId, kind: host.kind, status: 'manual_host_required', qualification_status: 'not_run', reason: 'ChatGPT Desktop exposes no supported isolated non-interactive host runner; desktop launch and in-app evidence are required.' };
  if (host.support === 'interactive_only') return { client: clientId, kind: host.kind, status: 'interactive_only', qualification_status: 'not_run', executable: host.executable, reason: 'Freebuff currently exposes no non-interactive prompt command; its app must provide the client-side smoke driver.' };
  const override = env[`CONTROLLA_${clientId.toUpperCase().replaceAll('-', '_')}_BIN`];
  const executable = override ? resolve(override) : await findExecutable(host.executable, env);
  if (!executable) return { client: clientId, kind: host.kind, status: 'missing_dependency', qualification_status: 'not_run', executable: host.executable, reason: `Required client executable '${host.executable}' was not found on PATH or in its CONTROLLA_*_BIN override.` };
  try { await access(executable, constants.X_OK); } catch { return { client: clientId, kind: host.kind, status: 'missing_dependency', qualification_status: 'not_run', executable, reason: `Configured client executable is missing or not executable: ${executable}` }; }
  return { client: clientId, kind: host.kind, status: 'ready', qualification_status: 'not_run', executable };
}

export async function prepareClientHostConfig(clientId, launch, directory, traceOptions = undefined) {
  await mkdir(directory, { recursive: true });
  const server = traceOptions?.traceFile ? {
    command: process.execPath,
    args: [fileURLToPath(new URL('./mcp-stdio-trace-proxy.mjs', import.meta.url)), launch.command, ...launch.args],
    env: { ...launch.env, CONTROLLA_PROMPT_TRACE_FILE: traceOptions.traceFile,
      CONTROLLA_PROMPT_TRACE_ID: traceOptions.traceId,
      CONTROLLA_PROMPT_COLD_WARM: traceOptions.coldWarm ?? 'unknown' },
  } : { command: launch.command, args: launch.args, env: launch.env };
  if (clientId === 'codex') return { command: server.command, args: server.args, env: server.env };
  if (clientId === 'opencode') {
    const configDir = resolve(directory);
    await writeFile(join(configDir, 'opencode.json'), `${JSON.stringify({ '$schema': 'https://opencode.ai/config.json', mcp: { 'chrome-controlla': { type: 'local', command: [server.command, ...server.args], enabled: true, environment: server.env } }, permission: { edit: 'deny', bash: 'deny' } }, null, 2)}\n`, { flag: 'wx' });
    return { configDir };
  }
  if (clientId === 'claude-code') {
    const configPath = join(resolve(directory), 'claude-mcp.json');
    await writeFile(configPath, `${JSON.stringify({ mcpServers: { 'chrome-controlla': { command: server.command, args: server.args, env: server.env } } }, null, 2)}\n`, { flag: 'wx' });
    return { configPath };
  }
  throw new Error(`${clientId} does not support an isolated CLI host config`);
}

export function buildClientHostInvocation(clientId, { executable, workDir, mcpConfig, prompt }) {
  if (!HOSTS[clientId] || typeof executable !== 'string' || typeof workDir !== 'string' || typeof prompt !== 'string') {
    throw new Error(`invalid host invocation context for ${clientId}`);
  }
  if (clientId === 'codex') return { command: executable, args: ['exec', '--ephemeral', '--json', '--ignore-user-config', '--approve-for-me', '--skip-git-repo-check', '-C', workDir, '-c', `mcp_servers.chrome-controlla.command=${JSON.stringify(mcpConfig.command)}`, '-c', `mcp_servers.chrome-controlla.args=${JSON.stringify(mcpConfig.args)}`, '-c', `mcp_servers.chrome-controlla.env=${codexTomlInlineTable(mcpConfig.env)}`, prompt] };
  if (clientId === 'opencode') return { command: executable, args: ['run', '--format', 'json', '--dir', workDir, prompt], env: { OPENCODE_CONFIG_DIR: mcpConfig.configDir } };
  if (clientId === 'claude-code') return { command: executable, args: ['-p', '--output-format', 'json', '--strict-mcp-config', '--mcp-config', mcpConfig.configPath, '--max-turns', '8', prompt] };
  throw new Error(`${clientId} has no supported non-interactive host invocation`);
}

function codexTomlInlineTable(environment) {
  if (!environment || typeof environment !== 'object' || Array.isArray(environment)) {
    throw new Error('Codex MCP environment must be a map of string values');
  }
  const entries = Object.entries(environment).sort(([left], [right]) => left.localeCompare(right));
  for (const [key, value] of entries) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key) || typeof value !== 'string') {
      throw new Error('Codex MCP environment keys must be identifiers and values must be strings');
    }
  }
  return `{${entries.map(([key, value]) => `${key} = ${JSON.stringify(value)}`).join(', ')}}`;
}

function collectToolEvents(value, events = []) {
  if (Array.isArray(value)) { for (const item of value) collectToolEvents(item, events); return events; }
  if (!value || typeof value !== 'object') return events;
  const name = value.tool_name ?? value.toolName ?? value.tool ?? value.name;
  const type = String(value.type ?? value.event ?? '').toLowerCase();
  if (typeof name === 'string' && (type.includes('tool') || name.includes('chrome-controlla'))) {
    const input = value.arguments ?? value.input ?? value.tool_input ?? value.params?.arguments;
    let boundedInput;
    if (input && typeof input === 'object' && !Array.isArray(input)) {
      const safe = {};
      for (const key of ['action', 'mode', 'provider']) if (typeof input[key] === 'string') safe[key] = input[key].slice(0, 80);
      if (Object.keys(safe).length) boundedInput = safe;
    }
    events.push({ name, type, id: value.id ?? value.call_id ?? null,
      ...(typeof value.server === 'string' ? { server: value.server } : {}),
      ...(typeof value.status === 'string' ? { status: value.status } : {}),
      ...(boundedInput === undefined ? {} : { input: boundedInput }) });
  }
  for (const [key, child] of Object.entries(value)) if (!['tool_input', 'arguments', 'input', 'params'].includes(key)) collectToolEvents(child, events);
  return events;
}

function collectToolEventsFromHost(events) {
  const controllaTools = new Set(['act', 'browser', 'extract', 'snapshot', 'verify', 'workflow']);
  return events.filter(event => event.status !== 'in_progress'
    && (event.server === 'chrome-controlla' && controllaTools.has(event.name)
      || /chrome-controlla[._-](act|browser|extract|snapshot|verify|workflow)|mcp__chrome-controlla__(act|browser|extract|snapshot|verify|workflow)/.test(event.name)));
}

export async function runClientHostTask(clientId, { executable, workDir, mcpConfig, prompt, env = process.env, timeoutMs = 120_000, spawnProcess = spawn,
  traceFile, traceId = 'unbound', coldWarm = 'unknown' } = {}) {
  const invocation = buildClientHostInvocation(clientId, { executable, workDir, mcpConfig, prompt });
  return new Promise((resolveTask, rejectTask) => {
    let traceWrites = Promise.resolve();
    let lineBuffer = '';
    const hostEvents = [];
    const queueTrace = (event, fields = {}) => {
      if (!traceFile) return;
      const happenedAt = process.hrtime.bigint();
      traceWrites = traceWrites.then(() => recordPromptTraceEvent(traceFile, traceId, event, fields, happenedAt));
    };
    let firstToolDecisionRecorded = false;
    let finalAnswerRecorded = false;
    const consumeLine = line => {
      if (!line.trim()) return;
      let value;
      try { value = JSON.parse(line); } catch { return; }
      const parsed = collectToolEvents(value);
      hostEvents.push(...parsed);
      if (parsed.length && !firstToolDecisionRecorded) {
        firstToolDecisionRecorded = true;
        queueTrace('first_tool_decision', { external_tool_calls: parsed.length });
      }
      const type = String(value.type ?? value.event ?? '').toLowerCase();
      const nestedType = String(value.item?.type ?? value.message?.type ?? '').toLowerCase();
      if (/turn[._ -]?completed|turn[._ -]?complete/.test(type)) queueTrace('model_turn');
      if (/tool_result|mcp_tool_result|tool_output/.test(`${type} ${nestedType}`)) queueTrace('model_visible_observation');
      if (!finalAnswerRecorded && ((/item[._ -]?completed/.test(type) && /agent_message|assistant_message/.test(nestedType))
        || /assistant_final|final_answer|turn[._ -]?result/.test(type))) {
        finalAnswerRecorded = true;
        queueTrace('final_answer');
      }
      if (clientId === 'claude-code' && type === 'result') {
        finalAnswerRecorded = true;
        queueTrace('final_answer');
      }
    };
    const child = spawnProcess(invocation.command, invocation.args, { cwd: workDir, env: { ...env, ...invocation.env }, stdio: ['ignore', 'pipe', 'pipe'] });
    // The prompt is delivered as a process argument; timestamp immediately after
    // process creation, before consuming any host output or MCP events.
    queueTrace('prompt_sent', { input_bytes: Buffer.byteLength(prompt), cold_warm: coldWarm });
    const stdout = []; const stderr = []; let stdoutBytes = 0; let stderrBytes = 0;
    child.stdout.on('data', chunk => {
      const bounded = Buffer.from(chunk).subarray(0, Math.max(0, 4_000_000 - stdoutBytes));
      if (bounded.length) { stdout.push(bounded); stdoutBytes += bounded.length; }
      lineBuffer += bounded.toString('utf8');
      let newline;
      while ((newline = lineBuffer.indexOf('\n')) >= 0) {
        consumeLine(lineBuffer.slice(0, newline).replace(/\r$/, ''));
        lineBuffer = lineBuffer.slice(newline + 1);
      }
    });
    child.stderr.on('data', chunk => { if (stderrBytes < 64_000) { const bounded = Buffer.from(chunk).subarray(0, 64_000 - stderrBytes); stderr.push(bounded); stderrBytes += bounded.length; } });
    const timer = setTimeout(() => child.kill('SIGTERM'), timeoutMs);
    child.once('error', error => { clearTimeout(timer); rejectTask(error); });
    child.once('close', code => {
      clearTimeout(timer);
      const output = Buffer.concat(stdout).toString('utf8');
      const errorOutput = Buffer.concat(stderr).toString('utf8').slice(0, 64_000);
      consumeLine(lineBuffer);
      void (async () => {
        await traceWrites;
        if (code !== 0) throw new Error(`${clientId} host exited with code ${code}${errorOutput ? `: ${errorOutput}` : ''}`);
        const controllaEvents = collectToolEventsFromHost(hostEvents);
        const promptTrace = traceFile ? summarizePromptTrace(await readPromptTrace(traceFile)) : null;
        resolveTask({ client: clientId, status: controllaEvents.length ? 'host_tool_invocation_observed' : 'no_host_tool_invocation_observed', qualification_status: 'not_run', process_id: child.pid,
          tool_events: controllaEvents, stdout_sha256: createHash('sha256').update(output).digest('hex'), stderr: errorOutput, exit_code: code,
          prompt_trace: promptTrace });
      })().catch(rejectTask);
    });
  });
}

export function registeredClientHostAdapters() {
  return Object.fromEntries(Object.entries(HOSTS).map(([id, host]) => [id, { ...host }]));
}
