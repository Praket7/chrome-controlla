# Build progress

Status: Phases 0 and 1 are pushed and gated. Phase 1 hosted CI run `37465119376` passed on Ubuntu, macOS, and Windows for commit `8dbf51d`. Phase 2 code review and local fixture gate passed; its live browser and platform qualification gate remains open. Phase 1 still has no live browser or client dispatch.

## Phase 0 — source extraction and baseline

- [x] Confirm source repository, remote, clean status, and pinned upstream SHA.
- [x] Copy controlling design documents into the new repository.
- [x] Extract only the self-contained browser crate and its inline tests.
- [x] Preserve Apache-2.0 license and upstream notice; record every copied source path and digest.
- [x] Pin Rust 1.99.0, Node.js 24.19.0, npm 11.17.0, Playwright 1.63.0, `rmcp` 3.5.1, MCP protocol 2026-07-28, and Chrome for Testing 154.0.8037.92; generate Rust and npm lockfiles.
- [x] Add a cross-platform Rust build, dependency, package, and documentation-link CI baseline.
- [x] Rerun focused checks after review findings are fixed; retain exact commands/results in `verification-matrix.md`.
- [x] Pass scoped independent review; commit with `chore: establish Chrome-only source extraction and provenance`.

## Review record

- Review 1: not approved; corrections and pinned-toolchain checks are recorded in [Phase 0 review 1](review/phase0-review-1.md). Sol re-review approved Phase 0 on 2026-10-06 after independently checking package, docs, dependency, provenance, and Rust gates. The final npm pin in CI was re-reviewed and approved. Commit `10f1aa7` is pushed to the private repository; hosted CI run `37456380443` passed on macOS, Linux, and Windows.

## Remaining phases

### Phase 1 — capability evaluator, CLI and package launcher

- [x] Add a single evaluator consumed by capability listing and dispatch authorization; include policy and capability revisions.
- [x] Add side-effect-free Rust CLI parsing/help/schema, invalid-flag exit code 2, and conservative JSON doctor report.
- [x] Add a package-relative launcher and host/architecture-bound npm archive with Apache license and NOTICE; clean-prefix install with spaces and restricted PATH verified on macOS arm64.
- [x] Add direct-only, bridge-only, unconfigured, denied, revoked, bounded dispatch/revocation, global/subcommand open-stdin help, doctor health-separation and launcher tests.
- [ ] Live transport/process/authentication round trip and client/browser dispatch remain unavailable: no service or browser runtime exists yet.

The first Phase 1 Sol review was not approved; all reported findings were fixed and Sol approved the local/fixture gate on 2026-10-06. Hosted CI run `37465119376` passed the full configured matrix for commit `8dbf51d`; local checks pass with 36 workspace tests. Red-first execution evidence was not recorded, so it is not claimed. Phase 1 evidence and remaining platform/live boundaries are in `verification-matrix.md` and `blockers.md`. Phase 2 code now has independent approval and local evidence; the live extension, headed Chrome and native input observation qualifications remain open.

### Phase 2 — persistent sessions, modes, identities and tabs

- [x] Implement persistent target/frame tracking, revision-bound references, per-target scheduling, shared-document locks, and dedicated/shared provider fixtures.
- [x] Implement owned/adopted/borrowed tab records, cleanup receipts, crash reconciliation, safe recovery handles, retryable pairing/cleanup, and focused cancellation/lifecycle tests.
- [x] Pass Sol's independent code review on 2026-10-06. Review findings and fixes are summarized in the Phase 2 local acceptance entry in `verification-matrix.md`.
- [ ] Qualify live MV3 extension attachment, headed Chrome, independent in-tab cleanup observation, and native focus/cursor/clipboard behavior. These are not claimed from fixtures or the headless provider smoke.
- [x] Commit and push the reviewed Phase 2 implementation; hosted CI run `37511048769` passed on Ubuntu, macOS, and Windows for final commit `928c3a8`.

Phase 2 source landed in `f0fd586`; follow-up commits `cfcf877` and `928c3a8` made the 1/4/8 scheduler fixture deterministic and fixed a Windows-only Clippy warning. Hosted CI run `37511048769` passed on all three configured operating systems. The failed intermediate runs and their causes are recorded in `verification-matrix.md`. The local code gate is complete; live browser, app-identity, and native input qualification remain open.

### Phase 3 — durable jobs and idempotency

- [x] Add SQLite journal admission keyed by authenticated-principal input, session, and idempotency key; bind replay to canonical JSON request digest.
- [x] Add transactional concurrent admission, one-time dispatch claims, persisted bounded deadlines, principal/session scoped access, bounded cursor wait, and pre-dispatch cancellation.
- [x] Unknown is terminal in this phase because no independent browser/app observer exists; the public journal has no promotion API.
- [x] Delivery is a persisted field separate from status: a claim is `unknown`, only `acknowledge_dispatch` sets `sent`, confirmed pre-send failure/deadline is `failed/not_sent`, and dispatched deadline/recovery is `unknown/unknown`.
- [x] Red-first focused tests observed the missing jobs module/Tokio compile failure; ten focused journal/lifecycle tests cover competing admissions/claims, crash windows, fixture endpoint send counting across replay, deadline recovery, and pre-send failure delivery status.
- [x] Sol approved the local Phase 3 journal gate after corrective reviews; full local workspace tests pass (95 passed, 1 ignored) with format, Clippy, provenance and doc-link checks. This approval does not cover the missing MCP jobs tool or browser dispatch route.
- [ ] Add the actual jobs MCP tool and route accepted work through CDP/extension handlers; these handlers and transport do not exist in the current repository. No live browser mutation or arbitrary Promise completion is claimed.
- [ ] Inject process crashes at every journal boundary and verify exact 12-second resolve/reject cases through both browser routes.


### Phase 4 — reliable input, guards and interference

- [x] Add typed guard snapshots, invalidation records, input action variants, and strict-background `NeedsForeground` fail-closed behavior.
- [x] Add revision-bound CDP fill/insert/sequential text dispatch, exact semantic matching, actual value readback, and predicate-gated click/drag event sequences. Final fill evaluation compares the live value immediately before writing; insert and every key event refresh value/focus immediately before each send.
- [x] Inject target navigation after final pre-dispatch check; confirm write is withheld and record fixture probe-to-mutation timing (2773 us in the recorded run).
- [x] Add local fixtures for revision/dependency mismatch, Unicode input/readback, strict-background non-dispatch, locator ambiguity, mouse predicates, and race-window invalidation.
- [x] Run an ignored installed-Chrome headless DOM fixture: revision-bound fill, insert, and ASCII sequential keys produced the expected Unicode value and UTF-16 caret position. Dedicated launch now bootstraps Page/Runtime for newly created targets.
- [ ] Qualify actual Chrome DOM behavior for masked/contenteditable/IME/event-dependent controls, real click/overlay interception, drag DOM results, and canvas object movement.
- [ ] Measure the real Chrome check-to-dispatch window and repeated browser interference. The websocket fixture measures only its mock protocol window; no DOM-side mutation was injected during that live interval.
- [ ] Qualify native focus/cursor/clipboard behavior with an independent observer.

The supported-control code path includes the final fill-side value comparison and per-CDP-send value/focus checks for insert and sequential keys. These narrow the race window; page handlers, Chrome processing, and server effects are not atomic. Focused mock coverage passes; the real Chrome DOM readback predates this follow-up. The broader Phase 4 acceptance gate remains partial: application identity/edit observers, qualified masks/contenteditable/IME/event-dependent controls, real overlay/drag/canvas outcomes, repeated adversarial interference, and native focus/cursor/clipboard observation remain open.

### Phase 5 — compact observations and bounded extraction

- [x] Add revision-bound `BrowserConnection::observe` with selected CSS fields, page-side item/text/byte caps, freshness timestamp/epoch, explicit missing-field/item omissions, truncation, and informational non-resumable cursor metadata. The serialized result including metadata is kept within the byte budget or returns an error.
- [x] Add bounded scroll-container extraction with selected field schema, stable-ID deduplication, page-side record/text/byte caps, step limits, progress cursor, missing coverage and terminal evidence. A wrong-account preflight returns before issuing a scroll command; each later page script checks identity before scrolling. `complete` additionally requires an independently authoritative expected-count match, a container-scoped terminal marker at scroll end on two no-new-ID observations, and no missing/clipped/truncated evidence.
- [x] Add policy fixtures for 42 records across recycled eight-node batches, a separate 21-item bucket, blocked expansion, duplicate labels with distinct IDs, stale count, infinite feed without terminal marker, and wrong-account 404. Fixture policy and dedup tests pass.
- [ ] Run the extraction API against real Chrome virtualized and hidden-section DOM fixtures; current Phase 5 fixtures model expected evidence and ID recycling but do not exercise CDP/DOM traversal.
- [ ] Add the MCP `observe` tool and adapter-level pagination/section traversal, AX and visual crop routes. Current extraction takes one caller-selected container per call; cursors are explicitly not resumable.

Local verification on 2026-10-06, macOS 26 / Darwin 25.6 arm64, Rust 1.99.0: focused observation tests cover mocked wrong-account/no-scroll, runtime selector exceptions, malformed evaluation values, empty result arrays, missing fields, required expected count, page-script limits and exact final serialized-size enforcement. Full workspace tests and Clippy pass (81 passed, 1 ignored; integration groups 7, 3, 8, 10, and 3). Provenance, docs, and dependency checks pass. No live extraction, payload/latency baseline, MCP tool, or complete Phase 5 gate is claimed. Page budgets do not impose a wall-time bound on native selector/text reads.
