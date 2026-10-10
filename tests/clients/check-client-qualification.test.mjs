import assert from 'node:assert/strict';
import test from 'node:test';
import { loadClientQualification, validateClientQualification } from '../../apps/check-client-qualification.mjs';

test('qualification checker binds every client row to its checked-in adapter', async () => {
  const loaded = await loadClientQualification();
  validateClientQualification(loaded.matrix, loaded.adapters);

  const bad = structuredClone(loaded.adapters);
  bad.freebuff.serverId = 'other-server';
  assert.throws(() => validateClientQualification(loaded.matrix, bad), /freebuff.*server id/i);
});

test('qualification checker rejects adapter command drift and missing required evidence dimensions', async () => {
  const loaded = await loadClientQualification();
  const wrongCommand = structuredClone(loaded.adapters);
  wrongCommand.opencode.command = ['/other/controlla', 'mcp'];
  assert.throws(() => validateClientQualification(loaded.matrix, wrongCommand), /opencode.*command/i);

  const missingEvidence = structuredClone(loaded.matrix);
  missingEvidence.requiredCapabilities = missingEvidence.requiredCapabilities.filter((name) => name !== 'verified_mutations');
  assert.throws(() => validateClientQualification(missingEvidence, loaded.adapters), /verified_mutations/);

  const missingClientEvidence = structuredClone(loaded.matrix);
  missingClientEvidence.clients.codex.requiredEvidenceCategories.pop();
  assert.throws(() => validateClientQualification(missingClientEvidence, loaded.adapters), /codex.*required capability evidence categories/i);
});
