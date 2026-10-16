# Chrome Execution Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Reduce avoidable MCP-to-extension round trips and repeated guarded reads while preserving Controlla's existing identity, deadline, mutation, and unknown-delivery safety contracts.

**Architecture:** Add one shared identity/interaction read envelope to the native transport, reuse it within each guarded interaction operation, and keep mutations and postcondition verification ordered. Fill and accessibility retain their dedicated identity-only reads because they do not carry the interaction-epoch guard and combining them would change their established fallback/error boundaries. Add bounded extension-side parallelism only for independent read inventory, plus telemetry and regressions that expose MCP calls, host messages, and CDP commands separately.

**Tech Stack:** Rust/Tokio, Chrome DevTools Protocol, Manifest V3 service worker, Node.js extension harness, existing telemetry and smoke suites.

**Spec:** In-chat approved optimization design from 2026-10-15, grounded in Chrome debugger/service-worker and CDP Runtime/Input documentation.

## Global Constraints

- Never reuse identity, loader, transform, or interaction state across MCP calls.
- Never combine mutation dispatch with an unverified postcondition.
- Every read and dispatch remains bounded by the operation deadline.
- Unknown delivery is terminal until the caller inspects fresh state.
- MV3 worker state remains restart-safe; no permanent heartbeat is added.

## Review Focus

- Navigation between coalesced identity and epoch reads must refuse dispatch.
- A timeout in the combined read must not send a mutation.
- Replacement or transform drift must invalidate the per-operation cache.
- Extension-side parallel reads must not race target attach/detach or mutate state.
- Telemetry must distinguish MCP calls, host messages, and CDP commands.

---

### Task 1: Coalesced identity and interaction reads

**Files:**
- Modify: `crates/controlla-runtime/src/mcp.rs`
- Modify: `crates/controlla-browser/src/providers.rs`
- Test: `crates/controlla-runtime/src/mcp.rs`

**Interfaces:**
- Add `PageSessionTransport::command_batch` use through a helper returning `(frame_identity, interaction_epoch)`.
- Preserve existing `shared_frame_identity` and `shared_interaction_epoch` behavior for callers that need one read.

- [ ] Add tests for one combined envelope, timeout refusal, and document drift.
- [ ] Implement the helper with the existing bounded batch protocol and deadline.
- [ ] Replace repeated paired reads in guarded action paths.
- [ ] Run focused Rust tests.

### Task 2: Per-operation read reuse and invalidation

**Files:**
- Modify: `crates/controlla-runtime/src/mcp.rs`
- Test: `crates/controlla-runtime/src/mcp.rs`

**Interfaces:**
- Add an operation-local read record carrying identity, interaction epoch, and optional viewport/frame topology.

- [ ] Add tests showing reuse within one operation and refusal after identity/loader/transform drift.
- [x] Thread the record through click, swipe, scroll, and pinch guarded paths; fill and accessibility remain explicitly out of scope because they have no interaction-epoch guard to coalesce without changing their contracts.
- [ ] Keep cross-call state and mutation boundaries unchanged.
- [ ] Run focused Rust tests.

### Task 3: Safe extension read parallelism and telemetry

**Files:**
- Modify: `extensions/chrome-controlla/background.js`
- Modify: `extensions/chrome-controlla/test-background.cjs`
- Modify: `crates/controlla-browser/src/telemetry.rs`
- Modify: docs/verification-matrix.md

**Interfaces:**
- Independent frame inventory calls may use `Promise.all`; mutation and topology checks remain ordered.
- Telemetry retains separate `mcp_calls`, `host_round_trips`, `cdp_commands`, and verification polls.

- [ ] Add extension harness coverage for parallel independent reads and detach fencing.
- [ ] Implement only independent read parallelism.
- [ ] Add telemetry assertions for the separate counters.
- [ ] Run extension and telemetry tests.

### Task 4: Full local verification

**Files:**
- No production files beyond fixes discovered by tests.

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p controlla-runtime --locked --offline`.
- [ ] Run `cargo clippy -p controlla-runtime --all-targets --locked --offline -- -D warnings`.
- [ ] Run the extension, docs, provenance, clients, V3, apps, and agent-UX checks.
- [ ] Record measured local command/round-trip evidence without claiming live qualification.
