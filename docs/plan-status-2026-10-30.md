# Execution plan status — 2026-10-30

Plan: `2026-10-30-controlla-best-in-class-chrome-execution-plan.md`.
This file records what actually landed. It does not claim any benchmark result,
and it does not claim the plan is complete.

## Implemented and covered by tests

| Plan item | State | Evidence |
|---|---|---|
| §4 window presentation leases | done | `lib/presentation-slots.js`: one VISIBLE owner per window, `NONE`/`VISIBLE`/`FOREGROUND`, epochs, token revalidation, stale takeover, `releaseWindow`. Unit tests `test-p0-resources.mjs`; concurrency case in `test-background.cjs` (second visible task gets another window; first tab stays presented). |
| §5 read-only reuse discovery | done | `findReusableOwnedBackgroundCandidate` proves identity and window eligibility without `tabs.update`. Harness asserts discovery changes neither `active` nor the activation counter. |
| §6 minimized-window eligibility | done | `windowPresentationEligibility` rejects focused, minimized, non-normal, incognito windows for both window and tab reuse; harness asserts a minimized owned window is skipped. |
| §7 owned browser resources | done | `lib/owned-resources.js` builds `tab → record`, `window → records`, `key → request`, exact-URL and logical-resource indexes, folds alias receipts into one record, and carries `TabDurability`. |
| §8 explicit reuse classes | done | `ReuseKind`: exact document, same-resource navigation, recycled owned tab, new tab existing window, new tab new window. Every open receipt reports `reuse_kind` plus current-call telemetry. |
| §9 logical-resource reuse | done for cold opens | `lib/resource-keys.js` names Docs/Sheets/Slides documents, GitHub issues/PRs, the repository landing page, and Gmail threads. Query strings are never stripped globally. Harness navigates a second Docs view in the same tab. Shared-session opens still reuse exact documents only. |
| §10 idle owned-tab recycling | done, opt-in | `recycle_idle_tabs: true` plus ephemeral durability, 60 s idle, unleased, completed receipt. Harness proves reuse without a new tab and that the default stays off. |
| §11 alias persistence on reuse | done | A new idempotency key that resolves to an existing tab writes a completed alias receipt before returning; harness asserts the receipt and an idempotent retry. |
| §12 shrink the open lock | done for the native cold-open path | Reservation (receipts, index, window slot, tab creation) runs under the state lock; navigation and readiness run outside it. Harness blocks one open's document read and proves a second open completes meanwhile. |
| §13 PresentationProof | done | `Controlla.presentationState` now returns `presentation_proof`, `presentation_epoch`, and the current `presentation_slot`; open receipts carry the Chrome-level proof with `visibility_deferred: true` until a session is paired. |
| §14 canvas visibility loss | done | A hidden unreadable canvas reacquires presentation and rechecks visibility; on failure it returns `not_dispatched` before input, or `unknown` with `canvas_presentation_lost` mid-transaction. It never appends a DOM-insert tail. The unknown receipt now reports only the prefix the page text proves landed (`landedPrefix`), so a queued key that never applied is not counted as delivered. Harness asserts no input command is sent; real Chrome (`check:transaction-background`) proves the stop, the resume-after-reacquire, and the corrected count. |
| §15 bulk text fast path | done, including Google Docs | `bulk_insert` (and `insert_text` on a rich editor) dispatches one `Input.insertText` for the whole block, then verifies through the editor's own value or the document's copy path. Four thousand characters complete in one insertion; `key_events` is zero. Playwright: 13/13 in `check:transaction-background`, including 1,000- and 4,000-character rich-editor cases, canvas present/refuse, unicode, and paragraph readback. Live installed Chrome (`scripts/live-bulk-docs.mjs`): a 1,027-character Docs block verified in 1,475 ms (696 chars/s) and a 4,027-character block in 1,595 ms (2,525 chars/s); both survived a same-tab reopen and were proven with a `text_present` predicate. The canvas editor is driven through its event-target frame and its own copy event; the DOM guard is not asked for a value it cannot hold. |
| §16 plain Unicode vs IME | done | `text_semantics` is `plain_unicode_insert` unless the caller explicitly selects `ime`; `stringNeedsComposition` no longer turns a non-ASCII string into an implicit composition transaction. The receipt carries `text_semantics` and `bulk_insert`. |
| §17–§22 unified task IR, operation contracts, cost-aware planning, scheduled reads, speculative reads, WebMCP-first routing | done | `execution_plan.rs` wraps task programs, browser workflows, skill workflows, task primitives, and learned extract operations in one `ExecutionPlan`/`OperationContract` envelope: effect, preconditions, postconditions, cleanup, verification, and a bounded budget. `for_task_program` returns the scheduled program and its plan; independent `observe`/`extract` runs of 4..=8 are grouped into `parallel_read`, while stateful or short runs stay serial. Task primitives attach the plan as `execution_route.execution_plan`; skill register/replay attach it; a learned extract hit attaches `learned_operation`. Empirical p95 selection is recorded through `with_empirical_route`, and a qualified read-only WebMCP site tool is preferred in the static read route, matching the executor's existing WebMCP-before-skill order. Speculative and WebMCP execution were already present in the runtime; this pass routes and receipts them through the shared plan instead of adding a second speculative scheduler. Fourteen `execution_plan` tests cover contracts, schedule bounds, learned operations, empirical routing, speculative routing, WebMCP-first selection, upload postcondition completeness, and fail-closed skill replay. |
| §23 operation memory evolution | done | `page_action_memory.rs` now stores bounded `OperationPlan`/`OperationStep` entries, promotes them only after two independent verified passes, invalidates on failure, and exposes promoted operations through `lookup_operation`, `has_promoted_operation`, and snapshot `page_action_hints`. `act` records extract, click, click-hint, fill-hint, type, and select-hint replay metadata; a completed task records one bounded operation under `operation:{intent}`. `cargo test -p controlla-runtime page_action_memory` covers promotion, staleness, secret rejection, and unbounded shapes. |
| §24 task-conditioned filtering | done | `task_observation.rs` derives query, required roles, and a receipt from the task primitive only. V3 task observations pass that filter as the snapshot goal, so fill/click/save/upload/verify steps retain textbox/button/link/alert roles before query ranking. The route receipt carries `task_conditioned_filter` with `derived_from_task_only: true`; tests assert no fill values enter the query. |
| §25 WALT URL promotion | done | `site_profiles.rs` promotes verified URLs from one query slot or one operation path slot, requires three distinct verified samples, keeps only `{query}` templates and slot metadata, and encodes path slots without allowing origin or path escape. A promoted profile is registered as an exact-origin, exact-structure, read-only `SiteProfile` capability. Tests cover path search, paginated paths, entity-path rejection, path encoding, and training-text exclusion. |
| §26 progressive observation | done | `progressive_observation.rs` evaluates a compact packet against truncation, scope, required roles, goal satisfaction, and status. A single bounded second stage widens item/text budgets when the caller did not set them explicitly; the receipt reports stages, reason, final mode, and `page_instructions_can_escalate: false`. Unit tests cover no-escalation, each escalation reason, and the one-stage bound. |
| §27 snapshot fencing | done for semantic fences | Every V3 snapshot returns `snapshot_fence` with snapshot id, semantic revision, document identity, exact tab binding, and a stale-reference contract. Existing reference rebinding still rejects a different document. The fence is deliberately kept out of compact act receipts so bounded action receipts do not lose proof fields. |
| §29–§30 shadow/OOPIF and pointer parity | done for deterministic coverage | `test-background.cjs` proves open-shadow composed hit-testing, a fresh open-shadow click in an attached frame, child-frame swipe phases through the owner quad, and a two-finger child-frame pinch whose mapped contacts open from 80 to 120 px. `Controlla.frameMultiTouch` now lives in `lib/frame-multitouch.js` with loader, session, origin-chain, owner-transform, and activation/focus identity rechecks immediately before dispatch; a stale loader refuses with `not_dispatched` and no input. PointerEngine coverage includes root pinch contact identity and release. | A live child-frame multi-touch case and the live Docs same-origin blank-frame candidate ordering still require the installed-app matrix; no application claim is made from the fixture. |
| §32 event-driven rebase classes | done | `reference_rebase.rs` classifies a lost reference once as stable, rebound, ambiguous, removed, or document-replaced and carries evidence plus `safe_for_mutation`; V3 stale-reference receipts expose `rebase.class`, `rebase.to`, and a typed `reason_code` instead of a generic stale error. The module's five tests cover unique replacement, document replacement, duplicate ambiguity, bounded maps, and value-free receipts. | The rebound reference is retried by the caller; the runtime does not silently dispatch to a different control. |
| §33 text-conflict semantics for fill/type | done | `text_conflict.rs` separates clean precondition, already-at-requested-value, human edit, and unreadable surfaces. Fill and type call the preflight before input: an existing requested value returns `verified` with no write, a third value returns `not_dispatched` with `same_field_human_edit_detected`, and rich/canvas/blank-frame surfaces fall through to their own proof. Receipts carry lengths and booleans, never values. | A target that exposes no trustworthy DOM value is not treated as permission to overwrite. |
| §34 focus emulation for background targets | done | `lib/focus-emulation.js` enables `Emulation.setFocusEmulationEnabled` lazily after identity/topology gates, skips document-only reads, disables it before debugger detach, and reports `focus_emulated` in presentation evidence. `test-p0-resources.mjs` proves the command classification and one-handshake/one-disable behavior. | Emulation makes focus handlers coherent; it does not activate the tab or raise the user's window. |
| §35 bounded owned-resource pool | done | `lib/owned-pool.js` evicts only idle ephemeral owned tabs that are neither leased, debugger-attached, active, nor presented, marks receipts `closed_reason: owned_pool_eviction`, and throws a typed refusal when no safe victim exists. `planPoolEviction` tests cover durability, lease, protection, freshness, and keep-set boundaries. | The limit is 64 live automation tabs; the receipt table still keeps bounded history. |
| §36–§38 one-call existing-tab tasks, automatic safe binding, cheap typed verification | done for read-only Research | `TaskPrimitive::Research` can bind one uniquely matching existing tab in the same `browser action=task` call: zero candidates returns `no_matching_tab`, several return `ambiguous` with bounded candidates, and one pairs, accepts, and releases the temporary session. Matching accepts an exact tab ID or the same HTTP(S) origin/path, never a guessed query-string view. Existing typed verification keeps canvas `text_present` on the document copy proof and returns typed conflict/stale receipts before dispatch. | Automatic binding is deliberately limited to read-only Research; a mutation still requires an explicit session and tab. |
| §28 semantic mirror | done for delta mirror | Snapshot responses carry `semantic_mirror` with the stable-identity delta, document-change flag, upsert/removed counts, and mutation clock. The mutation clock comes from the page-side `MutationObserver` in `shared_page/snapshot.js`; page text and values never enter mirror keys. |
| §40 decomposition | partial | The native cold-open lifecycle moved to `lib/owned-open.js`; the CDP script bodies moved to `lib/input-scripts.js` and the readback/pacing helpers to `lib/typing-support.js`. The canvas document-proof path moved to `mcp/v3_canvas_proof.rs`, and moving the route-signal helpers into `execution_plan.rs` brought `v3_full.rs` to 31,997 lines under its 32,000-line ratchet, with operation memory, automatic binding, and typed receipt helpers extracted to `v3_core.rs` (786 lines). `execution_plan.rs` is 1,277 lines under its 1,400-line bound. `lib/input.js` is 890 lines and its 900-line ratchet holds. `background.js` is at its 3,350-line ceiling after moving focus emulation, pool eviction, execution counters, navigation URL helpers, and child-frame multi-touch into `lib/`. |

## Correctness fixes landed alongside

- Evidence store retention no longer evicts the scope the current call just wrote. A
  saturated 128-scope store with a lexicographically-first scope previously
  discarded the caller's evidence and returned a handle that resolved to nothing.
- `scroll` accepts one axis; the omitted axis defaults to zero, and background
  page scroll no longer requires a target. A scroll that leaves the viewport
  unchanged reports `inconclusive`/`scroll_offset_unchanged` instead of
  `verified`, and the readback and bulk-insert proof survive the compact receipt
  projection.
- A canvas text predicate now falls back to the document's copy-event proof
  when the DOM cannot answer it. `verify {kind:"text_present"}` proves Google
  Docs text after a reopen without asking the caller to parse the canvas.

## Not implemented

- §41–§45 installed-app matrix, concurrency suite, headed OS matrix, matched
  competitor benchmark, and published benchmark. These require accounts,
  installed applications, and matched competitor runs; fixture tests cannot
  substitute for them.

## Last verified run

- `cargo test --workspace`: all suites pass, including 403 runtime library
  tests, 338 facade binary tests, and 4 `source_structure` tests. The
  `execution_plan` module contributes 14 passing contract and routing tests;
  `reference_rebase`, `text_conflict`, `progressive_observation`, and
  `task_observation` contribute their module tests.
  The intentionally Chrome-backed tests remain ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `npm run check:docs`, `check:topology`, `check:webmcp-browser`,
  `check:release`, and `check:v3`: pass in this pass (93 Markdown files, 12
  topology tests, a real Chrome WebMCP round trip, 16 release-gate tests, and
  the client-qualification checks).
- `node --test extensions/chrome-controlla/test-background.cjs`: pass in this
  pass, including child-frame swipe, child-frame pinch, and open-shadow click.
- `node --test extensions/chrome-controlla/test-p0-resources.mjs`: 15 passed,
  including the new focus-emulation contract. `test-observation.mjs`: 6 passed.
- `node --test scripts/test-transaction-background.mjs`: 13 passed in this pass
  against real Chrome.
- Earlier browser-harness evidence remains valid but was not rerun for this
  Rust-only change: `test-background-actions.mjs` (9), `test-p0-resources.mjs`
  (13), `test-editor-rebind.mjs` (19), `test-snapshot-efficiency.mjs` (6), and
  `test-observation.mjs` (6).
- `scripts/live-bulk-docs.mjs`: `ALL_LIVE_CHECKS_PASSED` on the previous live
  run. Docs 1k and 4k bulk inserts were verified, the exact-URL reopen reused
  the same tab, the persisted text was proven, ESPN page scroll moved the
  viewport, ESPN text verification passed, and the NFL link navigated.

## Boundary

The Node harness and crate suite cover the implemented items. Playwright-driven
real Chrome covers the canvas and bulk-editor transactions. The live Docs run
used the installed Chrome and the rebuilt native host; it did not reload the
extension. The inserted text was verified through the document's own copy path
and survived a same-tab reopen, so persistence is claimed for those two
documents only, not for every Google Docs surface. The remaining sections above
are still unimplemented and are not covered by these runs.
