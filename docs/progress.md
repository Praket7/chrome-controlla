# Build progress

## Offline continuation — 2026-10-07

- 2026-10-08 live native-route follow-up: the current MCP paired and released only the authorized testing Classroom tab, read its heading and one selected accessibility node with stable root identity, then paired a temporary local HTTP fixture and completed guarded fill and click with DOM readback. A separate Chrome accessibility read confirmed the fixture result. A later fresh Hotload revision 1 session independently repeated exact-tab pairing, read `Classroom / testing`, and released the tab. A live byte-budget probe plus source audits exposed observation schema/runtime mismatches for byte, item, text, and cursor limits. After aligning direct/shared observation schema and workflow compiler/schema bounds, the current binary was loaded by Hotload revision 2; live schema reads confirmed the corrected limits, and the selected Classroom tab was paired, read, and released again. This latest heading scan returned two `h1` matches and correctly marked possible omissions/truncation. Full local validation passes; hosted CI for the follow-up commit is still running. No Classroom write, app-save persistence, cross-profile interleaving, or broad external-client acceptance was tested. After the repository was made public, hosted run [37822533307](https://github.com/Praket7/chrome-controlla/actions/runs/37822533307) passed on Ubuntu, macOS, and Windows for commit `35a8c1f`.

- 2026-10-08 local follow-up: a Phase 6 journal fix permits completion when no browser dispatch occurred while preserving the refusal to complete an unacknowledged dispatch; the 18-test jobs suite and full workspace suite pass, and Sol approved the scoped transition. Client config generation no longer enables legacy remote-debugging auto-connect by default. Freebuff Desktop was connected, but its isolated run loaded no Controlla MCP tools (0/2 requested read-only actions); Freebuff client acceptance remains open.

- Added a Chrome native messaging host and extension route modeled on Comptrol's one-time bridge registration. The host provides read-only tab discovery before pairing, consumes private one-session rendezvous files, forwards bounded commands through the existing guarded shared provider, and releases tabs on disconnect. Native and popup attachments have separate ownership. The focused extension/native-host fixtures pass and Sol re-reviewed the bridge fixes. An installed Chrome error exposed a one-character extension-ID mismatch in the host manifest; it was corrected. Chrome then launched the native host, refreshed a private tab inventory, and the live MCP discovery route reported `companion_extension.available=true`. The extension uses a Chrome alarm for reconnect after worker suspension. The later 2026-10-08 live test above verifies native attachment and guarded commands on selected tabs.

- Native bridge setup is the authoritative shared-tab path: run the one-time `controlla install-bridge <exact-extension-id>`, reload the unpacked extension, call `discover_shared_tabs`, and pass that result's `snapshot.host_id` with only intended exact tab IDs to `pair_shared`; then confirm using `accept_shared`. Discovery lists at most 100 HTTP(S) tabs and sets `snapshot.truncated` when more are available; the snapshot is fresh for 15 seconds and pairing rejects a changed host or IDs absent from the fresh inventory. The corrected ID `bhgfjpbajecihfbgaikampminappgodh`, rebuilt host, fresh private inventory, and current MCP companion discovery are live-verified after the extension reload. Selected-tab attachment, guarded fixture commands, and release are also live-verified as of 2026-10-08. Host/profile binding, truncation, stale-inventory and pair-error checks, serialized release, and cleanup remain fixture/local evidence. Popup endpoint/token pairing is reserved for an unavailable or busy native host. Alarm reconnection and popup native-status behavior remain fixture-only.

- Phase 3 installed-Chrome direct-CDP qualification covers measured 12-second resolve/reject and independently dispatched navigation during a confirmed in-flight evaluation; the original target reference becomes stale. Sol approved the narrow test after a dispatch-barrier fix. The live native-extension route also returned a delayed observation after 12,017 ms and surfaced a delayed CDP exception after 12,014 ms without hanging. Async page-Promise parity, extension-route navigation invalidation, live MCP job parity, and full MCP/browser process-kill recovery remain open.
- Phase 5 adds `shared_accessibility` for one CSS-selected AX node, with a 1–60 second whole-call deadline, caller byte budget, root-frame/loader/URL recheck, and a 1 MiB pre-parse WebSocket message cap. Phase 4 shared click requires a distinct DOM postcondition whose exact value changes; fixture tests reject unchanged-state false positives. Sol reviewed and approved the fixes. The 2026-10-08 live test verified selected-node AX on Classroom and guarded click on a local fixture.
- Phase 8 offline CapCut recipe now rejects end cards under one second or below 4.5:1 text contrast; the focused recipe suite passes 5/5. Slides and Canva planner principal labels now come from the server-owned `local-stdio` identity, with regression coverage for forged request fields. App routes remain unqualified.
- Phase 12 has macOS arm64 distinct-binary lifecycle evidence and an unpublished `0.1.0` candidate; prior completed hosted run [37628108314](https://github.com/Praket7/chrome-controlla/actions/runs/37628108314) passed clean-prefix host-matched consumer installs on Windows, macOS, and Linux. Published GitHub installation, signing, live MCP compatibility, and release remain open.
- Final integrated full workspace suite: 186 passed with the default filter; all 3 Chrome-required ignored tests then passed explicitly. Formatting, Clippy, docs, benchmark fixtures, app/client checks, package checks, extension fixtures, and provenance validation passed after the latest evidence edits. Phases 6, 7, 9, 10 and 12 still have live-environment or platform gates open.
- Live Phase 5 shared-extension evidence: the loaded Chrome Controlla Shared Tab Bridge 0.1.0 paired only the test Classroom tab, server accepted the session, target enumeration returned the requested URL/root frame, and bounded read-only observation returned page text with frame/loader/URL unchanged. A broad `body` text read was truncated; a focused `body`→`h1` observation completed without truncation. No writes were sent; complete extraction, mutation/save/persistence, and external-client acceptance remain open. See the dated live evidence in [verification-matrix.md](verification-matrix.md).
- Phase 6 adds a real child-process kill/reopen test for checkpoint preservation, unknown delivery, and replay refusal at the journal/dispatch-claim layer. It uses fixture file effects, not a live MCP service or browser dispatch. Phase 9 adds a real OpenCode 1.18.5 stdio connection check using an isolated temporary config; the generated v1 shape connected, while the v2 shape was rejected by this installed v1 client. No model-driven tool call was made.
- Phase 4 follow-up: fixed the shared click guard’s false refusal of implicit-submit buttons that are not associated with a form. The regression test evaluates the production predicate; focused input tests and independent review pass. The earlier live Classroom refusal remains historical evidence; a post-fix guarded click succeeded on the local fixture, without claiming Classroom mutation.

Status: Phases 0–5 implementation and local/CI gates are committed and pushed on their phase branches; Phase 2 platform focus/cursor/clipboard qualification and Phase 3 extension-route Promise parity/full-service process-kill coverage remain open. Direct installed-Chrome Promise resolve/reject and external navigation now pass a focused headless test; they do not establish extension parity. Phase 4–5 have live native-route pairing, read-only observation and selected-node AX in the test Classroom, plus guarded fill/click on a local fixture; Classroom writes and app persistence remain open. Phase 6 includes a bounded child-process script worker and journal-layer forced-process-kill fixture; OS RSS/CPU isolation and full-service browser process-kill coverage remain open. Phase 7 now adds runtime-owned artifact integrity readback, but no production independent app-state observer or cold/warm performance evidence exists. Phase 8 has offline app planners only; OAuth/API/UI routes, app readback, persistence, export, and visual review remain open. Phase 9 has an opt-in authenticated loopback HTTP endpoint, passing in-process protocol checks, a bounded live HTTP process smoke, 20/20 pre-live guide usability, a narrow Classroom connection through local stdio, and one real OpenCode 1.18.5 read-only `guide` tool call using a local model; full client workflow acceptance remains open. Phase 10 has preregistered pilot/held-out manifests and fixture analysis; no benchmark tasks or comparisons have run. Phase 12 has macOS arm64 lifecycle evidence, an unpublished candidate, and current three-platform clean-prefix installs; published GitHub installs, signing, and release remain open. Production app workflows, independent outcomes, remaining external-client acceptance, and real benchmark results are still required. The child JS worker is not an OS security boundary; untrusted/production scripts, arbitrary mutation programs, and file-backed/download artifacts remain blocked. File selection proves browser selection only, not app acceptance/persistence.

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
- [x] Start a bounded authenticated loopback HTTP service and verify initialize, tool discovery, guide call, and missing-bearer rejection; see the Phase 9 process-smoke evidence.
- [ ] Full external MCP-client acceptance and selected-tab browser input remain open. The current Codex Hotload route completed live pairing and read-only observation in the authorized test Classroom tab; see the latest connection evidence below.

The first Phase 1 Sol review was not approved; all reported findings were fixed and Sol approved the local/fixture gate on 2026-10-06. Hosted CI run `37465119376` passed the configured matrix for commit `8dbf51d`; local checks pass with 36 workspace tests. Red-first execution evidence was not recorded, so it is not claimed. The bounded loopback HTTP process smoke is now verified; external-client tool actions and live browser dispatch remain open. Phase 2 has independent approval and local evidence plus an earlier selected-tab attachment/read-only Classroom observation/release. Native input observation and fresh post-fix browser actions remain open.

### Phase 2 — persistent sessions, modes, identities and tabs

- [x] Implement persistent target/frame tracking, revision-bound references, per-target scheduling, shared-document locks, and dedicated/shared provider fixtures.
- [x] Implement owned/adopted/borrowed tab records, cleanup receipts, crash reconciliation, safe recovery handles, retryable pairing/cleanup, and focused cancellation/lifecycle tests.
- [x] Pass Sol's independent code review on 2026-10-06. Review findings and fixes are summarized in the Phase 2 local acceptance entry in `verification-matrix.md`.
- [x] Qualify one live MV3 attachment to the explicitly selected test Classroom tab in headed Chrome; observe the intended page and release debugger attachment while leaving the tab open (recorded under Phase 5).
- [ ] Repeat attachment/cleanup independently and qualify native focus/cursor/clipboard behavior across supported OSes/modes.
- [x] Commit and push the reviewed Phase 2 implementation; hosted CI run `37511048769` passed on Ubuntu, macOS, and Windows for final commit `928c3a8`.

Phase 2 source landed in `f0fd586`; follow-up commits `cfcf877` and `928c3a8` made the 1/4/8 scheduler fixture deterministic and fixed a Windows-only Clippy warning. Hosted CI run `37511048769` passed on all three configured operating systems. The failed intermediate runs and their causes are recorded in `verification-matrix.md`. The local code gate is complete. One live selected-tab Chrome attachment/read-only observation/release is recorded, while repeated cleanup, independent app identity, and native input qualification remain open.

### Phase 3 — durable jobs and idempotency

- [x] Add SQLite journal admission keyed by authenticated-principal input, session, and idempotency key; bind replay to canonical JSON request digest.
- [x] Add transactional concurrent admission, one-time dispatch claims, persisted bounded deadlines, principal/session scoped access, bounded cursor wait, and pre-dispatch cancellation.
- [x] Unknown is terminal in this phase because no independent browser/app observer exists; the public journal has no promotion API.
- [x] Delivery is a persisted field separate from status: a claim is `unknown`, only `acknowledge_dispatch` sets `sent`, confirmed pre-send failure/deadline and restart before dispatch claim are `failed/not_sent`, and restart/deadline after dispatch claim is `unknown` while preserving observed `sent` or `unknown` delivery.
- [x] Red-first focused tests observed the missing jobs module/Tokio compile failure; the journal/lifecycle suite covers competing admissions/claims, pre-dispatch and post-claim subprocess kills, fixture endpoint send counting across replay, deadline recovery, and pre-send failure delivery status. The 2026-10-07 restart correction adds acknowledged-delivery, completed-job, and running/not-sent coverage; see `verification-matrix.md`.
- [x] Sol approved the local Phase 3 journal gate after corrective reviews; full local workspace tests pass (95 passed, 1 ignored) with format, Clippy, provenance and doc-link checks. Phase 6 later added the MCP workflow/job route and async QuickJS observation broker; these do not establish live browser mutation or exactly-once effects.
- [x] Add the durable workflow/job MCP tools and local broker route through the current session adapter. No live browser mutation or arbitrary Promise completion is claimed.
- [x] Verify the broker-backed QuickJS worker resolves a Promise after exactly 12 seconds; this is local worker evidence only.
- [x] Qualify measured 12-second resolve/reject and navigation during a confirmed in-flight evaluation against isolated installed Chrome through direct CDP; the original reference is stale after navigation. This is not extension-route or full MCP-job evidence.
- [x] Kill/reopen a child process at seven durable journal fixture cut points and verify recovered state plus replay refusal; see `verification-matrix.md`. This does not cover every runtime/browser boundary.
- [ ] Verify 12-second resolve/reject and navigation during evaluation through the extension-bridge route and live MCP job; inject kills at remaining runtime/browser dispatch boundaries.
- [x] Verify a delayed 12-second result and exception through the native extension route on a temporary local Chrome page; release and close the fixture after both calls.


### Phase 4 — reliable input, guards and interference

- [x] Add typed guard snapshots, invalidation records, input action variants, and strict-background `NeedsForeground` fail-closed behavior.
- [x] Add revision-bound CDP fill/insert/sequential text dispatch, exact semantic matching, actual value readback, and predicate-gated click/drag event sequences. Final fill evaluation compares the live value immediately before writing; insert and every key event refresh value/focus immediately before each send.
- [x] Inject target navigation after final pre-dispatch check; confirm write is withheld and record fixture probe-to-mutation timing (2773 us in the recorded run).
- [x] Add local fixtures for revision/dependency mismatch, Unicode input/readback, strict-background non-dispatch, locator ambiguity, mouse predicates, and race-window invalidation.
- [x] Run an ignored installed-Chrome headless DOM fixture: revision-bound fill, insert, and ASCII sequential keys produced the expected Unicode value and UTF-16 caret position. Dedicated launch now bootstraps Page/Runtime for newly created targets.
- [x] Repeat the installed-Chrome focus-handler interference fixture three times; every guarded fill yields and preserves the external value. Controlled fixture evidence only, not live-user interference or a timing bound.
- [x] Extend the isolated Chrome fixture: stale values withhold writes; marked masked/event-dependent controls and plain contenteditable fail closed; password controls return unsupported without observed values; an overlay blocks click dispatch; DOM drag verifies the moved element bounds; strict-background text works with no native requirement.
- [x] Exercise supported installed-Chrome DOM cases: declared masked/event-dependent controls and contenteditable fail closed, password values are not exposed, an overlay blocks click, and DOM drag verifies final bounds.
- [x] Add guarded CDP text composition for ordinary input/textarea and verify composition events and committed Japanese text in isolated Chrome. Native OS IME candidate UI/conversion and masked/app-specific controls remain unqualified.
- [x] Make shared-extension click confirmation depend on a distinct, changed exact DOM postcondition; unchanged text and oversized page strings fail closed in focused fixtures. Live click remains open.
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
- [x] Add one-node `shared_accessibility` through the selected-tab extension route with whole-call deadline, selected-node/output bounds, root-frame/loader/URL recheck, and a 1 MiB inbound message cap. Fixture-verified; live AX read remains open.

Local library verification on 2026-10-06, macOS 26 / Darwin 25.6 arm64, Rust 1.99.0: focused observation tests cover mocked wrong-account/no-scroll, selector exceptions, malformed evaluation values, empty results, missing fields, expected counts, page-script limits and exact final-size enforcement. Phase 5 advanced tests additionally bound AX selection and crop pixels, scope IDs by section, preserve deterministic cursor bindings, and mark trimmed coverage partial. Native selector/text reads still have no wall-time bound.

## Phase 7 local implementation record — 2026-10-06

- [x] Red-first review-fix tests reproduced forged `IndependentState` claims passing and cache target drift failing to quarantine. Added checks for operation/provenance fields and positive, truncated, and same-length-corrupt artifacts.
- [x] Evidence declarations bind operation/provenance ID, principal/session/target/app/account/revision, observer label, time and predicate hash. Because no runtime-controlled app observer exists, all caller-supplied evidence now returns `inconclusive`.
- [x] Added canonical cache signatures over workflow/site/app/schema/content/permission/identity/footprint/verifier/version/authority/failure-policy fields; drift and non-pass verification quarantine. Admission and unquarantine require an opaque successful training/validation token.
- [ ] No production suite runner can mint qualification tokens, so production cache admission is not available. B28–B30 independent-ground-truth fixtures, CC-16, cold/warm cost study, live app persistence, artifact download capture and workflow/MCP integration remain open.

This is a local contract and fail-closed cache-mechanics slice. The Phase 6 workflow compiler is present, but the cache is not wired to execution and no production path can issue qualification tokens. The public verifier cannot claim observed or persisted success. No CC-16, app persistence, visual quality or live cache qualification is claimed.

The 2026-10-06 continuation adds field/object/state predicate evaluation gated on an opaque observer receipt and fixture-only receipt issuance. A review of the MCP and browser session paths found no safe production readback issuer for app state: direct/shared observation returns page DOM data, while account/document revisions are supplied by the caller. Those paths cannot establish independent application state or persistence, so they do not mint app-verification evidence. The 2026-10-07 artifact slice adds `artifact_verify`: after `artifact_register`, the runtime re-reads bytes through the opaque session handle and compares them with the registration-time length/SHA-256. Its MCP fixture verifies the pass result and explicit scope. This proves local staging-file integrity only; it is not evidence of app upload, acceptance, persistence, or download origin. Caller-supplied app evidence remains inconclusive and cache admission stays unavailable. Live app persistence, visual review, cold/warm cost study and the release suite remain open.

## Phase 8 app capability gate — 2026-10-06

- [x] Implemented and fixture-verified generic shared-extension `shared_input` fill/click: exact unique CSS target, visible/unobstructed and non-disabled controls, expected current value/text, separate post-event fill readback, fresh click hit test immediately before mouse dispatch, same-root-frame/loader/URL readback, best-effort release after every attempted press, and a bounded 6–60 second overall deadline. A check/dispatch race remains; uncertain input returns an error and requires re-observation.
- [x] Added offline plans for a 10-slide revision-bound Google Slides deck, a five-page Canva design, and a licensed-media/timing-validated CapCut timeline recipe. Slides/Canva caller preconditions are explicitly non-authoritative; Slides never emits caller-directed deletes, and Canva sync is only an advisory candidate. Exposed them as read-only MCP tools; none connects to a vendor API, browser app, or media asset.
- [x] Added per-app pre-live acceptance briefs with independent correctness and visual-quality rubrics; all evidence is still pending.
- [x] Recorded per-app readiness and evidence requirements in `verification-matrix.md` and the master guide. All professional-app workflow cells remain `unqualified`; the generic browser tools do not establish app support.
- [ ] Slides API OAuth injection/dispatch, object/revision readback, reload/export and visual review are absent. Both the single-text request and fixed 10-slide batch plan are locally fixture-tested; neither calls Google. No app-specific writes were attempted.
- [ ] Canva Connect or Apps SDK route, account/entitlement qualification, page-lock/conflict handling and save/export verification are absent. No sync was used as a read probe.
- [ ] CapCut Web has no implemented UI adapter or verified official timeline API route; no third-party API is substituted. Import/edit/export/playback remain unqualified.
- [ ] A dedicated Chrome test profile and test-account sign-in were approved. Representative Slides/Canva/CapCut documents/media and live reload/export evidence are still needed after supported routes exist; no app-specific route or write was available in this phase.

The Phase 8 slice includes generic browser fill/click plus the three offline app planners described in `docs/review/phase8-offline-planners.md`. No professional-app edit, save/persistence verification, or live qualification is implemented; the app execution gates stay open. Official vendor docs confirm Slides revision checks and Canva's short-lived design-edit session/commit-on-sync semantics, but no authorized adapter is connected; no official general CapCut Web timeline API was confirmed in the reviewed docs.

## Phase 5 MCP stdio vertical slice — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| MCP initialize, `tools/list`, and session discovery/guide `tools/call` over rmcp duplex transport | pass | macOS 26 / Rust 1.99.0 | Lists session/observe/extract, guide, accessibility, screenshot crop, artifact register/file select, and shared observe tools with schemas; discovery call succeeds and guide rejects a mismatched server version. |
| MCP session connect/list-targets, observe, and two-section extract `tools/call` against mocked CDP websocket | pass | macOS 26 / Rust 1.99.0 | Exercises manager bootstrap, explicit target selection, target-ref serialization/resolution, bounded observation, and per-section completeness through actual MCP calls. |
| Loopback endpoint policy and Direct CDP grants | pass | Rust fixtures | Rejects DNS/non-loopback, credential-bearing, malformed, and non-WebSocket endpoints. Direct CDP requires a distinct grant; shared-extension sessions remain rejected by direct CDP. |
| Phase 5 synthetic virtualized-list extraction | pass | Installed Chrome 154.0.8037.98; commits `44041af`, `1fd49f9`, and advanced-fixture follow-up | Latest single run extracted 42 rows; 438.17 ms / 1,860 B versus 1.39 ms / 2,760 B for full DOM. This is a diagnostic, not a performance claim. Wrong-account preflight returned unknown without scrolling; successful and blocked expansion are also covered by a synthetic Chrome fixture. |

## Phase 5 MCP boundaries

- The stdio adapter is local-only. Explicit CDP requires `COMPTROL_ALLOW_DIRECT_CDP=1` plus `COMPTROL_CDP_ENDPOINT`; only loopback IP literals are accepted. Permissioned auto-connect requires `COMPTROL_CHROME_AUTO_CONNECT=1` and preserves Chrome's native consent prompt.
- Synthetic list and hidden-section fixtures do not qualify representative app account markers, browser/client performance, or general virtualized-list behavior. MCP transport tests use mocked CDP; no external MCP client acceptance is claimed.
- At the Phase 5 commit, `workflow`, `workflow_status`, and the `guide` tool did not exist; they were added in later phases. Durable identity, artifact output/download, and shared-extension input/extraction remain outside the current slice.

## Phase 6 deterministic workflow slice — 2026-10-06

- [x] Spike pinned `rquickjs 0.14.0`: verified async broker functions, interrupt, heap and stack caps; Wasmtime was confirmed to be Wasm, not JavaScript. `node:vm` is not treated as isolation.
- [x] Add typed observation/wait/checkpoint/script IR; deterministic observations revalidate current session-bound target and bound each call to 10 seconds.
- [x] Add red-first compiler and JavaScript security tests for cross-session handle rejection, no loader/ambient network/file/process globals, loop interruption, memory/output caps, and workflow limits.
- [x] Add asynchronous MCP job admission, persisted 60-second deadline/status/checkpoints, resumable partial receipts, per-dispatch claims/counts, unknown delivery after timeout, exclusive state-directory lifetime lock, and startup recovery that preserves running checkpoints as unknown without replay. Concurrent clients need separate `CONTROLLA_STATE_DIR` values. Client disconnect does not cancel an accepted job; no workflow-cancel tool is exposed.
- [x] Add an opt-in local-trust JavaScript worker with only an async read broker. Each call rechecks principal/session/target and claims browser dispatch before execution.
- [x] Sol review found that an unresolved Promise bypassed the QuickJS interrupt callback. A bounded Tokio timeout now drops the active worker future; a red-first regression returns `script deadline exceeded` for `new Promise(()=>{})`.
- [x] Add one bounded inline artifact output for the gated sole-script step; artifact bytes and identity metadata persist in the durable job receipt and are returned by workflow_status/replay without filesystem writes.
- [ ] OS process isolation, untrusted/production script qualification, arbitrary mutation programs, file-backed/download artifacts, and representative app/platform qualification remain blocked.

### Phase 9 — local client setup and stdio conformance preview

- [x] Add dated local stdio config examples for Freebuff/Codebuff, OpenCode v1/v2, and Claude Code; mark ChatGPT local stdio unavailable: [clients.md](clients.md).
- [x] Add read-only `guide` MCP tool for `clients` and `master` topics, requiring exact server version `0.1.0`.
- [x] Replace the obsolete mutating workflow sample with an observe/checkpoint request and add a runnable JSON/request-shape check (`node tests/guide/check-master-example.mjs`). This is example-shape evidence only.
- [x] Add a checked versioned client-config generator with unique state directories; generated configs are still examples, not client acceptance.
- [x] Add `controlla://guide/{topic}/{server_version}` resources and a child-process RMCP test for initialize negotiation, resource list/read, all tool schemas, and stale/invalid request errors.
- [x] Add a child-process RMCP stdio check for the current no-initialize discovery lifecycle; it discovers the latest advertised protocol and exercises tool/resource listing over per-request metadata.
- [x] Negotiate every rmcp-advertised initialize revision through packaged stdio and verify tool discovery; keep this scoped to the pinned SDK/server's local stdio behavior.
- [x] Add a JSON config installer for Freebuff and OpenCode that preserves unrelated entries and refuses links, invalid JSON, and conflicting entries. Replacement is atomic and it best-effort detects edits before replacement; it does not serialize with arbitrary external writers. Claude Code prints a command for explicit human review; no client config outside temp fixtures was changed.
- [x] Generate the initialize-time bootstrap tool names from the MCP router and direct clients to `tools/list` for current argument schemas; a focused test checks every registered tool name appears and that the bootstrap remains short.
- [x] Run release-prep app, client, benchmark, docs, provenance, extension-command, package, and full Rust checks; 91 tests passed and 2 Chrome-required tests remained ignored in this environment.
- [x] Add an opt-in authenticated loopback Streamable HTTP endpoint with fixed endpoint audience, server-owned principal, Host/Origin validation, and bounded request bodies; in-process tests cover rejected requests, discovery, and a read-only guide call. No HTTP service process or external client was run.
- [ ] Run the service in the dedicated test setup and qualify real-client session/workflow/reconcile/artifact/cleanup behavior. Do not expose unauthenticated HTTP or raw CDP.
- [ ] Run real-client setup/workflow/cleanup acceptance for each available client and record installed version and server SHA.
- [x] Cover every rmcp-advertised initialize revision over packaged stdio; other transports remain unsupported. Run four five-task fresh-agent groups for B35/guide usability: 20/20 pass after guide corrections against current packaged schemas; no browser or app was connected.
- [ ] Generate full descriptive guide facts from runtime schemas and test every supported external client in a live client session.

**HTTP process smoke — 2026-10-07:** Started `serve-http` as a temporary local process on `127.0.0.1` with a random bearer, disposable state directory, and test principal. A separate HTTP harness completed MCP initialize, `tools/list`, and the master guide call (v3); a request without bearer returned 401. The process was stopped and its state removed. This verifies the live loopback listener and a bounded HTTP protocol exchange, not acceptance by OpenCode/Freebuff/Claude/ChatGPT or browser actions.

### Phase 10 — controlled comparison and optimization

- [x] Preregister 30 pilot templates (six each for generic, extraction, interference, multi-tab, and design), three planned repetitions, reset/predicate rules, seeded order randomization, outcomes, analysis, and a strict no-results status; validate with the Node built-in test runner and manifest checker.
- [x] Freeze a disjoint 100-template held-out split (20/category, 20 critical), preregister 5/10 repetition counts and task-clustered analysis, validate split and baseline-lock digests, and reject incomplete/unverified result rows. No results are present.
- [x] Add a deterministic offline task-harness contract with clone-based resets, bounded state edits, and independent predicate readback; the 12-test benchmark check passes.
- [x] Bind per-task reset/predicate fixtures to all 30 pilot templates; the offline harness validates clone resets, bounded synthetic actions, and independent state readback. These fixtures do not run Chrome, candidate tools, or measure task outcomes.
- [ ] Freeze supported model and baseline configurations before running the pilot. Exact model identity remains unresolved and no baseline has been installed.
- [ ] Execute the pilot and estimate variance against the already frozen held-out split. No benchmark task has been run and no comparison result is claimed; the held-out set cannot be changed after results are observed.
- [ ] Complete the 100 held-out templates × 5 resets, 20 critical workflows × 10 resets, and AGWC ablations; publish all failures and task-clustered results.

### Phase 12 — local release-candidate checks

- [x] Build the host release binary and stage the macOS arm64 npm package.
- [x] Verify Cargo archive contents/licenses, clean-prefix npm install with a path containing spaces and empty `PATH`, launcher version/help, and package metadata.
- [x] Add and run local package-manager lifecycle smoke for install, package-version upgrade, rollback, uninstall, and preservation of an unrelated user-data sentinel; distinct-binary mode checks installed executable digests at each version. Evidence: [distinct-binary lifecycle](review/phase12-binary-lifecycle.md).
- [x] Add a clean-commit release-candidate assembly path for host package and extension archives, runtime dependency SBOM, checksums, support matrix, release notes, and rollback instructions.
- [x] Extend its gate to run app-brief, client-config, pilot/held-out analysis, and Phase 6 comparison checks.
- [x] Add and fixture-test strict shared-extension/server version matching; pairing rejects missing/stale extension versions and the extension releases selected-tab debugger attachments on server mismatch.
- [x] Generate a local macOS arm64 release-candidate bundle from clean source commit `13c64e3`; verify its SBOM and all artifact checksums. The candidate is unpublished and not live-qualified.
- [x] Pass hosted Windows, macOS, and Linux build/package CI on candidate source commit `13c64e3`; run `37618717861` passed all three OS jobs. Published consumer install remains open.
- [x] Verify the host-matched npm package clean-prefix consumer install and command shim on Windows and Linux in hosted run [37628108314](https://github.com/Praket7/chrome-controlla/actions/runs/37628108314), alongside the macOS job. This installs the CI-built archive, not a published GitHub package.
- [x] Verify a local distinct-binary package upgrade/rollback with installed executable digests; this is not live MCP server/client compatibility. Evidence: [distinct-binary lifecycle](review/phase12-binary-lifecycle.md).
- [ ] Verify stale/current server-client compatibility beyond the extension handshake, signed release artifacts, and consumer installation from GitHub.
- [x] Run final review, commit all integrated changes, and push the authorized feature branch (`13c64e3` on `codex/phase4-5`).

The dated Phase 12 review record will bind any distinct binaries, full digests, source commits, and local lifecycle output. No registry publication was performed.

## Current Codex MCP connection diagnostic — 2026-10-07

The earlier direct Codex MCP entry failed during initialization. The local Hotload route now exposes and invokes all 20 Controlla tools from an isolated state directory. `guide` and `session discover` returned through this task. The shared-tab pairing race was reproduced: `pair_shared` bound a listener but deferred accepting until a separate blocking `accept_shared` call, so sequential popup use could time out. The listener now starts with `pair_shared` and remains available for five minutes; `accept_shared` returns a prompt waiting status when called early. A red-first regression, 190 workspace tests, Clippy, extension fixture, provenance, and documentation checks passed. Sol independently approved the pairing fix.

The rebuilt release binary was reloaded through Hotload without a Codex or extension restart. Chrome Controlla Shared Tab Bridge 0.1.0 then paired exactly tab `1649771543` in the authorized testing Classroom. `accept_shared` returned accepted, target inventory gave the requested Classroom URL, and `shared_observe` read `Classroom / testing` with stable root frame, loader, and URL. This verifies live read-only dispatch through the current Codex task. Shared input, app acceptance/persistence, broad client qualification, and the remaining build-plan gates are still open.

## Phase 12 current candidate follow-up — 2026-10-07

Source commit `b55499f103ebcbc0ed0085c82263b3545e8f1223` included the reviewed Phase 7 opaque artifact-integrity check. `npm run prepare:release-candidate` completed for macOS arm64; the package archive install, upgrade, rollback, uninstall, data-preservation, SBOM, and checksum checks passed. Candidate binary SHA-256: `e9080d9bcb89a6e514b184b0b75ca10c0a93a70cccf31d557d69bfc2a38bc9a7`. Hosted CI run [37684795222](https://github.com/Praket7/chrome-controlla/actions/runs/37684795222) passed on Ubuntu, macOS, and Windows. That candidate remained local and unpublished; no signing identity or consumer install from a published GitHub release was verified. The direct Codex MCP route then returned `Transport closed`; the working Hotload route and later live browser check above supersede that connection diagnostic. No restart is needed for the current Hotload route.

### Phase 11 — optional learning decision

No learning experiment was run. The build plan makes this stage optional and requires it to earn its runtime cost; no controlled comparison runs or consented test traces currently exist. Keep deterministic rules as the policy and revisit learning only after the Phase 10 pilot/held-out gates produce suitable evidence. This is a deferred experiment, not a completed learning feature.
