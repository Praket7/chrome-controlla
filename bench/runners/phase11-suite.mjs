import { appendFile, mkdir, writeFile } from 'node:fs/promises';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadPhase11Config, MAX_REPORT_BYTES, parseCommand, runBenchmark } from './phase11-runner.mjs';

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
  executionModes,
  concurrencyLevels,
  repetitions,
  retainRows = true,
  maxReportBytes = MAX_REPORT_BYTES,
  onComparison,
  onRow,
  deadlineMs = 60_000,
  env = process.env,
  configPath = process.env.PHASE11_TASK_SET ?? 'bench/tasks/phase11-realistic.json',
} = {}) {
  const config = await loadPhase11Config(configPath);
  const baselines = baselineIds ?? await structuredBaselineIds(candidateId, configPath);
  const comparisons = [];
  let retainedBytes = 0;
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
      executionModes,
      concurrencyLevels,
      repetitions,
      maxReportBytes: retainRows ? maxReportBytes - retainedBytes : maxReportBytes,
      onRow: onRow ? row => onRow(baselineId, row) : undefined,
      deadlineMs,
      env,
      configPath,
    });
    const evidence = { baseline_adapter: baselineId, warmup_rows: report.warmup_rows,
      paired_rows: report.paired_rows, randomization: report.randomization };
    if (retainRows) {
      retainedBytes += Buffer.byteLength(JSON.stringify(evidence));
      if (retainedBytes > maxReportBytes) throw new Error(`full-suite report exceeded ${maxReportBytes} bytes`);
      comparisons.push(evidence);
    }
    else {
      if (typeof onComparison !== 'function') throw new Error('onComparison is required when retainRows is false');
      if (!/^[a-z0-9][a-z0-9-]{0,63}$/i.test(baselineId)) throw new Error(`unsafe comparison adapter id ${baselineId}`);
      comparisons.push({ baseline_adapter: baselineId, evidence_file: await onComparison(evidence),
        randomization: report.randomization });
    }
  }
  const result = {
    schema_version: retainRows ? 2 : 3,
    track: config.track,
    candidate_adapter: candidateId,
    randomization: { method: 'seeded_paired_order', seed: config.random_seed },
    ...(retainRows ? {} : { results_storage: 'comparison_files' }),
    environment,
    comparisons,
  };
  if (retainRows && Buffer.byteLength(JSON.stringify(result)) > maxReportBytes) {
    throw new Error(`full-suite report exceeded ${maxReportBytes} bytes`);
  }
  return result;
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
  const executionModes = argValue('execution-modes')?.split(',').filter(Boolean);
  const concurrencyLevels = argValue('concurrency')?.split(',').filter(Boolean).map(Number);
  const repetitions = argValue('repetitions') === undefined ? undefined : Number(argValue('repetitions'));
  const stamp = new Date().toISOString().replaceAll(/[:.]/g, '-');
  const output = resolve(root, argValue('output') ?? `bench/reports/phase11-suite-${config.track}-${stamp}.json`);
  const candidateId = process.env.PHASE11_CANDIDATE_ID ?? 'controlla';
  const baselines = argValue('baseline-ids')?.split(',').map(value => value.trim()).filter(Boolean)
    ?? (process.env.PHASE11_BASELINE_IDS
      ? process.env.PHASE11_BASELINE_IDS.split(',').map(value => value.trim()).filter(Boolean)
      : await structuredBaselineIds(candidateId, configPath));
  const commands = {
    [candidateId]: parseCommand(process.env[commandEnvName(candidateId)], commandEnvName(candidateId)),
  };
  for (const baselineId of baselines) {
    commands[baselineId] = parseCommand(process.env[commandEnvName(baselineId)], commandEnvName(baselineId));
  }
  const resetCommand = parseCommand(process.env.PHASE11_RESET_COMMAND_JSON, 'PHASE11_RESET_COMMAND_JSON');
  const verifierCommand = parseCommand(process.env.PHASE11_VERIFIER_COMMAND_JSON, 'PHASE11_VERIFIER_COMMAND_JSON');
  const comparisonDir = `${output}.comparisons`;
  const checkpoint = `${output}.partial.jsonl`;
  await mkdir(dirname(output), { recursive: true });
  await writeFile(checkpoint, `${JSON.stringify({status:'running',candidate_adapter:candidateId,baselines,environment,config,modes,executionModes,concurrencyLevels,repetitions})}\n`, {flag:'wx'});
  const report = await runFullSuite({ candidateId, baselineIds: baselines, fixtureBaseUrl, commands, resetCommand, verifierCommand,
    environment, modes, executionModes, concurrencyLevels, repetitions, deadlineMs, configPath, retainRows: false,
    onRow: async (baseline, row) => appendFile(checkpoint, `${JSON.stringify({baseline,row})}\n`),
    onComparison: async evidence => {
      await mkdir(comparisonDir, { recursive: true });
      const filename = `${evidence.baseline_adapter}.json`;
      const destination = resolve(comparisonDir, filename);
      const shard = `${JSON.stringify({ schema_version: 2, track: config.track, candidate_adapter: candidateId,
        environment, randomization: evidence.randomization, ...evidence })}\n`;
      if (Buffer.byteLength(shard) > MAX_REPORT_BYTES) throw new Error(`comparison shard exceeded ${MAX_REPORT_BYTES} bytes`);
      await writeFile(destination, shard, { flag: 'wx' });
      return relative(root, destination).replaceAll('\\', '/');
    } });
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  await appendFile(checkpoint, `${JSON.stringify({status:'completed'})}\n`);
  console.log(`wrote a full Phase 11 evidence manifest for ${report.comparisons.length} structured baselines to ${output}`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main().catch(error => { console.error(error.stack ?? error.message); process.exit(1); });
}
