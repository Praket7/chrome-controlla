import test from 'node:test';
import assert from 'node:assert/strict';
import { analyzePhase12, REQUIRED_COMPETITORS } from './phase12-analysis.mjs';

function row(competitor, overrides = {}) {
  return {
    competitor,
    evidence_kind: 'real',
    reproducible: true,
    success: true,
    verified_success: true,
    severe_safety_failures: 0,
    wall_ms: 100,
    browser_round_trips: 4,
    ...overrides,
  };
}

test('generated rows can never unlock the public claim gate', () => {
  const rows = REQUIRED_COMPETITORS.map((competitor) => row(competitor, { evidence_kind: 'generated' }));
  const result = analyzePhase12(rows);
  assert.equal(result.claimEligible, false);
  assert.ok(result.reasons.some((reason) => reason.includes('missing real reproducible evidence')));
});

test('complete real evidence with safe controlla results is eligible', () => {
  const rows = REQUIRED_COMPETITORS.flatMap((competitor) =>
    Array.from({ length: 50 }, (_, index) => row(competitor, { wall_ms: 100 + index })),
  );
  const result = analyzePhase12(rows);
  assert.equal(result.claimEligible, true);
  assert.equal(result.summary['controlla-v3'].verifiedSuccessRate, 1);
});

test('one severe controlla safety failure blocks eligibility', () => {
  const rows = REQUIRED_COMPETITORS.flatMap((competitor) =>
    Array.from({ length: 50 }, (_, index) =>
      row(competitor, competitor === 'controlla-v3' && index === 0 ? { severe_safety_failures: 1 } : {}),
    ),
  );
  const result = analyzePhase12(rows);
  assert.equal(result.claimEligible, false);
  assert.ok(result.reasons.includes('controlla-v3 has severe safety failures'));
});
