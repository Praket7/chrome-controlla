import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { evaluateEvidence, validateMatrix } from './phase12-installed-apps.mjs';

const matrix = JSON.parse(
  await readFile(new URL('../tasks/phase12-installed-apps.json', import.meta.url), 'utf8'),
);

test('shipped matrix validates without invented evidence', () => {
  const result = validateMatrix(matrix);
  assert.equal(result.pass, true, result.failures.join('\n'));
  assert.ok(result.rows.length >= 10);
  const claims = evaluateEvidence(matrix);
  assert.equal(claims.pass, true, claims.failures.join('\n'));
  assert.equal(claims.summary.qualified, 0);
  assert.equal(claims.summary.unqualified, result.rows.length);
  assert.equal(claims.summary.superiority_claims_allowed, false);
});

test('incomplete or unknown evidence cannot qualify a capability', () => {
  const unknown = evaluateEvidence(matrix, {
    version: 1,
    kind: 'phase12_evidence',
    results: [{ id: 'not-a-row', application_accepted: true }],
  });
  assert.equal(unknown.pass, false);
  assert.match(unknown.failures.join('\n'), /unknown row/);

  const incomplete = evaluateEvidence(matrix, {
    version: 1,
    kind: 'phase12_evidence',
    results: [{ id: 'google-docs-type-1000', application_accepted: true }],
  });
  assert.equal(incomplete.pass, false);
  assert.equal(incomplete.claims['google-docs-type-1000'].qualified, false);
  assert.match(incomplete.failures.join('\n'), /incomplete/);
});

test('a complete row qualifies only itself', () => {
  const result = evaluateEvidence(matrix, {
    version: 1,
    kind: 'phase12_evidence',
    results: [
      {
        id: 'gmail-compose',
        application_accepted: true,
        client_completed: true,
        assertions_passed: true,
        evidence_hash: 'sha256:abc12345',
        controlla_version: '3.0.0',
        recorded_at_ms: 1,
      },
    ],
  });
  assert.equal(result.pass, true, result.failures.join('\n'));
  assert.equal(result.claims['gmail-compose'].qualified, true);
  assert.equal(result.claims['word-web-type-1000'].qualified, false);
  assert.equal(result.summary.qualified, 1);
  assert.equal(result.summary.superiority_claims_allowed, false);
});
