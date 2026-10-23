# Performance and qualification implementation plan

> Use superpowers:subagent-driven-development or superpowers:executing-plans task by task. User authorized implementation of the review on October 23.

**Goal:** Implement the review's concrete efficiency improvements and make qualification reproducible without relaxing identity or outcome verification.
**Architecture:** Preserve the six MCP tools and existing Rust/native-extension transport. Reduce work at collection and operation boundaries; keep correctness-critical state synchronous in ordering but off async worker threads. Add independently checked fixture state and machine-readable qualification evidence.
**Tech stack:** Existing pinned Rust 1.99.0, Node 24.19.0, Chrome, no unpinned dependencies.
**Spec:** [repository review](../../performance-review-2026-10-23.md) (review accepted by the user's instruction to make all fixes and test).

## Global constraints
- Chrome/web apps only; six public tools remain stable.
- Preserve stale identity, account, navigation, cancellation, and unknown-mutation protections.
- Preserve extracted-code provenance; update its hashes if affected.
- Preserve failed evidence. Live application/client qualification may be reported only from actual runs.
- No account credentials or personal browser profiles needed for disposable fixture tests.

## Review focus
- A browser must not forge independent success through an event endpoint.
- A truncated snapshot must not assert unique/exhaustive results.
- Navigation during a batched operation must stop mutation or return unknown.
- Storage failures and cancellation must not reorder write-ahead mutation state.
- Build/browser changes must not reuse stale routing costs as qualified evidence.

## Tasks
1. Independent fixture acceptance: add actual server-side form/upload state, privileged reset/readback contract with run binding and bounded inputs; reject fabricated events. Keep unsupported scenarios unqualified. Add negative and positive verifier tests.
2. Observation efficiency: propagate bounded collection budgets, cache presentation geometry within a pass, keep completeness metadata; add collector and Rust budget regressions.
3. Guarded transport efficiency: identify and combine redundant read-only identity/epoch/readback exchanges using existing command batching, preserving final mutation guards. Add transport-count and drift tests.
4. Runtime efficiency: move synchronous checkpoint and DAG persistence work to blocking workers with bounded ownership; reduce polling/contended waits where safely supported; expire/version cost history. Add storage/concurrency regressions.
5. Capability qualification: add reproducible fixture qualification and WebMCP compatibility tests, explicit observed/unavailable evidence. Run isolated Chrome cases if supported; never fabricate client/account acceptance.
6. Maintainability and CI: extract a coherent bounded subsystem from the large dispatcher, Rust caching, PR cancellation, separate deterministic/current Chrome lanes, avoid repeated platform-independent checks while retaining package tests.
7. Integration: run relevant Node and Rust suites, format/lint, package and provenance checks; update progress/verification/blockers. Independent whole-branch review and fixes; leave reviewable branch and patch.

For each behavioral task: add a regression, observe failure, implement, run focused tests, then broader suites. Record commands and limitations in `docs/performance-implementation-2026-10-23.md`.

## Decisions
The existing review supplies the design and authorization; no additional approval round is needed. Installed personal Chrome/account/client measurements cannot be substituted with headless fixture results. Refactoring is incremental to avoid a wholesale rewrite of the 29k-line V3 file.
