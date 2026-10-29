# Execution-core plan status — 2026-10-28

Status against `2026-10-28-controlla-56970d9-audit-and-best-in-class-plan.md`.
"Verified" means a local command or a live browser run produced the stated
evidence. It never means a broader claim than the boundary in the same row.

## Implemented and verified

| Plan item | Status | Evidence | Boundary |
|---|---|---|---|
| P0.8 Reclassify guards: backend mismatch is not a boundary | done | `guard_reason.rs`, `action_recovery.rs`, `shared_page/guard.js`. `cargo test -p controlla-runtime --lib` (358 pass), `cargo test -p controlla-runtime --test guard_diagnostics -- --ignored` (7 Chrome-backed cases pass). | Classification and receipts. A route that needs a person (unauthorized secret entry) or a Chrome-owned surface stays terminal. |
| P0.10 `execution_first` policy | done at the guard contract | `ExecutionPolicy::{ExecutionFirst, Strict}` + `GuardContext`; live receipt showed `policy: "execution_first"`. | The policy is the default classification. It is not yet a request field on `act`/`browser`, so a caller cannot opt into `strict` over MCP yet. |
| P0.9 Make `not_dispatched` exceptional | partial | Receipts now carry `automatic_recovery`, `recovery.plan`, `recovery.backend`, and `model_action_required`; the V2 normalizer removes a contradictory `next_action` when recovery is automatic. Live receipt confirmed. | The runtime still does not execute every ladder rung before returning. Recovery execution exists as a bounded ledger plus receipt contract; wiring it into each dispatch site remains. |
| P0.11 SPA document identity | done | `SemanticDocumentKey::same_document`, `document_key`, `classify_invalidation`, `hard_boundary`, `reacquire_reference`. Unit tests plus a live probe: `@c1/@c2` stayed valid across `history.pushState` and the document receipt reported the new route. | Frame + loader identity. A real navigation still replaces the loader and invalidates references. |
| P0.12 Stable refs / no recycling | done for the reference namespace | Reference namespace is monotonic per paired tab across document replacement (`reset_semantic_document`), and duplicate controls are disambiguated by structural identity. `source_structure` and semantic-state tests cover the contract. | A control that cannot be distinguished still receives a new ref rather than reusing a retired one; it is not silently merged. |
| P0.13 Stable observation delta identity | done | `lib/observation.js` + `extensions/chrome-controlla/test-observation.mjs` (6 tests): duplicate role/name controls, occurrence fallback, document replacement, measured retention. | Node-level helper coverage. The production router consumes the same helper but full delta routing remains task-conditioned. |
| Observation metrics | done for the helper | `pruneSemanticItems` returns `required_total`, `required_retained`, `required_retention_rate`, `dropped_required`; the constant `recall_floor` was removed. | Retention is measured against caller-declared required roles, not a whole-page oracle. |
| P1 normal skill ≤ 4 KB | done | `packages/chrome-controlla/plugin/skills/browser-control/SKILL.md` is 4,091 bytes; detail moved to `references/advanced.md`; `source_structure::normal_agent_instructions_stay_short` enforces it. | Size and content contract only. |
| P0.1 Monolith decomposition (guard) | not done; ratchet added | `tests/source_structure.rs` pins `v3_full.rs` at 31,491 lines and `background.js` at 3,322 lines and forbids a second declaration of `GuardReason`, `RecoveryClass`, or `TaskStepKind`. | This prevents regrowth. It does not decompose either file. |

## Not started (staged)

These remain exactly as the audit describes them and must not be reported as
delivered:

- P0.1 actual decomposition of `v3_full.rs` and `background.js`.
- P0.2 one internal `TaskProgram` IR across task, workflow, skills, and learned
  operations.
- P0.3 public workflow control flow compiled to that IR.
- P0.4 target-adaptive `Auto` typing in the V3 facade (`typing::compile_transaction`
  still supplies fixture characteristics; the live V3 route uses `v3.rs`
  characteristics).
- P0.5–P0.7 Docs/Word/CodeMirror/Monaco qualification and a real Phase 12
  executor.
- P0.14 event-driven semantic mirror.
- P0.15 WebMCP-first routing ladder.
- P0.16–P0.18 state-machine operation memory, WALT-style promotion, and replay
  integration.
- P0.19 dispatch-scoped mutation leases.
- P0.20 automatic unique-tab binding.
- P0.21 empirical background-pointer capability matrix.
- P0.22 pointer/gesture parity (HTML5 drag, pinch, child-frame parity).
- P0.23 same-process execution-context tracking.
- Matched competitor benchmarks and any superiority claim.

## Verification commands

```text
cargo test --workspace --quiet
cargo clippy --workspace --all-targets --quiet -- -D warnings
cargo test -p controlla-runtime --test guard_diagnostics -- --ignored
cargo test -p controlla-runtime --test source_structure
npm run check:observation
npm run check:docs
node --test scripts/test-editor-rebind.mjs
```

Live checks used the debug `controlla-v2` build against a local page in
dedicated headless Chrome: verified click, `pointer_events_none` receipt with
`automatic_recovery: true`, `navigation`-style click verified after a
`history.pushState`, and reference retention across that route change.
