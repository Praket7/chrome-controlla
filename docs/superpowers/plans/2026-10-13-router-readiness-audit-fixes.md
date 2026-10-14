# Controlla Router and Readiness Audit Fixes

> **For agentic workers:** Implement this plan inline and use test-driven development. Each behavioral change starts with a failing regression test.

**Goal:** Make readiness waits cheaper, clean up failed cold opens, align freshness guidance, and ensure learned routing uses honest, useful latency data across task and one-call workflows.

**Architecture:** Keep current guarded browser behavior and one-call goal API. Replace repeated readiness polling with one bounded browser-side wait and final confirmation; route and record the browser workflow path using execution evidence without claiming prompt-to-outcome coverage that only client traces can provide; make the cost estimator statistically named and profile eviction recency-aware.

**Tech Stack:** Rust, Tokio, Chrome DevTools Protocol, Node.js MCP proxy, Markdown instructions.

**Spec:** User-requested fixes for the six findings in the preceding full code audit.

## Global Constraints

- Preserve guarded-mode target, document, freshness, and mutation verification.
- Do not add model-visible tools or discovery steps.
- Do not treat browser-runtime timing as prompt-send timing.
- Keep browser-side waits bounded and cancelable.

## Review Focus

- Readiness timeout, navigation, and selector errors leave no orphan cold session.
- A background tab remains inactive while readiness and workflow execution proceed.
- Cost history cannot be biased by unverified/unsafe outcomes or mislabeled statistics.
- Runtime and skill freshness rules describe the same evidence hierarchy.

### Task 1: Readiness and cold-open cleanup

**Files:** `crates/controlla-runtime/src/mcp/v3_full.rs`; focused tests in the existing V3 MCP test module.

- [x] Add failing coverage for a single browser-side bounded readiness wait and for cold-session cleanup when readiness returns an error.
- [x] Implement a browser-side event wait for selector/semantic readiness with timeout, navigation cancellation, and one final independent readback.
- [x] Release a cold session on readiness errors; preserve the existing incomplete receipt on an ordinary timeout.

### Task 2: Honest cost learning across browser workflows

**Files:** `crates/controlla-runtime/src/execution_cost_router.rs`, `crates/controlla-runtime/src/mcp/v3_full.rs`, `crates/controlla-runtime/src/observation_router.rs`.

- [x] Add failing tests for a statistically honest latency estimator and recency-based bounded profile retention.
- [x] Replace the small-sample maximum with a nearest-rank p90 estimate and a 16-sample verified/clean cohort gate.
- [x] Record and expose route timing for verified `browser action=open` workflows; keep it explicitly separate from client prompt-to-outcome traces and use only route choices with valid comparable evidence.
- [x] Replace lexicographic eviction with recency-based eviction for observation and execution-cost histories.

### Task 3: Align freshness contract

**Files:** `crates/controlla-runtime/src/mcp/v3_full.rs`, `packages/chrome-controlla/plugin/skills/browser-control/SKILL.md`.

- [x] Add/update contract coverage for structured listing timestamps, linked article metadata, and uniquely ranking coarse relative-age evidence.
- [x] Make runtime schema/server instructions match the skill's safe evidence order and unresolved/tied behavior.

### Task 4: Verification

- [x] Run targeted Rust and Node suites for changed code.
- [x] Run formatting, workspace tests, and relevant package checks; inspect every result.
- [x] Ask a fresh Luna Medium reviewer to assess the diff and test evidence against all audit findings.
