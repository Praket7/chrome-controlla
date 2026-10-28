#!/usr/bin/env node
// Installed-Chrome application qualification.
//
// This runner never invents evidence. It validates the task matrix, then reads
// operator-produced evidence rows and reports which capabilities may be
// claimed. Without `--evidence` every row is reported as unqualified, which is
// the truthful default for a checkout that has not run the live matrix.
import { readFile } from 'node:fs/promises';
import process from 'node:process';

const TASK_FILE = new URL('../tasks/phase12-installed-apps.json', import.meta.url);

export function validateMatrix(matrix) {
  const failures = [];
  if (matrix?.version !== 1 || matrix?.kind !== 'phase12_installed_apps') {
    failures.push('matrix must declare version 1 and kind phase12_installed_apps');
  }
  const rows = Array.isArray(matrix?.rows) ? matrix.rows : [];
  if (rows.length === 0) failures.push('matrix needs at least one row');
  const ids = new Set();
  for (const row of rows) {
    if (!row || typeof row.id !== 'string' || row.id.length === 0) {
      failures.push('every row needs a bounded id');
      continue;
    }
    if (ids.has(row.id)) failures.push(`duplicate row id ${row.id}`);
    ids.add(row.id);
    for (const field of ['application', 'capability', 'task']) {
      if (typeof row[field] !== 'string' || row[field].trim().length === 0) {
        failures.push(`${row.id} needs ${field}`);
      }
    }
    if (!row.assertions || Object.keys(row.assertions).length === 0) {
      failures.push(`${row.id} needs at least one correctness assertion`);
    }
    if (!['fixture', 'qualification'].includes(row.evidence)) {
      failures.push(`${row.id} evidence must be fixture or qualification`);
    }
  }
  const matrixCompetitors = Array.isArray(matrix?.competitors) ? matrix.competitors : [];
  if (matrixCompetitors.length < 2) failures.push('matched competitor list is required');
  return { pass: failures.length === 0, failures, rows };
}

export function evaluateEvidence(matrix, evidence) {
  const { pass: matrixValid, failures: matrixFailures, rows } = validateMatrix(matrix);
  const failures = [...matrixFailures];
  const reported = new Map();
  if (evidence !== undefined) {
    if (evidence?.version !== 1 || evidence?.kind !== 'phase12_evidence') {
      failures.push('evidence must declare version 1 and kind phase12_evidence');
    }
    for (const result of Array.isArray(evidence?.results) ? evidence.results : []) {
      const row = rows.find((candidate) => candidate.id === result?.id);
      if (!row) {
        failures.push(`evidence references unknown row ${result?.id}`);
        continue;
      }
      const complete =
        result.application_accepted === true &&
        result.client_completed === true &&
        typeof result.evidence_hash === 'string' &&
        result.evidence_hash.length >= 8 &&
        typeof result.controlla_version === 'string' &&
        result.controlla_version.length > 0 &&
        typeof result.recorded_at_ms === 'number' &&
        Number.isFinite(result.recorded_at_ms) &&
        result.assertions_passed === true;
      reported.set(row.id, complete);
      if (!complete) failures.push(`row ${row.id} evidence is incomplete`);
    }
  }
  const claims = Object.fromEntries(
    rows.map((row) => [row.id, { capability: row.capability, application: row.application,
      qualified: reported.get(row.id) === true }]),
  );
  const anyQualified = Object.values(claims).some((claim) => claim.qualified);
  return {
    pass: matrixValid && failures.length === 0,
    failures,
    claims,
    summary: {
      rows: rows.length,
      qualified: anyQualified ? Object.values(claims).filter((claim) => claim.qualified).length : 0,
      unqualified: rows.length - (anyQualified ? Object.values(claims).filter((claim) => claim.qualified).length : 0),
      superiority_claims_allowed: false,
    },
  };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const args = process.argv.slice(2);
  const evidenceIndex = args.indexOf('--evidence');
  const matrix = JSON.parse(await readFile(TASK_FILE, 'utf8'));
  const evidence = evidenceIndex === -1 ? undefined : JSON.parse(await readFile(args[evidenceIndex + 1], 'utf8'));
  const result = evaluateEvidence(matrix, evidence);
  console.log(JSON.stringify(result, null, 2));
  if (!result.pass) {
    console.error(`Installed-app qualification blocked:\n- ${result.failures.join('\n- ')}`);
    process.exit(1);
  }
  if (args.includes('--require-qualified') && result.summary.unqualified > 0) {
    console.error('Installed-app qualification is not complete; capability claims stay blocked.');
    process.exit(1);
  }
  console.log('Installed-app matrix validated. Unqualified rows stay unclaimed.');
}
