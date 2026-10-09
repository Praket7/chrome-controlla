#!/usr/bin/env node
import { readFile } from 'node:fs/promises';

export function evaluateV3Gate(evidence) {
  const failures = [];
  const require = (condition, message) => { if (!condition) failures.push(message); };
  const requireNonnegative = (value, label) => {
    require(typeof value === 'number' && Number.isFinite(value) && value >= 0,
      `${label} must be a finite nonnegative number`);
    return typeof value === 'number' && Number.isFinite(value) && value >= 0;
  };
  require(evidence.browser_mode_parity === true, 'foreground/background/headless parity missing');
  require(evidence.fast_keys_fixed_delay_ms === 0, 'FastKeys still has a fixed per-character delay');
  if (requireNonnegative(evidence.block_typing_protocol_calls_per_1000_chars, 'block typing protocol calls')) {
    require(evidence.block_typing_protocol_calls_per_1000_chars <= 4, 'Block typing is not effectively constant protocol work');
  }
  require(evidence.interference_wrong_target_mutations === 0, 'user-interference wrong-target mutation observed');
  require(evidence.multi_client_wrong_target_mutations === 0, 'multi-client wrong-target mutation observed');
  require(evidence.unknown_mutation_auto_retries === 0, 'unknown mutation delivery was retried automatically');
  require(evidence.default_tool_count === 6, 'default agent tool surface is not six tools');
  if (requireNonnegative(evidence.default_schema_bytes, 'default schema bytes')) {
    require(evidence.default_schema_bytes <= 48 * 1024, 'default agent schema exceeds 48KiB');
  }
  require(evidence.client_contracts_qualified === true, 'supported client contracts are not qualified');
  require(evidence.headless_recovery_bounded === true, 'headless reconnect is not bounded');
  require(evidence.page_tools_fail_closed === true, 'page tool route is not fail-closed');
  require(evidence.severe_safety_failures === 0, 'severe safety failure present');
  require(evidence.real_competitor_evidence_complete === true, 'real controlled competitor evidence incomplete');
  require(evidence.generated_rows_used_for_best_claim !== true, 'generated evidence cannot unlock a best claim');
  return { pass: failures.length === 0, failures };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const path = process.argv[2];
  if (!path) {
    console.error('usage: node scripts/check-v3-release-gate.mjs <evidence.json>');
    process.exit(2);
  }
  const evidence = JSON.parse(await readFile(path, 'utf8'));
  const result = evaluateV3Gate(evidence);
  if (!result.pass) {
    console.error(`V3 release gate blocked:\n- ${result.failures.join('\n- ')}`);
    process.exit(1);
  }
  console.log('V3 release gate passed with complete real evidence.');
}
