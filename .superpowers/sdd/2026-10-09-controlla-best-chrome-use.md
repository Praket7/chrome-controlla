# Controlla Best Chrome Use — SDD Ledger

Starting point: `62e6407d0e75b83a1be8d69468476601a6e6da37`

## Completion state

- Persistent shared-Chrome session continuity and explicit selected-tab authority are preserved.
- Compact semantic snapshots, revision-bound short refs, changed-only deltas, and guarded reference resolution are implemented.
- Verified bulk fill/select and strict retained-node typing/press paths are implemented with stale-document checks and unknown-delivery semantics.
- Deterministic MCP v2 workflow actions cover snapshot, navigate, find, click, fill, type, press, select, wait, observe, extract, assert, verify, and checkpoint. Shared-route scripts fail closed.
- Runtime-owned verification is required for qualification-sensitive skill outcomes. Caller-supplied verification evidence is not accepted.
- Persistent reusable skill records support candidate, qualification, replay, drift quarantine, expiry, and runtime verification.
- Targeted visual probes capture bounded element/selector/region PNG crops and discard stale captures after document identity changes.
- Agent-facing MCP v2 exposes session, snapshot, find, act, extract, workflow, probe, verify, and skill tools while retaining v1 compatibility.
- ChatGPT Desktop/local MCP integration and benchmark/release documentation are present.
- One-time patch transport used during implementation removed itself from the tree. The source was formatted before the final verification run.
- The strict typing path has also been normalized for the Rust 1.99 clippy gate before this final matrix run.

## Safety invariants

- Never silently broaden selected-tab authority.
- Never treat stale semantic refs as valid mutations.
- Never retry an unknown mutation outcome without fresh observation.
- Never accept caller-authored evidence as independent verification.
- Prefer semantic state first and bounded visual probes only when semantics are insufficient.
- Public superiority claims remain gated on reproducible benchmark evidence rather than implementation status alone.

## Verification gate

The final branch is only considered complete after the repository CI matrix passes on Linux, macOS, and Windows, including rustfmt, clippy with `-D warnings`, workspace tests, package checks, client/app/benchmark/release checks, extension tests, and dependency/package validation.
