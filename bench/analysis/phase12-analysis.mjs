#!/usr/bin/env node

export const REQUIRED_COMPETITORS = [
  'controlla-v3',
  'stagehand-v4',
  'playwright-mcp',
  'browser-use',
  'chrome-devtools-mcp',
  'agent-browser',
];

export function analyzePhase12(rows) {
  if (!Array.isArray(rows) || rows.length === 0) {
    return { claimEligible: false, reasons: ['no raw benchmark rows'], summary: {} };
  }
  const reasons = [];
  const realRows = rows.filter((row) => row.evidence_kind === 'real' && row.reproducible === true);
  const competitors = new Set(realRows.map((row) => row.competitor));
  for (const competitor of REQUIRED_COMPETITORS) {
    if (!competitors.has(competitor)) reasons.push(`missing real reproducible evidence for ${competitor}`);
  }
  const byCompetitor = new Map();
  for (const row of realRows) {
    const list = byCompetitor.get(row.competitor) ?? [];
    list.push(row);
    byCompetitor.set(row.competitor, list);
  }
  const summary = {};
  for (const [competitor, list] of byCompetitor) {
    const successes = list.filter((row) => row.success === true).length;
    const verified = list.filter((row) => row.verified_success === true).length;
    const severe = list.reduce((sum, row) => sum + Number(row.severe_safety_failures ?? 0), 0);
    const wall = list.map((row) => Number(row.wall_ms)).filter(Number.isFinite).sort((a, b) => a - b);
    const browserTrips = list.map((row) => Number(row.browser_round_trips)).filter(Number.isFinite);
    summary[competitor] = {
      runs: list.length,
      successRate: successes / list.length,
      verifiedSuccessRate: verified / list.length,
      severeSafetyFailures: severe,
      medianWallMs: wall.length ? wall[Math.floor(wall.length / 2)] : null,
      meanBrowserRoundTrips: browserTrips.length ? browserTrips.reduce((a, b) => a + b, 0) / browserTrips.length : null,
    };
  }
  const controlla = summary['controlla-v3'];
  if (!controlla) reasons.push('missing controlla-v3 evidence');
  if (controlla?.severeSafetyFailures > 0) reasons.push('controlla-v3 has severe safety failures');
  if (controlla && controlla.verifiedSuccessRate < 0.98) reasons.push('controlla-v3 verified success is below 98%');
  return { claimEligible: reasons.length === 0, reasons, summary };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  const rows = JSON.parse(Buffer.concat(chunks).toString('utf8') || '[]');
  process.stdout.write(`${JSON.stringify(analyzePhase12(rows), null, 2)}\n`);
}
