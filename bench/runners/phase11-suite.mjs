import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadPhase11Config, parseCommand, runBenchmark } from './phase11-runner.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');

function commandEnvName(adapterId) {
  return `PHASE11_${adapterId.toUpperCase().replace(/[^A-Z0-9]+/g, '_')}_COMMAND_JSON`;
}

export async function structuredBaselineIds() {
  const config = await loadPhase11Config();
  return config.adapters
    .filter(adapter => adapter.track === 'structured' && adapter.id !== 'controlla')
    .map(adapter => adapter.id);
}

export async function runFullSuite({
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
  const baselines = await structuredBaselineIds();
  const comparisons = [];
  for (const baselineId of baselines) {
    if (!commands?.[baselineId]) throw new Error(`missing command for structured baseline ${baselineId}`);
    const report = await runBenchmark({
      baselineId,
      fixtureBaseUrl,
      commands: { controlla: commands.controlla, [baselineId]: commands[baselineId] },
      resetCommand,
      verifierCommand,
      environment,
      modes,
      concurrencyLevels,
      deadlineMs,
      env,
    });
    comparisons.push({ baseline_adapter: baselineId, paired_rows: report.paired_rows });
  }
  return {
    schema_version: 2,
    track: 'phase11-realistic',
    candidate_adapter: 'controlla',
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
  const environment = JSON.parse(process.env.PHASE11_ENVIRONMENT_JSON ?? '{}');
  const deadlineMs = Number(argValue('deadline-ms') ?? 60_000);
  const modes = argValue('modes')?.split(',').filter(Boolean);
  const concurrencyLevels = argValue('concurrency')?.split(',').filter(Boolean).map(Number);
  const output = resolve(root, argValue('output') ?? 'bench/reports/phase11-results.json');
  const baselines = await structuredBaselineIds();
  const commands = {
    controlla: parseCommand(process.env[commandEnvName('controlla')], commandEnvName('controlla')),
  };
  for (const baselineId of baselines) {
    commands[baselineId] = parseCommand(process.env[commandEnvName(baselineId)], commandEnvName(baselineId));
  }
  const resetCommand = parseCommand(process.env.PHASE11_RESET_COMMAND_JSON, 'PHASE11_RESET_COMMAND_JSON');
  const verifierCommand = parseCommand(process.env.PHASE11_VERIFIER_COMMAND_JSON, 'PHASE11_VERIFIER_COMMAND_JSON');
  const report = await runFullSuite({ fixtureBaseUrl, commands, resetCommand, verifierCommand, environment, modes, concurrencyLevels, deadlineMs });
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  const rows = report.comparisons.reduce((sum, comparison) => sum + comparison.paired_rows.length, 0);
  console.log(`wrote ${rows} paired Phase 11 rows across ${report.comparisons.length} structured baselines to ${output}`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main().catch(error => { console.error(error.stack ?? error.message); process.exit(1); });
}
