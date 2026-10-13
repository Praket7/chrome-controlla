import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadPhase11Config, parseCommand, runBenchmark } from './phase11-runner.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');

function commandEnvName(adapterId) {
  return `PHASE11_${adapterId.toUpperCase().replace(/[^A-Z0-9]+/g, '_')}_COMMAND_JSON`;
}

export async function structuredBaselineIds(candidateId = 'controlla', configPath = process.env.PHASE11_TASK_SET ?? 'bench/tasks/phase11-realistic.json') {
  const config = await loadPhase11Config(configPath);
  const selected = config.default_baseline_ids ? new Set(config.default_baseline_ids) : null;
  return config.adapters
    .filter(adapter => adapter.track === 'structured' && adapter.id !== candidateId && (!selected || selected.has(adapter.id)))
    .map(adapter => adapter.id);
}

export async function runFullSuite({
  candidateId = 'controlla',
  baselineIds,
  fixtureBaseUrl,
  commands,
  resetCommand,
  verifierCommand,
  environment,
  modes,
  concurrencyLevels,
  repetitions,
  deadlineMs = 60_000,
  env = process.env,
  configPath = process.env.PHASE11_TASK_SET ?? 'bench/tasks/phase11-realistic.json',
} = {}) {
  const config = await loadPhase11Config(configPath);
  const baselines = baselineIds ?? await structuredBaselineIds(candidateId, configPath);
  const comparisons = [];
  for (const baselineId of baselines) {
    if (!commands?.[baselineId]) throw new Error(`missing command for structured baseline ${baselineId}`);
    const report = await runBenchmark({
      baselineId,
      candidateId,
      fixtureBaseUrl,
      commands: { [candidateId]: commands[candidateId], [baselineId]: commands[baselineId] },
      resetCommand,
      verifierCommand,
      environment,
      modes,
      concurrencyLevels,
      repetitions,
      deadlineMs,
      env,
      configPath,
    });
    comparisons.push({ baseline_adapter: baselineId, paired_rows: report.paired_rows });
  }
  return {
    schema_version: 2,
    track: config.track,
    candidate_adapter: candidateId,
    environment,
    comparisons,
  };
}

function argValue(name) {
  const prefix = `--${name}=`;
  return process.argv.find(arg => arg.startsWith(prefix))?.slice(prefix.length);
}

async function main() {
  const fixtureBaseUrl = process.env.PHASE11_FIXTURE_BASE_URL;
  const configPath = argValue('task-set') ?? process.env.PHASE11_TASK_SET ?? 'bench/tasks/phase11-realistic.json';
  const config = await loadPhase11Config(configPath);
  const environment = JSON.parse(process.env.PHASE11_ENVIRONMENT_JSON ?? '{}');
  const deadlineMs = Number(argValue('deadline-ms') ?? 60_000);
  const modes = argValue('modes')?.split(',').filter(Boolean);
  const concurrencyLevels = argValue('concurrency')?.split(',').filter(Boolean).map(Number);
  const repetitions = argValue('repetitions') === undefined ? undefined : Number(argValue('repetitions'));
  const stamp = new Date().toISOString().replaceAll(/[:.]/g, '-');
  const output = resolve(root, argValue('output') ?? `bench/reports/phase11-suite-${config.track}-${stamp}.json`);
  const candidateId = process.env.PHASE11_CANDIDATE_ID ?? 'controlla';
  const baselines = process.env.PHASE11_BASELINE_IDS
    ? process.env.PHASE11_BASELINE_IDS.split(',').map(value => value.trim()).filter(Boolean)
    : await structuredBaselineIds(candidateId, configPath);
  const commands = {
    [candidateId]: parseCommand(process.env[commandEnvName(candidateId)], commandEnvName(candidateId)),
  };
  for (const baselineId of baselines) {
    commands[baselineId] = parseCommand(process.env[commandEnvName(baselineId)], commandEnvName(baselineId));
  }
  const resetCommand = parseCommand(process.env.PHASE11_RESET_COMMAND_JSON, 'PHASE11_RESET_COMMAND_JSON');
  const verifierCommand = parseCommand(process.env.PHASE11_VERIFIER_COMMAND_JSON, 'PHASE11_VERIFIER_COMMAND_JSON');
  const report = await runFullSuite({ candidateId, fixtureBaseUrl, commands, resetCommand, verifierCommand, environment, modes, concurrencyLevels, repetitions, deadlineMs, configPath });
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  const rows = report.comparisons.reduce((sum, comparison) => sum + comparison.paired_rows.length, 0);
  console.log(`wrote ${rows} paired Phase 11 rows across ${report.comparisons.length} structured baselines to ${output}`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main().catch(error => { console.error(error.stack ?? error.message); process.exit(1); });
}
