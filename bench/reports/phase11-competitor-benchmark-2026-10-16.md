# Phase 11 competitor benchmark

Generated from the fresh real-client run at:

- `bench/reports/phase11-public-readonly-pilot-2026-10-16T02-55-19-128Z.json`

## Evidence boundary

This is a measured six-adapter pilot, not a release qualification result. It used one public read-only task (`https://example.com/`), one replicate, isolated Chrome profiles, independent HTTPS verification, and cleanup checks. The runner explicitly records `release_claim_eligible: false` and the scope excludes the full Phase 11 matrix.

The complete Phase 11 matrix is defined in `bench/tasks/phase11-realistic.json`: 25 tasks × 4 site modes × 3 execution modes × 4 concurrency levels × 30 measured repetitions. The complete suite also needs an independently configured command argv for Controlla and each baseline, a reset command, a verifier command, and an environment JSON. Those `PHASE11_*` bindings are not present in this environment, so the full matrix cannot be honestly executed here. The mock runner was not used as competitor evidence.

## Fresh measured pilot

Environment: Google Chrome 155.0.8059.84, model `gpt-6-luna`.

| Adapter | Verified | Browser outcome (ms) | Independent verification (ms) | Final (ms) | Browser calls | Input tokens |
|---|---:|---:|---:|---:|---:|---:|
| Controlla | **no** | — | 15,092.94 | 14,123.46 | 1 | 92,144 |
| Playwright MCP 0.0.83 | yes | 18,140.56 | 23,833.83 | 22,960.63 | 3 | 106,346 |
| Chrome DevTools MCP 1.10.1 | yes | 14,148.81 | 20,050.68 | 19,242.98 | 3 | 153,774 |
| agent-browser 0.39.1 | yes | 12,452.77 | 16,723.23 | 15,854.92 | 3 | 106,501 |
| Browser Use 0.13.11 | yes | 10,254.93 | 15,207.19 | 14,162.73 | 3 | 52,596 |
| Stagehand 4.1.0 | yes | 7,503.19 | 9,527.80 | 8,660.99 | 1 | 53,458 |

### What the run proves

- Stagehand was fastest on this single task and used one browser call.
- Browser Use used the fewest input tokens among the passing baselines in this replicate.
- Controlla used one browser call, but did **not** pass the independently verified task: its compact task response returned receipts without the requested title and paragraph, so both browser-result and final-answer reference checks failed.
- The result is not a statistically meaningful speed or superiority ranking. One replicate cannot provide p50/p95, variance, confidence intervals, or a claim-gate decision.
- Exact model round trips were unavailable from the Codex JSON stream and are recorded as `null` by the runner.

## Release-gate status

**Not eligible.** The repository’s public claim requires complete independently verified structured competitor evidence, at least 30% model-round-trip reduction, at least 25% median wall-time reduction, and zero wrong-target, authority, stale-state, and false-success regressions. This pilot supplies none of the full-matrix comparisons needed to evaluate those gates.

## To run the complete measured matrix

Provide real command bindings and a fixture/verifier environment, then run `bench/runners/phase11-suite.mjs`. Required variables are:

- `PHASE11_CONTROLLA_COMMAND_JSON`
- one `PHASE11_<BASELINE>_COMMAND_JSON` for every baseline in `bench/tasks/phase11-realistic.json`
- `PHASE11_RESET_COMMAND_JSON`
- `PHASE11_VERIFIER_COMMAND_JSON`
- `PHASE11_ENVIRONMENT_JSON`

The suite should be run with a fixed checkout, pinned adapter identities, a dedicated fixture base URL, and the repository’s default 30 repetitions after two warmups. No synthetic/mock rows should be merged into release evidence.

## Harness repair

The configured runner now lives at `scripts/run-phase11-suite.mjs`. It starts `bench/fixtures/realistic/server.mjs`, binds `bench/adapters/phase11-command.mjs` for every structured adapter, and uses `phase11-reset.mjs` plus `phase11-verifier.mjs` for independent state reset/readback. It records all required environment identity fields and writes comparison shards through the existing suite runner.

The adapter smoke path is real Codex MCP execution. Unsupported foreground/background requests return explicit unsupported rows; they are never relabeled as headless measurements. The full release matrix still requires a headed/background display harness before those cohorts can qualify.
