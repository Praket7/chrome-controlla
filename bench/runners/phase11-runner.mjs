import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');
const MAX_STDIO_BYTES = 1024 * 1024;

export async function loadPhase11Config() {
  return JSON.parse(await readFile(resolve(root, 'bench/tasks/phase11-realistic.json'), 'utf8'));
}

export async function loadAdapterContract() {
  return JSON.parse(await readFile(resolve(root, 'bench/runners/adapter-contract.json'), 'utf8'));
}

function commandEnvName(adapterId) {
  return `PHASE11_${adapterId.toUpperCase().replace(/[^A-Z0-9]+/g, '_')}_COMMAND_JSON`;
}

export function parseCommand(value, label) {
  if (!value) throw new Error(`${label} is required and must be a JSON argv array`);
  let argv;
  try {
    argv = JSON.parse(value);
  } catch (error) {
    throw new Error(`${label} must be valid JSON: ${error.message}`);
  }
  if (!Array.isArray(argv) || argv.length === 0 || argv.some(part => typeof part !== 'string' || part.length === 0)) {
    throw new Error(`${label} must be a non-empty JSON array of non-empty strings`);
  }
  return argv;
}

function boundedAppend(current, chunk, label) {
  const next = current + chunk;
  if (Buffer.byteLength(next) > MAX_STDIO_BYTES) throw new Error(`${label} exceeded ${MAX_STDIO_BYTES} bytes`);
  return next;
}

export async function invokeJsonCommand(argv, payload, timeoutMs, env = process.env) {
  return await new Promise((resolvePromise, reject) => {
    const child = spawn(argv[0], argv.slice(1), {
      cwd: root,
      env,
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true,
    });
    let stdout = '';
    let stderr = '';
    let settled = false;
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(error);
      else resolvePromise(value);
    };
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      finish(new Error(`command deadline exceeded after ${timeoutMs}ms`));
    }, timeoutMs);
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      try { stdout = boundedAppend(stdout, chunk, 'stdout'); }
      catch (error) { child.kill('SIGKILL'); finish(error); }
    });
    child.stderr.on('data', chunk => {
      try { stderr = boundedAppend(stderr, chunk, 'stderr'); }
      catch (error) { child.kill('SIGKILL'); finish(error); }
    });
    child.on('error', error => finish(error));
    child.on('close', code => {
      if (settled) return;
      if (code !== 0) return finish(new Error(`command exited ${code}: ${stderr.trim()}`));
      try {
        finish(null, JSON.parse(stdout));
      } catch (error) {
        finish(new Error(`command returned invalid JSON: ${error.message}; stderr=${stderr.trim()}`));
      }
    });
    child.stdin.end(JSON.stringify(payload));
  });
}

function numericMetric(result, name) {
  if (typeof result[name] !== 'number' || !Number.isFinite(result[name]) || result[name] < 0) {
    throw new Error(`adapter result missing non-negative numeric ${name}`);
  }
}

export function validateAdapterResult(result, contract, expected) {
  if (!result || typeof result !== 'object' || Array.isArray(result)) throw new Error('adapter result must be an object');
  for (const field of contract.output_required) {
    if (!(field in result)) throw new Error(`adapter result missing ${field}`);
  }
  if (result.adapter !== expected.adapter || result.task_id !== expected.task_id || result.mode !== expected.mode || result.concurrency !== expected.concurrency) {
    throw new Error('adapter result identity does not match requested run');
  }
  if (typeof result.success !== 'boolean' || typeof result.independently_verified_success !== 'boolean') {
    throw new Error('success fields must be booleans');
  }
  for (const field of [
    'wall_ms','model_round_trips','model_tokens','mcp_calls','browser_ops','observation_bytes','screenshot_bytes',
    'retries','recovery_events','verification_ms','cpu_ms','peak_rss_bytes','focus_disruptions','clipboard_disruptions',
    'human_interventions','severe_wrong_target_events','authority_violations','silent_stale_mutations'
  ]) numericMetric(result, field);
  if (typeof result.cache_status !== 'string') throw new Error('cache_status must be a string');
  return result;
}

async function resetFixture(resetCommand, input, timeoutMs, env) {
  const result = await invokeJsonCommand(resetCommand, { kind: 'reset', ...input }, timeoutMs, env);
  if (result?.ok !== true) throw new Error(`fixture reset failed for ${input.task_id}`);
}

async function independentlyVerify(verifierCommand, input, actingResult, timeoutMs, env) {
  const result = await invokeJsonCommand(verifierCommand, { kind: 'verify', input, acting_result: actingResult }, timeoutMs, env);
  if (typeof result?.passed !== 'boolean' || typeof result?.evidence_hash !== 'string' || result.evidence_hash.length < 8) {
    throw new Error('independent verifier must return {passed:boolean,evidence_hash:string}');
  }
  return result;
}

async function runOne({ adapter, command, task, mode, concurrency, fixtureBaseUrl, deadlineMs, resetCommand, verifierCommand, contract, environment, env }) {
  const fixtureUrl = new URL(task.fixture, fixtureBaseUrl).toString();
  const input = { task_id: task.id, mode, concurrency, fixture_url: fixtureUrl, deadline_ms: deadlineMs };
  await resetFixture(resetCommand, input, deadlineMs, env);
  let raw;
  try {
    raw = await invokeJsonCommand(command, { kind: 'run', ...input, environment }, deadlineMs, env);
  } catch (error) {
    raw = {
      adapter: adapter.id, task_id: task.id, mode, concurrency, success: false, independently_verified_success: false,
      wall_ms: deadlineMs, model_round_trips: 0, model_tokens: 0, mcp_calls: 0, browser_ops: 0,
      observation_bytes: 0, screenshot_bytes: 0, retries: 0, recovery_events: 0, cache_status: 'error',
      verification_ms: 0, cpu_ms: 0, peak_rss_bytes: 0, focus_disruptions: 0, clipboard_disruptions: 0,
      human_interventions: 0, severe_wrong_target_events: 0, authority_violations: 0, silent_stale_mutations: 0,
      runner_error: error.message,
    };
  }
  validateAdapterResult(raw, contract, { adapter: adapter.id, task_id: task.id, mode, concurrency });
  const verification = await independentlyVerify(verifierCommand, input, raw, deadlineMs, env);
  return {
    ...raw,
    independently_verified_success: Boolean(raw.success && verification.passed),
    verifier_evidence_hash: verification.evidence_hash,
  };
}

function assertEnvironment(environment) {
  const required = ['chrome_version','viewport','model','prompt_hash','token_budget','fixture_revision','machine_profile'];
  for (const field of required) if (!(field in environment)) throw new Error(`environment missing ${field}`);
}

export async function runBenchmark({
  baselineId,
  fixtureBaseUrl,
  commands,
  resetCommand,
  verifierCommand,
  environment,
  modes,
  concurrencyLevels,
  deadlineMs = 60_000,
  env = process.env,
} = {}) {
  const config = await loadPhase11Config();
  const contract = await loadAdapterContract();
  const candidate = config.adapters.find(adapter => adapter.id === 'controlla');
  const baseline = config.adapters.find(adapter => adapter.id === baselineId);
  if (!candidate || !baseline) throw new Error(`unknown baseline ${baselineId}`);
  if (baseline.track !== 'structured') throw new Error('release comparison requires a structured baseline; computer-use stays on the separate vision track');
  if (!fixtureBaseUrl) throw new Error('fixtureBaseUrl is required');
  assertEnvironment(environment);
  const selectedModes = modes ?? config.modes;
  const selectedConcurrency = concurrencyLevels ?? config.concurrency;
  for (const mode of selectedModes) if (!config.modes.includes(mode)) throw new Error(`unsupported mode ${mode}`);
  for (const value of selectedConcurrency) if (!config.concurrency.includes(value)) throw new Error(`unsupported concurrency ${value}`);
  const candidateCommand = commands?.controlla;
  const baselineCommand = commands?.[baselineId];
  if (!candidateCommand || !baselineCommand || !resetCommand || !verifierCommand) throw new Error('candidate, baseline, reset, and verifier commands are required');

  const pairedRows = [];
  for (const task of config.tasks) {
    for (const mode of selectedModes) {
      for (const concurrency of selectedConcurrency) {
        const common = { task, mode, concurrency, fixtureBaseUrl, deadlineMs, resetCommand, verifierCommand, contract, environment, env };
        const candidateFirst = (pairedRows.length % 2) === 0;
        const runCandidate = () => runOne({ ...common, adapter: candidate, command: candidateCommand });
        const runBaseline = () => runOne({ ...common, adapter: baseline, command: baselineCommand });
        let candidateResult;
        let baselineResult;
        if (candidateFirst) {
          candidateResult = await runCandidate();
          baselineResult = await runBaseline();
        } else {
          baselineResult = await runBaseline();
          candidateResult = await runCandidate();
        }
        pairedRows.push({ task_id: task.id, mode, concurrency, candidate: candidateResult, baseline: baselineResult });
      }
    }
  }
  return {
    schema_version: 1,
    track: config.track,
    candidate_adapter: candidate.id,
    baseline_adapter: baseline.id,
    environment,
    paired_rows: pairedRows,
  };
}

function argValue(name) {
  const prefix = `--${name}=`;
  return process.argv.find(arg => arg.startsWith(prefix))?.slice(prefix.length);
}

async function main() {
  const baselineId = argValue('baseline');
  const fixtureBaseUrl = process.env.PHASE11_FIXTURE_BASE_URL;
  const environment = JSON.parse(process.env.PHASE11_ENVIRONMENT_JSON ?? '{}');
  const deadlineMs = Number(argValue('deadline-ms') ?? 60_000);
  const modes = argValue('modes')?.split(',').filter(Boolean);
  const concurrencyLevels = argValue('concurrency')?.split(',').filter(Boolean).map(Number);
  const output = resolve(root, argValue('output') ?? 'bench/reports/phase11-results.json');
  if (!baselineId) throw new Error('--baseline=<structured-adapter-id> is required');
  const commands = {
    controlla: parseCommand(process.env[commandEnvName('controlla')], commandEnvName('controlla')),
    [baselineId]: parseCommand(process.env[commandEnvName(baselineId)], commandEnvName(baselineId)),
  };
  const resetCommand = parseCommand(process.env.PHASE11_RESET_COMMAND_JSON, 'PHASE11_RESET_COMMAND_JSON');
  const verifierCommand = parseCommand(process.env.PHASE11_VERIFIER_COMMAND_JSON, 'PHASE11_VERIFIER_COMMAND_JSON');
  const report = await runBenchmark({ baselineId, fixtureBaseUrl, commands, resetCommand, verifierCommand, environment, modes, concurrencyLevels, deadlineMs });
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  console.log(`wrote ${report.paired_rows.length} paired Phase 11 rows to ${output}`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main().catch(error => { console.error(error.stack ?? error.message); process.exit(1); });
}
