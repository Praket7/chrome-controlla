import assert from 'node:assert/strict';
import test from 'node:test';
import { loadClientQualification, validateClientQualification } from '../../apps/check-client-qualification.mjs';

test('qualification checker binds every client row to its checked-in adapter', async () => {
  const loaded = await loadClientQualification();
  validateClientQualification(loaded.matrix, loaded.adapters, loaded.integrationConfigs);

  const bad = structuredClone(loaded.adapters);
  bad.freebuff.serverId = 'other-server';
  assert.throws(() => validateClientQualification(loaded.matrix, bad, loaded.integrationConfigs), /freebuff.*server id/i);
});

test('qualification checker rejects config drift and missing required evidence dimensions', async () => {
  const loaded = await loadClientQualification();
  const wrongConfig = structuredClone(loaded.matrix);
  wrongConfig.clients.opencode.integrationConfig = 'integrations/other.json';
  assert.throws(() => validateClientQualification(wrongConfig, loaded.adapters, loaded.integrationConfigs), /opencode.*checked-in integration config/i);

  const missingEvidence = structuredClone(loaded.matrix);
  missingEvidence.requiredCapabilities = missingEvidence.requiredCapabilities.filter((name) => name !== 'verified_mutations');
  assert.throws(() => validateClientQualification(missingEvidence, loaded.adapters, loaded.integrationConfigs), /verified_mutations/);

  const missingClientEvidence = structuredClone(loaded.matrix);
  missingClientEvidence.clients.codex.requiredEvidenceCategories.pop();
  assert.throws(() => validateClientQualification(missingClientEvidence, loaded.adapters, loaded.integrationConfigs), /codex.*required capability evidence categories/i);
});
