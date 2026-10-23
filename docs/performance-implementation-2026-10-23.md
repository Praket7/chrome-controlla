# Performance implementation ledger — 2026-10-23

Base: 34924239529bdc6bf2cfa389d933a0bb75d828a4. Branch: codex/performance-and-qualification.
Plan: docs/superpowers/plans/2026-10-23-performance.md.

- Baseline: 31 benchmark-analysis/V3 release-gate tests passed.
- Ruling: use an isolated clone because the earlier review directory was a hash-verified source snapshot without Git history.
- Ruling: retain live account/client qualification as an explicit external gate unless actual environment access is available; fixture results never imply it passed.

## Implemented changes

- Independent disposable-fixture acceptance for basic forms and byte uploads. Privileged reset/readback, UUID attempts, server-computed upload hashes, persistence-before-acknowledgement, actor environment isolation, malformed-input handling, and unsupported-scenario refusal.
- Snapshot collection follows the caller response budget subject to the collector's text/metadata reserve. One geometry read per candidate; retained action guards still read fresh geometry.
- Click and retained text-action preflight use the existing ordered identity/epoch batch. Final mutation guards and uncertain-delivery handling remain intact.
- Ordered checkpoint blocking worker and awaited write-ahead calls; owned store guards survive caller cancellation. DAG history persistence runs on blocking workers. Observation telemetry has a bounded writer and nonblocking read fallback.
- Lease releases notify in-process waiters; 25 ms reconciliation remains for other processes. Lease admission moved into `mcp/target_leases.rs`.
- Timing histories bind runtime/extension sources, dependency lock, toolchain, target and build flags. Route, observation and DAG learning is scoped to accepted sessions; observation/DAG samples now expire after 30 days, matching route history.
- Native Chromium WebMCP supports JSON-text schemas and JSON-text invocation inputs. Page programs moved into separately testable JS files.
- CI caches Rust builds, cancels obsolete PR runs, consolidates platform-independent checks, and runs pinned Chrome plus stable compatibility lanes. Browser test paths are configurable with CONTROLLA_CHROME.

## Review and decisions

- Independent review: `/workspace/controlla-review-feedback.md`, gpt-6.1-sol; no Astra agents used.
- Fixed review finding: queued malformed form JSON no longer terminates the HTTP fixture service; regression returns 400 and verifies service/readback remains available.
- Fixed review finding: costs cannot qualify across accepted sessions. Ruling: use conservative session isolation until authoritative browser-build attestation is available; this trades cross-session timing reuse for correct invalidation on re-pair/restart. It does not claim an observed browser version.
- Added exact native WebMCP callback-input assertion after review.
- Ruling: preserve the native host's pairing/inventory polling fallback. Cross-platform event IPC and wholesale session-lock redesign require separate lifecycle evidence; notification replaces avoidable in-process lease waiting without weakening the existing recovery path.
- Ruling: preserve route weights and minimum clean sample thresholds. Real client/model telemetry and matched competitor task measurements are still required to calibrate them; no heuristic speedup is fabricated.

## Supporting evidence

- Baseline benchmark-analysis/release gate: 31 passing.
- Earlier unrestricted full Rust baseline: all default suites passed (see retained execution log).
- Snapshot regression: geometry reads changed from four to two for two controls; bounded candidate coverage remains partial.
- Real Chromium DOM plus snapshot plus native WebMCP: 12 passing, zero skipped, Chromium 154.0.8037.57. WebMCP testing flag was enabled.
- Shared preflight on real disposable Chromium with simulated extension transport: passing. This checks ordered read batching and denied-action guards, not installed-extension mutation delivery.
- Checkpoint worker: responsiveness, exact write-ahead state and cancellation/ordering tests.
- Oracle HTTP/pure/environment tests: 6 passing. Benchmark/runner suites: 77 passing in the initial broad run.
- Agent UX: 25 passing. Release structure: 16 passing; superiority claim stays disabled.
- Rust compile and production Clippy passed. Integrated suite outcomes are recorded below.

## Retained failed evidence and external limits

- Earlier restricted tests failed due to listener/process permissions; later unrestricted tests passed.
- The first complete Rust rerun exposed a timing-sensitive existing unknown-hold test using a fixed 40 ms sleep. It now waits for the actual bounded-release notification.
- The older ignored shared full click fixture still fails because its simulated CDP transport forwards extension-only pointer commands directly to CDP. The new preflight fixture is scoped to the path it genuinely exercises. No installed personal Chrome extension or account-backed app qualification is claimed.
- The isolated headless stress run initially timed out because managed Chromium blocks data: fixture URLs. Replacing the disposable fixture with loopback HTTP resolved the cause; all eight launches now pass run-specific AX/title, fresh same-origin localStorage and independent cleanup checks. Earlier failed logs are retained.
- Live clients, user accounts, application persistence beyond the disposable fixture, a representative 30-run end-to-end task matrix, prompt submission/delivery hooks and matched competitor results remain external qualification gates.

## Final validation evidence

- `cargo test --workspace --locked --offline`: 898 passed, 0 failed, 8 ignored across 51 reported suites. Ignored browser cases are qualified separately, not counted as passing by omission.
- `CONTROLLA_CHROME=/usr/bin/chromium node --test scripts/test-background-actions.mjs scripts/test-snapshot-efficiency.mjs scripts/test-webmcp-browser.mjs`: 12 passed, 0 failed, 0 skipped.
- `cargo test -p controlla-browser --test headless_stress -- --ignored --nocapture` with the configured Chromium: 1 passed; eight launches, separate profiles, exact fixture identity and storage isolation.
- `cargo test -p controlla-runtime --bin controlla-v2 shared_preflight_batches_identity_and_epoch_real_chrome -- --ignored --nocapture`: 1 passed in real Chromium using the simulated read-only bridge.
- Oracle HTTP/state/environment suite: 6 passed. Agent UX: 25 passed. Client configuration/trace suite: 17 passed. Final expanded benchmark/runner suite: 82 passed, 0 failed, 0 skipped.
- Production Clippy, formatting, dependency boundary, documentation links and extraction provenance checks passed. Six packaged client MCP launch contracts passed; these are startup contracts, not behavioral client qualification. Release package build and package checks passed on Linux/x64: Cargo archives, npm archive contents/target/attribution, clean-prefix install, command shim, direct/plugin six-tool startup, install/upgrade/rollback/uninstall and user-data preservation. These are local package checks; hosted cross-platform CI remains unrun.

## Collector microbenchmark

Thirty paired measured samples after four warmups, Chromium 154.0.8037.57, 50,000-node synthetic DOM with 500 controls. Baseline commit 34924239529bdc6bf2cfa389d933a0bb75d828a4 versus candidate, alternating order. Both variants selected Target 499 and reported partial coverage in every measured run. Baseline collector budget was 100 KB; candidate control-only budget was 12 KiB.

| Metric | Baseline | Candidate |
|---|---:|---:|
| Median collector time | 2160.2 ms | 2076.2 ms |
| p95 collector time | 2835.2 ms | 2573.0 ms |
| Geometry reads | 1000 | 500 |
| Collected JSON characters (ASCII fixture) | 100894 | 11000 |
| Verified selections | 30/30 | 30/30 |

This is a local collector microbenchmark, not prompt-to-verified task latency, a mixed-task non-inferiority result, or a competitor comparison. Raw local artifact: `/workspace/controlla-collector-benchmark.json`. The unchanged control traversal remains a substantial cost on this large DOM.
