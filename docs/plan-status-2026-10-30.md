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
| §40 decomposition | partial | The native cold-open lifecycle moved to `lib/owned-open.js`; the CDP script bodies moved to `lib/input-scripts.js` and the readback/pacing helpers to `lib/typing-support.js`. The canvas document-proof path moved to `mcp/v3_canvas_proof.rs`, bringing `v3_full.rs` to 31,987 lines under its 32,000-line ratchet. `lib/input.js` is 890 lines and its 900-line ratchet holds. `background.js` remains at its 3,350-line ceiling. |

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

- §17–§22 unified task IR, operation contracts, cost-aware planning, scheduled
  parallel reads, speculative reads, and WebMCP-first routing beyond what already
  exists in the runtime.
- §23–§26 operation memory evolution, task-conditioned filtering, WALT URL
  promotion, and progressive observation.
- §27–§30 snapshot fencing, semantic mirror, invisible OOPIF/shadow routing, and
  full pointer/gesture parity. The live Docs run still exposes the snapshot gap:
  a same-origin blank frame wins the candidate slot and the document text is only
  reachable through the copy proof, not through the snapshot item.
- §32–§39 event-driven rebase classes, text conflict semantics, focus emulation,
  bounded resource pool, one-call existing-tab programs, automatic tab binding,
  and cheap typed verification.
- §41–§45 installed-app matrix, concurrency suite, headed OS matrix, matched
  competitor benchmark, and published benchmark.

## Last verified run

- `cargo test --workspace`: all suites pass. The facade binary ran 337 tests,
  the crate library 369, and `source_structure` 4; 5 ignored tests remain
  intentionally Chrome-backed.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `node --test extensions/chrome-controlla/test-background.cjs`: pass (includes
  the cold-open readiness handshake fix; the full parallel suite no longer races
  the native bridge).
- `node --test scripts/test-transaction-background.mjs`: 13 passed (real Chrome).
- `node --test scripts/test-background-actions.mjs`: 9 passed.
- `node --test extensions/chrome-controlla/test-p0-resources.mjs`: 13 passed.
- `node --test scripts/test-editor-rebind.mjs`: 19 passed.
- `node --test scripts/test-snapshot-efficiency.mjs`: 6 passed.
- `node --test extensions/chrome-controlla/test-observation.mjs`: 6 passed.
- `npm run check:docs`, `check:topology`, `check:editor-ladder`,
  `check:presentation`, `check:webmcp-browser`, `check:release`, `check:v3`:
  pass.
- `scripts/live-bulk-docs.mjs`: `ALL_LIVE_CHECKS_PASSED`. Docs 1k and 4k bulk
  inserts were verified, the exact-URL reopen reused the same tab, the persisted
  text was proven, ESPN page scroll moved the viewport, ESPN text verification
  passed, and the NFL link navigated.

## Boundary

The Node harness and crate suite cover the implemented items. Playwright-driven
real Chrome covers the canvas and bulk-editor transactions. The live Docs run
used the installed Chrome and the rebuilt native host; it did not reload the
extension. The inserted text was verified through the document's own copy path
and survived a same-tab reopen, so persistence is claimed for those two
documents only, not for every Google Docs surface. The remaining sections above
are still unimplemented and are not covered by these runs.
