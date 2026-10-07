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
| Candidate package versions | Partial | Package registry queries and artifact hashes captured; source commits remain open |
| Protocol/config parity | Not run | No baseline packages installed/configured |
| 30-template pilot | Not run | Harness, prompts, and reset fixture absent |
| 500 held-out runs | Not run | No run data |
| 200 critical-workflow runs | Not run | No run data |
| AGWC ablations / statistical report | Not run | Depends on valid runs |
| Comparison claim | Prohibited | No results support one |

Phase 10 is **not complete**. This report deliberately records the minimum preregistration and pins gathered, but does not convert missing experiments into a pass.
