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
| §14 canvas visibility loss | done | A hidden unreadable canvas reacquires presentation and rechecks visibility; on failure it returns `not_dispatched` before input, or `unknown` with `canvas_presentation_lost` mid-transaction. It never appends a DOM-insert tail. Harness asserts no input command is sent; real Chrome (`check:transaction-background`) proves both the stop and the resume-after-reacquire. |
| §40 decomposition | partial | The native cold-open lifecycle moved to `lib/owned-open.js`; `background.js` is 3,350 lines and its ratchet was lowered from 3,360 to 3,350. `v3_full.rs` is untouched. |

## Not implemented

- §15 Google Docs bulk text qualification and fast path. The current transaction
  still paces key events; no default backend changed without an installed-Chrome
  persistence run.
- §16 `TextSemantics` split between plain Unicode insertion and IME composition.
- §17–§22 unified task IR, operation contracts, cost-aware planning, scheduled
  parallel reads, speculative reads, and WebMCP-first routing beyond what already
  exists in the runtime.
- §23–§26 operation memory evolution, task-conditioned filtering, WALT URL
  promotion, and progressive observation.
- §27–§30 snapshot fencing, semantic mirror, invisible OOPIF/shadow routing, and
  full pointer/gesture parity.
- §32–§39 event-driven rebase classes, text conflict semantics, focus emulation,
  bounded resource pool, one-call existing-tab programs, automatic tab binding,
  and cheap typed verification.
- §41–§45 installed-app matrix, concurrency suite, headed OS matrix, matched
  competitor benchmark, and published benchmark.

## Boundary

The Node harness and crate suite cover every item above. Playwright-driven real
Chrome additionally passed `check:background-dom` (9 tests) and
`check:transaction-background` (9 tests). The extension installed in the user's
profile was **not** reloaded, and no signed-in live site was driven, so the
presentation/reuse paths have harness evidence and the canvas rule has real
renderer evidence, but neither has installed-profile evidence.
