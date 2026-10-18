# Taboom-Derived Controlla Baseline

**Checkout:** `codex/comprehensive-chrome-audit` at `2998b7bd4683c3f15763ab086f7f2faae9f125bc`  
**Captured:** 2026-10-18  
**Status:** local tests passed; live ESPN and caller-latency telemetry not run

## Automated baseline

- `cargo test --workspace --all-targets`: 876 passed, 0 failed, 26 ignored across 45 reported test binaries/suites. The full output is preserved in the plan workspace as `task-1-baseline-cargo.txt`.
- `node packages/chrome-controlla/test-agent-ux.cjs`: 21 passed, 0 failed.
- ESPN/Bills agent UX fixture: 1 external `browser` call, 1 internal workflow call, 0 schema-discovery calls, 1 targeted snapshot, 0 homepage detours, 0 repository reads required. The workflow includes snapshot, find_all, choose, click, and verify in one open request.
- Live ESPN Rust tests: 3 tests are ignored because they require the loaded local Chrome bridge and a paired Bills tab. No live browser result is claimed.

## Metrics not available from the baseline fixture

- ESPN browser-command count, CDP-command count, host round trips, and observation bytes are runtime receipt fields, but the plugin UX fixture does not run the MCP runtime and therefore cannot measure them.
- Prompt-send-to-verified-result latency includes external model work that the MCP runtime cannot observe. The local fixture does not supply a client trace, so this remains unmeasured.
- Task 8 adds the end-to-end benchmark needed to gather these values. Until then, these fields are `not measured`, not zero.

## Scope decisions

- The live ESPN test now asserts that the four deterministic workflow steps are returned in the single `browser open` response. It remains ignored unless the bridge is explicitly available.
- Native envelope fixtures cover valid identity, stale visual state, unowned session, unavailable capability, acknowledged delivery, uncertain delivery, malformed input, and oversize refusal. The behavior assertions for malformed and oversize rejection belong to Task 5, where the production decoder and size bounds are introduced.
