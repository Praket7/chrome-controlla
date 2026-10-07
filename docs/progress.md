# Build progress

Status: Phases 0–3 are gated and pushed. Phase 4 guarded input, Phase 5 bounded extraction, and the Phase 7 local verifier/cache slice have fixture evidence recorded in `verification-matrix.md`. Phase 6 remains unimplemented. The unpacked Chrome extension is loaded in the user's Chrome, but live pairing/dispatch has not been verified. Broad live-app/platform qualification remains open. File artifacts can be registered and selected through local MCP stdio on Unix; app acceptance/persistence is not claimed.

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
- [x] Extend the isolated Chrome fixture: stale values withhold writes; marked masked/event-dependent controls and plain contenteditable fail closed; password controls return unsupported without observed values; an overlay blocks click dispatch; DOM drag verifies the moved element bounds; strict-background text works with no native requirement.
- [x] Exercise supported installed-Chrome DOM cases: declared masked/event-dependent controls and contenteditable fail closed, password values are not exposed, an overlay blocks click, and DOM drag verifies final bounds.
- [x] Add guarded CDP text composition for ordinary input/textarea and verify composition events and committed Japanese text in isolated Chrome. Native OS IME candidate UI/conversion and masked/app-specific controls remain unqualified.
- [x] Add a read-only macOS native snapshot and check foreground app, cursor position, and pasteboard change count before/after strict-background text in isolated Chrome.
- [ ] Qualify undeclared app-specific event semantics, canvas movement beyond fail-closed refusal, and repeated live interference.
- [ ] Measure the real Chrome check-to-dispatch window and repeated browser interference. The websocket fixture measures only its mock protocol window; no DOM-side mutation was injected during that live interval.
- [ ] Qualify native focus/cursor/clipboard behavior across supported OSes/modes and repeated real-user interference; current observer evidence is one macOS isolated fixture.

The supported-control code path includes the final fill-side value comparison and per-CDP-send value/focus checks for insert and sequential keys. These narrow the race window; page handlers, Chrome processing, and server effects are not atomic. Isolated Chrome fixtures cover marked masked/event-dependent controls and contenteditable refusals, password non-disclosure, overlay interception, DOM drag bounds, stale-value withholding, simulated IME composition, per-session internal text paste, and private-handle file selection. The read-only native observer checks focus/cursor/clipboard-change count on macOS only. OS-level IME, canvas semantics, authoritative app identity/edit observers, other OSes, repeated adversarial interference, and app acceptance/persistence remain open.

### Phase 5 — compact observations and bounded extraction

- [x] Add revision-bound `BrowserConnection::observe` with selected CSS fields, selected-node AX, bounded PNG crops, page-side item/text/byte caps, freshness timestamp/epoch, explicit missing-field/item omissions, truncation, and byte-bounded output.
- [x] Add bounded scroll-container extraction with selected field schema, stable-ID deduplication, page-side record/text/byte caps, step limits, progress cursor, missing coverage and terminal evidence. A wrong-account preflight returns before issuing a scroll command; each later page script checks identity before scrolling. `complete` additionally requires an independently authoritative expected-count match, a container-scoped terminal marker at scroll end on two no-new-ID observations, and no missing/clipped/truncated evidence.
- [x] Add policy fixtures for 42 records across recycled eight-node batches, a separate 21-item bucket, blocked expansion, duplicate labels with distinct IDs, stale count, infinite feed without terminal marker, and wrong-account 404. Fixture policy and dedup tests pass.
- [x] Run the extraction API against a real Chrome virtualized 42-record list; wrong-account preflight returns unknown without scrolling. The one synthetic run is recorded as diagnostic evidence only.
- [x] Qualify declared hidden-section traversal and blocked expansion in an isolated real Chrome synthetic DOM fixture. Representative application behavior remains open.
- [x] Add local MCP stdio `session`, `observe`, and `extract` tools. Session connects only to an opted-in loopback endpoint or Chrome permissioned auto-connect, requires explicit target IDs, and returns revision-bound target references. Extraction calls the existing bounded API for caller-declared sections under one aggregate record/byte/deadline budget.
- [x] Add selected-node accessibility, byte-preflighted screenshot crop, and bounded single-use resumable cursors bound to target/revision/spec.

Local library verification on 2026-10-06, macOS 26 / Darwin 25.6 arm64, Rust 1.99.0: focused observation tests cover mocked wrong-account/no-scroll, selector exceptions, malformed evaluation values, empty results, missing fields, expected counts, page-script limits and exact final-size enforcement. Phase 5 advanced tests additionally bound AX selection and crop pixels, scope IDs by section, preserve deterministic cursor bindings, and mark trimmed coverage partial. Native selector/text reads still have no wall-time bound.

## Phase 7 local implementation record — 2026-10-06

- [x] Red-first review-fix tests reproduced forged `IndependentState` claims passing and cache target drift failing to quarantine. Added checks for operation/provenance fields and positive, truncated, and same-length-corrupt artifacts.
- [x] Evidence declarations bind operation/provenance ID, principal/session/target/app/account/revision, observer label, time and predicate hash. Because no runtime-controlled app observer exists, all caller-supplied evidence now returns `inconclusive`.
- [x] Added canonical cache signatures over workflow/site/app/schema/content/permission/identity/footprint/verifier/version/authority/failure-policy fields; drift and non-pass verification quarantine. Admission and unquarantine require an opaque successful training/validation token.
- [ ] No production suite runner can mint qualification tokens, so production cache admission is not available. B28–B30 independent-ground-truth fixtures, CC-16, cold/warm cost study, live app persistence, artifact download capture and workflow/MCP integration remain open.

This is a local contract and fail-closed cache-mechanics slice. Phase 6's workflow compiler is not present; cache is not wired to execution; and no production path can issue qualification tokens. The public verifier cannot claim observed or persisted success. No CC-16, app persistence, visual quality or live cache qualification is claimed.

## Phase 5 MCP stdio vertical slice — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| MCP initialize, `tools/list`, and session discovery `tools/call` over rmcp duplex transport | pass | macOS 26 / Rust 1.99.0 | Lists session/observe/extract plus accessibility, screenshot crop, artifact register/file select, and shared observe tools with schemas; discovery call succeeds. |
| MCP session connect/list-targets, observe, and two-section extract `tools/call` against mocked CDP websocket | pass | macOS 26 / Rust 1.99.0 | Exercises manager bootstrap, explicit target selection, target-ref serialization/resolution, bounded observation, and per-section completeness through actual MCP calls. |
| Loopback endpoint policy and Direct CDP grants | pass | Rust fixtures | Rejects DNS/non-loopback, credential-bearing, malformed, and non-WebSocket endpoints. Direct CDP requires a distinct grant; shared-extension sessions remain rejected by direct CDP. |
| Phase 5 synthetic virtualized-list extraction | pass | Installed Chrome 154.0.8037.98; commits `44041af`, `1fd49f9`, and advanced-fixture follow-up | Latest single run extracted 42 rows; 438.17 ms / 1,860 B versus 1.39 ms / 2,760 B for full DOM. This is a diagnostic, not a performance claim. Wrong-account preflight returned unknown without scrolling; successful and blocked expansion are also covered by a synthetic Chrome fixture. |

## Phase 5 MCP boundaries

- The stdio adapter is local-only. Explicit CDP requires `COMPTROL_ALLOW_DIRECT_CDP=1` plus `COMPTROL_CDP_ENDPOINT`; only loopback IP literals are accepted. Permissioned auto-connect requires `COMPTROL_CHROME_AUTO_CONNECT=1` and preserves Chrome's native consent prompt.
- Synthetic list and hidden-section fixtures do not qualify representative app account markers, browser/client performance, or general virtualized-list behavior. MCP transport tests use mocked CDP; no external MCP client acceptance is claimed.
- `execute`, `jobs`, guide resource, durable MCP identity, artifact output/download, and shared-extension input/extraction are outside this slice.
