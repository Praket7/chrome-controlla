# Phase 10 — controlled comparison status

Captured 2026-10-06. Candidate package versions and artifact integrity values are in [`../baselines/versions.lock.json`](../baselines/versions.lock.json). These are reproducibility inputs only; no baseline package was installed or executed.

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
| Offline fixture contract | Verified locally | `node --test bench/analysis/offline-task-harness.test.mjs` passes clone-based reset, bounded state operations, and independent predicate-readback fixtures. This is contract validation only; it is not pilot execution or browser/model comparison evidence. |
| Candidate package versions | Partial | Registry artifacts and most source commits/tags pinned; Stagehand source commit remains unavailable from package metadata |
| Protocol/config parity | Not run | No baseline packages installed/configured |
| 30-template pilot manifest | Preregistered and structurally validated | `../tasks/phase10-pilot.json` defines 30 templates (six per category), three planned repetitions, reset/predicate fields, seeded candidate-order randomization, outcomes, analysis rules, and the no-results boundary; `node --test bench/phase10-pilot.test.mjs` and `node scripts/check-phase10-pilot.mjs` pass. No task has been run. |
| Pilot execution | Not run | The offline harness is not connected to pilot task-specific resets/predicates; candidate configuration parity is not frozen, and no baseline was installed |
| 500 held-out runs | Not run | No run data |
| 200 critical-workflow runs | Not run | No run data |
| AGWC ablations / statistical report | Not run | Depends on valid runs |
| Comparison claim | Prohibited | No results support one |

Phase 10 is **not complete**. The pilot manifest is a reproducible preregistration artifact, not a runnable harness or evidence. Held-out IDs and split digest must be frozen before inspecting pilot outcomes; the 100-template × 5-reset comparison and 20 critical workflows × 10 resets remain unrun.
