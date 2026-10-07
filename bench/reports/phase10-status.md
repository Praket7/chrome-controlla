# Phase 10 — controlled comparison status

Captured 2026-10-07. Candidate package versions and artifact integrity values are in [`../baselines/versions.lock.json`](../baselines/versions.lock.json). These are reproducibility inputs only; no baseline package was installed or executed.

## Preregistered primary outcomes

- **Task success:** all required target/state predicates pass independent readback; otherwise fail or inconclusive, never infer success from dispatch.
- **Non-inferiority:** Controlla's task-clustered success difference versus each baseline must have a lower confidence bound above -2 percentage points.
- **Efficiency targets:** at least 30% fewer model round trips and 25% lower median wall time on the held-out set. Report targets as unmet unless measured.
- **Safety:** any severe wrong-target effect or authority violation blocks a win regardless of efficiency.
- **Cost accounting:** include cold setup, observation bytes, model/MCP/browser calls, retries, verification, recovery, intervention, clipboard/focus disruption, cleanup, and cache preparation.

## Required study before comparison claims

1. Freeze supported versions/configs and a 30-template pilot spanning generic browser tasks, extraction, interference, multi-tab, and design tasks. Estimate variance and disclose the design-review sample size.
2. Before opening held-out tasks, commit the split, task rubric, excluded cases, model/config matching, reset procedure, primary outcomes, and analysis script.
3. Run 100 held-out task templates × 5 independent resets and 20 critical workflows × 10 resets. Preserve immutable per-run manifests and failures. Expand if power is insufficient.
4. Compute task-clustered intervals, success non-inferiority, p50/p95, all costs above, and AGWC ablations. Keep unavailable products as missing tracks.

## Evidence status

| Work | State | Evidence |
|---|---|---|
| Offline task fixtures | Verified locally | All 30 pilot templates bind task-specific initial-state resets, bounded setup/actions, and independent state predicate readback. Synthetic interference cases verify stale writes are withheld in the fixture harness. They do not verify candidate-reported outputs, app behavior, or visual quality. `npm run check:bench` validates these fixtures; no candidate, browser, or model ran. |
| Candidate package versions | Partial | Exact package versions and registry integrity values are recorded in the baseline lock. Source revisions are recorded where locally available; Stagehand source commit is still unresolved. None of the baselines is installed or tested. |
| Protocol/config parity | Not run | No baseline packages installed/configured |
| 30-template pilot manifest | Preregistered and structurally validated | `../tasks/phase10-pilot.json` defines 30 templates (six per category), three planned repetitions, reset/predicate fields, seeded candidate-order randomization, outcomes, analysis rules, and the no-results boundary; `node --test bench/phase10-pilot.test.mjs` and `node scripts/check-phase10-pilot.mjs` pass. No task has been run. |
| Pilot execution | Not run | No candidate configuration was run. The exact model ID/version is intentionally unresolved in the frozen held-out config, and no baseline was installed. Add a dated preregistration addendum binding the same exact model/config and candidate versions before any pilot run; do not infer model identity from this fixture suite. |
| 500 held-out runs | Not run | No run data |
| 200 critical-workflow runs | Not run | No run data |
| AGWC ablations / statistical report | Not run | Depends on valid runs |
| Comparison claim | Prohibited | No results support one |

Local revalidation on 2026-10-07 passed `npm run check:bench` (12 tests, frozen 100-template/20-critical split, deterministic Phase 6 fixture), `node scripts/check-phase10-pilot.mjs` (30 templates, five categories), and the full app/client/docs/provenance/package checks. These results validate manifests and harnesses only; no browser/model task or comparison was run.

Phase 10 is **not complete**. Each pilot template now has a runnable *offline state fixture*, but these do not execute candidate tools, Chrome, or model tasks and are not pilot evidence. The held-out IDs and split digest are frozen before any pilot outcome. The 30-template pilot, 100-template × 5-reset comparison, 20 critical workflows × 10-reset runs, and AGWC ablations remain unrun. A preregistration addendum must freeze the exact model ID/version and any changed baseline pins before execution.
