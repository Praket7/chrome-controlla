# Verified Replay Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make repeated Controlla tasks faster and more reliable through verified, non-secret replay with one bounded self-healing fallback.

**Architecture:** Extend the existing page action memory with a bounded replay record and explicit cache outcome. Reacquire targets against the fresh page identity before dispatch; use existing batch dispatch and independent verification. Invalidate stale entries and fall back once to the current fresh path.

**Tech Stack:** Rust, serde_json, existing Controlla V3 MCP runtime, existing action memory and CDP batch transport.

**Spec:** `docs/superpowers/specs/2026-10-16-verified-replay-cache-design.md`

## Global Constraints

- Cache entries contain no passwords, chat credentials, or secret values.
- Cache replay must require fresh document identity and independent postcondition verification.
- Unknown effects are terminal for replay and must not be retried blindly.
- Existing response, lease, checkpoint, and artifact contracts remain compatible.
- All cache records and metrics remain bounded.

## Review Focus

- Structural drift between cache write and replay must invalidate the entry and use the fresh path.
- A target that reacquires ambiguously must never dispatch.
- Unknown or partially acknowledged effects must not be cached or replayed.
- Secure-fill inputs from chat must never reach persistence or metrics.
- Cache hit metrics must not claim a hit when fresh observation was required.

### Task 1: Replay record contract

**Files:**
- Modify: `crates/controlla-runtime/src/page_action_memory.rs`
- Test: `crates/controlla-runtime/src/page_action_memory.rs`

**Interfaces:**
- Add a bounded replay-plan field to `ActionMemoryEntry` with serde compatibility.
- Add cache outcome helpers for hit, stale, invalidation, and bounded persistence.

- [x] Write failing tests for valid replay records, bounded serialization, secret rejection, and stale invalidation.
- [x] Run the focused page-action-memory tests and confirm the new assertions fail.
- [x] Implement the minimal record and validation changes.
- [x] Run the focused tests and confirm they pass.

### Task 2: V3 replay admission and self-healing

**Files:**
- Modify: `crates/controlla-runtime/src/mcp/v3_full.rs`
- Test: `crates/controlla-runtime/src/mcp/v3_full.rs`

**Interfaces:**
- Add a bounded replay lookup result carrying cache status and the verified semantic target.
- Reuse `current_skill_context_core`, `reference_reacquire`, existing command batches, and existing verification predicates.

- [x] Add replay admission tests for hit, structural miss, and stale invalidation; existing unknown-effect tests remain the terminal boundary.
- [x] Run the focused tests and confirm the new assertions fail before implementation.
- [x] Implement replay admission for the read-only extraction path, with fresh identity matching, one safe fallback, and explicit invalidation. Existing qualified mutation skills remain the mutation replay path.
- [x] Run focused tests. Live headless qualification is intentionally deferred to the user's reload/restart step.

### Task 3: Metrics and benchmark evidence

**Files:**
- Modify: `crates/controlla-runtime/src/mcp/v3_full.rs`
- Modify: `bench/runners/phase11-public-readonly-pilot.mjs` only if required by the existing schema
- Test: existing runtime and benchmark suites

- [x] Add regression coverage for truthful cache hit/miss/stale/healed state and secret exclusion.
- [x] Run it red, then implement bounded metrics.
- [x] Run the focused runtime test, full cargo suite, benchmark contract suite, and release gate. The previously committed public pilot remains the latest live evidence.

### Task 4: Review and publish

- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo test -p controlla-runtime --all-targets --locked --offline`.
- [x] Run `cargo build --locked --release -p controlla-runtime --bins`.
- [x] Run the local smoke, benchmark, release, documentation, and provenance checks. Live Chrome/client reload qualification remains outside this pass.
- [x] Review the diff for secret persistence and cache-bound violations.
- [ ] Commit and push `HEAD:main`; verify `origin/main` equals the commit.
