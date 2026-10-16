import { access, mkdir, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { delimiter, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { recordPromptTraceEvent, readPromptTraceFiles, summarizePromptTrace } from './prompt-send-trace.mjs';

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
    // The host adapter and MCP proxy are separate processes. Keep their
    // append-only streams separate and merge by monotonic timestamp later.
    env: { ...launch.env, CONTROLLA_PROMPT_TRACE_FILE: `${traceOptions.traceFile}.mcp`,
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
  if (clientId === 'codex') return { command: executable, args: ['exec', '--ephemeral', '--json', '--ignore-user-config', '--approve-for-me', '--skip-git-repo-check', '-C', workDir, '-c', `mcp_servers.chrome-controlla.command=${JSON.stringify(mcpConfig.command)}`, '-c', `mcp_servers.chrome-controlla.args=${JSON.stringify(mcpConfig.args)}`, '-c', `mcp_servers.chrome-controlla.env=${codexTomlInlineTable(mcpConfig.env)}`, '-'], stdin: prompt };
  if (clientId === 'opencode') {
    throw new Error('OpenCode CLI has no documented stdin prompt channel; refusing to expose prompt text in process arguments');
  }
  if (clientId === 'claude-code') return {
    command: executable,
    args: ['-p', '--output-format', 'stream-json', '--input-format', 'stream-json', '--strict-mcp-config', '--mcp-config', mcpConfig.configPath, '--max-turns', '8'],
    stdin: `${JSON.stringify({ type: 'user', message: { role: 'user', content: [{ type: 'text', text: prompt }] } })}\n`,
  };
  throw new Error(`${clientId} has no supported non-interactive host invocation`);
}

function codexTomlInlineTable(environment) {
  if (!environment || typeof environment !== 'object' || Array.isArray(environment)) {
    throw new Error('Codex MCP environment must be a map of string values');
  }
  const allowedNonSecretKeys = new Set([
    'CONTROLLA_STATE_DIR',
    'CONTROLLA_AGENT_SURFACE',
    'COMPTROL_CHROME_EXECUTABLE',
    'CONTROLLA_PROMPT_TRACE_FILE',
    'CONTROLLA_PROMPT_TRACE_ID',
    'CONTROLLA_PROMPT_COLD_WARM',
  ]);
  const entries = Object.entries(environment).sort(([left], [right]) => left.localeCompare(right));
  for (const [key, value] of entries) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key) || typeof value !== 'string') {
      throw new Error('Codex MCP environment keys must be identifiers and values must be strings');
    }
    if (!allowedNonSecretKeys.has(key)) {
      throw new Error(`unsupported Codex MCP environment key '${key}'; command-line config accepts only allowlisted nonsecret values`);
    }
    const safeValue = key === 'CONTROLLA_STATE_DIR' || key === 'COMPTROL_CHROME_EXECUTABLE'
      || key === 'CONTROLLA_PROMPT_TRACE_FILE'
      ? isAbsolute(value)
      : key === 'CONTROLLA_AGENT_SURFACE' ? value === 'v3-compact'
        : key === 'CONTROLLA_PROMPT_COLD_WARM' ? ['cold', 'warm', 'unknown'].includes(value)
          : key === 'CONTROLLA_PROMPT_TRACE_ID' && /^[A-Za-z0-9._:-]{1,128}$/.test(value);
    if (!safeValue) throw new Error(`unsafe Codex MCP environment value for '${key}'`);
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
    const clientToolDecisions = new Set();
    const queueTrace = (event, fields = {}, happenedAt = process.hrtime.bigint()) => {
      if (!traceFile) return;
      traceWrites = traceWrites.then(() => recordPromptTraceEvent(traceFile, traceId, event,
        { clock_domain: 'local_host_monotonic', clock_offset_ms: 0, ...fields }, happenedAt));
    };
    let firstToolDecisionRecorded = false;
    let finalAnswerRecorded = false;
    const consumeLine = line => {
      if (!line.trim()) return;
      let value;
      try { value = JSON.parse(line); } catch { return; }
      const parsed = collectToolEvents(value);
      hostEvents.push(...parsed);
      const controllaTools = collectToolEventsFromHost(parsed);
      if (controllaTools.length && !firstToolDecisionRecorded) {
        firstToolDecisionRecorded = true;
        const firstControllaTool = controllaTools[0];
        queueTrace('first_tool_decision', { external_tool_calls: controllaTools.length,
          ...(typeof firstControllaTool?.id === 'string' || Number.isSafeInteger(firstControllaTool?.id)
            ? { client_call_id: String(firstControllaTool.id).slice(0, 128) } : {}),
          ...(typeof firstControllaTool?.name === 'string' ? { tool: firstControllaTool.name.slice(0, 80) } : {}) });
      }
      for (const tool of controllaTools) {
        if (!(typeof tool.id === 'string' || Number.isSafeInteger(tool.id))) continue;
        const clientCallId = String(tool.id).slice(0, 128);
        const key = `${clientCallId}\u0000${tool.name}`;
        if (clientToolDecisions.has(key)) continue;
        clientToolDecisions.add(key);
        queueTrace('client_tool_decision', { client_call_id: clientCallId, tool: tool.name.slice(0, 80),
          provenance: 'client_host_tool_event' });
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
    const child = spawnProcess(invocation.command, invocation.args, { cwd: workDir, env: { ...env, ...invocation.env }, stdio: ['pipe', 'pipe', 'pipe'] });
    child.once('spawn', () => queueTrace('process_start', { client: clientId, child_pid: child.pid }));
    if (invocation.stdin !== undefined) {
      // Codex and Claude Code accept their prompt through stdin. The prompt is not exposed in the process list.
      child.stdin.once('finish', () => {
        // Writable `finish` means Node flushed the complete prompt into the
        // child stdin stream. It is still not an acknowledgement from the model.
        // Record local pipe-write completion only. It does not prove that the
        // client consumed the prompt or began its model turn.
        queueTrace('prompt_input_written', { prompt_input_bytes: Buffer.byteLength(prompt), cold_warm: coldWarm,
          prompt_boundary: 'client_stdin_write_finished', provenance: 'child_stdin_finish', clock_uncertainty_ms: 0 });
      });
      child.stdin.end(invocation.stdin);
    } else {
      // Prompt arguments are present at process launch, but the host exposes no
      // separate submission hook. Preserve that interval as process timing only.
      child.stdin.end();
    }
    const stdout = []; const stderr = []; let stdoutBytes = 0; let stderrBytes = 0; let stderrTruncated = false;
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
    child.stderr.on('data', chunk => {
      if (stderrBytes >= 64_000) { stderrTruncated = true; return; }
      const bounded = Buffer.from(chunk).subarray(0, 64_000 - stderrBytes);
      if (bounded.length) { stderr.push(bounded); stderrBytes += bounded.length; }
      if (bounded.length < chunk.length) stderrTruncated = true;
    });
    let settled = false;
    let timedOut = false;
    const rejectWithTrace = async (error, exitCode = null) => {
      if (settled) return;
      settled = true;
      let promptTrace = null;
      try {
        await traceWrites;
        if (traceFile) promptTrace = summarizePromptTrace(await readPromptTraceFiles([traceFile, `${traceFile}.mcp`], traceId));
      } catch {
        // Failure diagnostics must not mask the adapter's original failure.
      }
      // Attach only the bounded trace summary, never prompt text or raw streams.
      error.process_id = child.pid ?? null;
      error.exit_code = exitCode;
      error.timed_out = timedOut;
      error.prompt_trace = promptTrace;
      rejectTask(error);
    };
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGTERM'); }, timeoutMs);
    child.once('error', error => {
      clearTimeout(timer);
      queueTrace('process_end');
      void rejectWithTrace(error);
    });
    child.once('close', code => {
      if (settled) return;
      clearTimeout(timer);
      queueTrace('process_end', Number.isInteger(code) && code >= 0 ? { exit_code: code } : {});
      const output = Buffer.concat(stdout).toString('utf8');
      const stderrOutput = Buffer.concat(stderr);
      const stderrSha256 = createHash('sha256').update(stderrOutput).digest('hex');
      consumeLine(lineBuffer);
      void (async () => {
        await traceWrites;
        if (code !== 0 || timedOut) {
          const error = new Error(timedOut
            ? `${clientId} host exceeded its ${timeoutMs}ms deadline`
            : `${clientId} host exited with code ${code}`);
          error.stderr_bytes_captured = stderrOutput.length;
          error.stderr_prefix_sha256 = stderrSha256;
          error.stderr_truncated = stderrTruncated;
          await rejectWithTrace(error, Number.isInteger(code) && code >= 0 ? code : null);
          return;
        }
        const controllaEvents = collectToolEventsFromHost(hostEvents);
        const promptTrace = traceFile ? summarizePromptTrace(await readPromptTraceFiles([traceFile, `${traceFile}.mcp`], traceId)) : null;
        settled = true;
        resolveTask({ client: clientId, status: controllaEvents.length ? 'host_tool_invocation_observed' : 'no_host_tool_invocation_observed', qualification_status: 'not_run', process_id: child.pid,
          tool_events: controllaEvents, stdout_sha256: createHash('sha256').update(output).digest('hex'),
          stderr_bytes_captured: stderrOutput.length, stderr_prefix_sha256: stderrSha256,
          stderr_truncated: stderrTruncated, exit_code: code,
          prompt_trace: promptTrace });
      })().catch(rejectTask);
    });
  });
}

export function registeredClientHostAdapters() {
  return Object.fromEntries(Object.entries(HOSTS).map(([id, host]) => [id, { ...host }]));
}
