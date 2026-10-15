# Stage 1 — Reliable browser contracts and exact targets

> **For agentic workers:** Use superpowers:executing-plans to implement task by task. Checkboxes are completion evidence, not statements that the implementation already exists. `[x]` means the local implementation and its stated local evidence are complete; `[ ]` marks live qualification, manual reload, or an unrequested commit.

**Goal:** Make the existing six tools reliably address the intended tab, execute supported actions, and distinguish a failed action from missing evidence.

**Architecture:** A shared typed capability contract drives MCP validation, runtime dispatch, extension authorization, documentation, and transport tests. A target registry owns identity and presentation policy. Verification reads the requested application state rather than inferring success from transport acknowledgement.

**Tech stack:** Existing Rust/Tokio runtime, Chrome extension JavaScript, CDP, native messaging, Node test harness. Preserve dependency/license constraints and cross-platform packaging.

**Spec:** ../specs/2026-10-14-background-window-and-visual-control-design.md plus the user's 2026-10-14 capability-audit request. This plan supersedes that spec's optimistic “implemented at code/fixture level” qualification where the live failures below contradict it.

**State (2026-10-15):** Stage 1 code and wiring are implemented and locally validated, including the final exact packaged-extension asset identity and public foreground-open route regression. Configured extension/MCP reload and live qualification are intentionally still open under the user's no-live-test instruction. The matrix and progress ledger record local evidence separately from those live gates.

## Global constraints

- Keep browser, snapshot, act, workflow, extract, verify as the six eager tools.
- Ordinary tasks skip schema discovery and repository inspection. Resolve known targets internally.
- Exact existing-tab requests never silently create duplicates or choose another account/profile.
- Background means no momentary user-window focus or tab activation unless explicitly permitted.
- Preserve user-owned tabs, edits, focus, selection, clipboard, and downloads.
- A timeout after possible dispatch is unknown, never permission to repeat a write.
- Only Testing Google Classroom may receive test posts. The example assignment/deck is a workload specification, not authorization to submit it.
- Complete each task through its real public MCP route and loaded extension. Unit tests alone cannot close a transport defect.

## Current live evidence

Evidence: ../../../bench/reports/2026-10-14-capability-audit.json and ../../../bench/fixtures/capability-audit-20261014/.
Individual call samples below are not prompt-SEND timings, p50/p95 estimates, or competitor benchmarks.

| Case | Observation | Classification |
|---|---|---|
| Window discovery | 11 ms; existing tabs included real window IDs | Works; “no window identity” is too broad |
| Create owned background window | 442 ms, 2 commands; active tab inside unfocused window | Working sample |
| Background mouse click | 42 ms, 18 commands/17 host trips; trusted event and Count 1 | Working sample, excess transport |
| Background touch | 70 ms, 21 commands/20 host trips; trusted event | Working sample |
| Plain fill | 28 ms, 6 commands; correct value, synthetic input event | Working sample, distinct from trusted typing |
| Direct type/select/slider | Rejected by common value validator | Confirmed public-contract regression |
| Workflow exact find | Unread unrelated iframe makes root input unresolvable | Confirmed overly broad completeness gate |
| Shared screenshot | Canva and fixture rejected bounded capture | Confirmed runtime/extension mismatch |
| Shared upload | Registered bytes accepted, file selection refused | Confirmed capability mismatch |
| Headless upload | Server received 11 bytes; receipt only selected after 126 commands | Acceptance verifier misses ordinary status text |
| Headless research | example.com unknown, 7.422 s/67 commands; example.com/ completed, 0.919 s/35 commands | Exact URL equality bug; timings confounded by startup warmth |
| Foreground-mode open | Returned window_focused:false | Presentation contract not qualified |
| Release without mode | Failed for background/headless, succeeded for foreground | Session-derived mode missing |
| Canva page selection | Refused before dispatch, activation_forbidden, 55 ms/10 commands | Honest unsupported path; no seven-slide review |

The offscreen Bottom button really clicked. The audit initially checked the button instead of its adjacent output and caused an avoidable timeout. Independent server events and Chrome AX confirmed the effect. A later verify call used a CSS scope where the implementation expects a reference. Record these as audit/interface issues, not failed input dispatch.

## Review focus

1. Advertised argument accepted by schema but rejected by shared validation.
2. A status/output element excluded from the semantic control list.
3. An unrelated frame blocking a unique root control.
4. Same URL in multiple windows/accounts and target moving during dispatch.
5. A possibly delivered action followed by reconnection, cancellation, or human input.

## File ownership

- Modify crates/controlla-runtime/src/mcp/v3_full.rs: public validation, task routing, mode inference.
- Modify crates/controlla-runtime/src/mcp/v2.rs and v2_full.rs: predicates, snapshot/probe integration.
- Modify extensions/chrome-controlla/background.js: capability validation and exact presentation enforcement.
- Modify crates/controlla-runtime/src/capability_registry.rs: provider capability source of truth.
- Add focused modules under crates/controlla-runtime/src/ for target registry and predicate evidence only when they remove duplicated logic; do not rewrite unrelated code.
- Extend extensions/chrome-controlla/test-background.cjs and public-MCP contract tests.
- Update plugin instructions, docs/quickstart.md, docs/clients.md, and phase ledgers.

## S1-01 — One executable capability contract

**Interfaces:** CapabilityKey(provider, mode, action, version) -> Supported(parameters, effects, evidence) | Unsupported(reason, alternatives). Consume during tool validation and dispatch; do not make the model query it per task.

- [x] Add public-route/contract regressions for typed actions, screenshot routing, foreground presentation refusal, and session-derived mode; compare the advertised action/schema set with dispatch and extension authorization.
- [x] Replace the fill/secure_fill-only value check in act_inner with action-specific typed validation. Preserve credential restrictions without rejecting ordinary typing.
- [x] Mechanically compare schema, dispatch variants, extension allowlist, and instruction examples. Reject unsupported combinations before pairing/snapshot work.
- [x] Infer provider/mode from an existing session. Explicit contradictory mode is an error; omitted mode is not foreground by default.
- [x] Include runtime/extension protocol version and full runtime-asset build identity in initialization diagnostics, not every result. Detect incompatible pairs before mutations; tests prove Rust/extension digest parity and stale-build rejection.
- [x] Run Rust targeted/workspace tests, extension tests, and six-tool public contract smoke. Regression evidence is recorded in the verification matrix.
- [ ] Commit this unit only with regression evidence; no commit was requested or made.

**Acceptance:** All advertised actions reach the intended implementation. Wrong parameters yield invalid-argument receipts with no dispatch. Unsupported paths spend zero browser commands when support is statically known.

## S1-02 — Fix screenshot wire parameters and payload boundaries

**Source:** v2_full.rs visual_probe sends captureBeyondViewport:true; background.js bounded-PNG validator requires false. Lower-level observe.rs uses false, so testing that helper alone missed the live defect.

**Interface:** CaptureRequest(target, document, clip, max_bytes) -> ImageArtifact(hash, dimensions, bytes, captured_revision) or typed failure.

- [x] Replay the actual runtime-generated screenshot parameter object through the extension validator and Chrome-command mock using `runtime_generated_screenshot_request_passes_extension_validator`; the Rust test injects the production builder's JSON into the extension harness.
- [x] Align crop semantics, viewport bounds, scale, format, and captureBeyondViewport on the shared route. Return an MCP image block or an explicit artifact handle the client can consume.
- [x] Check nonempty decoded image, dimensions, hash, MIME, and byte counters. A successful JSON receipt without image data is failure.
- [x] Separate the current 900 KiB internal message limit from Chrome's directional limits. Add bounded chunk/artifact delivery with transfer IDs, length/hash validation, cancellation and cleanup; do not simply increase every limit.
- [x] Test maximum crop (1600×1200 logical pixels), high-DPI scale 2, oversize PNG, corrupt/truncated chunk, renderer navigation during capture, 15-second partial-transfer expiry, and chunked transfer through a running native-host process that remains available for the next command.
- [ ] Live-test fixture and Canva crop using the exact configured MCP after the user reloads the extension and restarts Codex.
- [ ] Commit only after both public routes return usable images; no commit was requested or made.

**Acceptance:** No allowlist mismatch, no silent zero-byte success, no native-host crash. Occlusion correctness is qualified in Stage 2.

## S1-03 — Verification that can see actual application state

**Source:** v2.rs TextPresent searches state.items (control text/name); ordinary output text is absent. A scope string is treated as a reference without validating CSS-vs-reference ambiguity.

**Interfaces:** PredicateTarget = Root | Reference | ScopedSelector | FrameReference, explicitly tagged. EvidenceResult = Proven | Disproven | Incomplete, with source/revision.

- [x] Reproduce status-output upload acceptance, noninteractive text, missing reference, wrong CSS scope, truncated text and unrelated unread frame in bounded local fixtures/tests.
- [x] Add bounded direct DOM/AX text readback for root and scoped text predicates. Do not use a list of actionable controls as a substitute for page text.
- [x] Validate selector/reference kinds before dispatch. Missing evidence must return incomplete, not definitive false.
- [x] Keep event receipt, selected file, app acceptance and persisted state distinct. On unknown upload reconcile once without reselecting/resending.
- [x] Register outcome observers before mutation; evaluate the strongest cheap independent readback in the same call.
- [x] Provide predicate-specific diagnostics: expected state, observed state, evidence completeness, and whether dispatch occurred, without leaking full page content.
- [x] Re-run local upload with independent server byte/hash evidence and both ordinary output and React-style status regions; server bytes and SHA-256 matched, acceptance was observed, persistence remained null.

**Acceptance:** The recorded 11-byte upload is recognized as application-accepted; no claim of persistence without persisted evidence. A wrong test selector remains a failed predicate, not a successful operation.

## S1-04 — Navigation equivalence and bounded readiness

**Source:** task_navigation_state_verified compares current_url == expected_url; task_navigate polls up to 20 times with 100 ms delays.

- [x] Add focused URL identity cases for root slash/default port equivalence and exact origin/path/query/fragment matching; malformed destinations fail closed. Redirect/login/SPA live-policy cases remain separate.
- [x] Normalize only URL equivalences that preserve resource identity. Redirect acceptance is explicit policy, not “any final URL is okay.”
- [x] Subscribe to document/lifecycle/navigation events before action. Use a bounded condition read for readiness; avoid universal network-idle waits.
- [x] Separate cold launch, attach, navigate, readiness, extraction and verifier deadlines.
- [x] Keep URL plus document identity and task postconditions. Navigation acknowledgement alone is not task success.
- [ ] Run randomized warm/cold slash/no-slash and redirect/login/SPA trials against the reloaded configured extension/MCP.

**Acceptance:** Both example.com forms complete identically when they resolve to the same intended resource. Unexpected account/origin redirects do not pass.

## S1-05 — Local completeness and compact evidence

**Interfaces:** Coverage(scope, frames, text, controls, limits); TargetMatch(reference, uniqueness_scope, document).

- [x] Reproduce exact root input selection while an unrelated cross-origin iframe is unread.
- [x] Require complete evidence only for the requested scope. Unique root actions can proceed; exhaustive newest-story or all-slide claims require complete relevant collection coverage.
- [x] Return actionable frame handles and reasons when needed content is in an unread frame.
- [x] Eliminate duplicate full items plus full delta upsert in the same default reply. Omit null/default fields; retain full evidence behind handles.
- [x] Make max_bytes govern final serialized response, not only intermediate text. Preserve explicit truncation and no false uniqueness.
- [x] Cover duplicate names, hidden controls, shadow roots, huge pages, advertisements, reorder and malformed page descriptions in local acceptance fixtures/tests.

**Acceptance:** Root workflow no longer blocked by an irrelevant iframe. Canva's 128,633-byte “compact” response becomes a bounded relevant projection without losing slide identity or coverage information.

## S1-06 — Exact target/window selection and reversible lifecycle

**Proposed typed browser operations within browser:** resolve_target, create_tab, create_window, move_tab, close_owned, release. Names must remain stable once shipped.

**TargetSpec:** existing_tab(tab_id, host_id, expected_url, account_hint?), existing_automation_window(window_id), create_background_window(placement?), or private_headless. No implicit fallback creation.

**TargetLease:** host/profile/tab/window/document IDs, owner, allowed presentation effects, revision, lifecycle.

- [x] Resolve explicitly supplied tab IDs first; URL matching uses bounded inventory and rejects ambiguous accounts/windows.
- [x] Cache event-updated topology; support caller-selected existing automation window and explicit placement. A tab moving windows updates identity without becoming another target.
- [x] Add close/move/activate operations with ownership rules, idempotency and independent resulting topology readback. Closing only audit-created tabs must be possible through Controlla.
- [x] Foreground mode verifies requested tab/window presentation; report inability rather than labeling an unfocused window foreground. Public route regression proves refusal stops before readiness/workflow dispatch.
- [x] Make release infer session mode; detach target-local resources without closing user tabs. Cleanup owned resources only when disposition permits and user has not adopted them.
- [x] Cover target ownership, duplicate target ambiguity, move/close/activate, replacement and stale receipts with local extension/public-route fixtures.
- [ ] Qualify profile ambiguity, real user drag-to-new-window, tab replacement, crash and service-worker restart in the reloaded configured extension.

**Acceptance:** Existing Canva is reused with no duplicate; background work can be placed in a selected separate window; all owned cleanup uses one call; unrelated bindings do not block target-local work.

## S1-07 — Deadline, interference and session recovery

**Files:** browser manager/scheduler, runtime native_host.rs/mcp.rs, extension waiters and session maps.

- [x] Propagate deadlines and operation IDs through queue/host/extension/CDP. Cancel queued work; fence late mutations after cancellation where dispatch has not occurred.
- [x] Record dispatch-possible vs definitely-not-dispatched; reconcile delivered-but-unacknowledged operations by ID.
- [x] Scope mutation serialization to target/document and resources. Independent tabs may progress concurrently; no global long-lived lock during network waits.
- [x] Preserve human edits when the same target changes; invalidate/reacquire once or yield. Another window's activity does not invalidate unrelated target work.
- [x] Bound reconnect, waiter cleanup, consumed pairing-token retention and failed attachment recovery. No global “release everything” requirement.
- [x] Cover cancellation fences, dispatch uncertainty, target-local contention and waiter cleanup in deterministic local tests.
- [ ] Qualify concurrent MCP processes, real human edits/focus interference, and extension/service recovery after the user reloads the configured runtime.

**Acceptance:** No blind retries, no late write after acknowledged cancellation, no unrelated-session deadlock or stale binding that requires manual release.

## S1-08 — Instructions and release gate

- [x] Single source for background/foreground semantics, password entry, upload scope, verification predicates and target reuse.
- [x] Remove stale guidance promising temporary activation/restoration in strict background mode.
- [x] Six compact schemas with concise routing examples; no routine list/describe/search step.
- [x] Run cargo workspace tests, extension background harness, Agent UX/client/docs checks and relevant public transport/package smoke contracts.
- [x] Build and locally package the candidate runtime/extension assets; clean-prefix package install/upgrade/rollback/uninstall and exact build identity checks pass.
- [x] Obtain independent fresh Luna Medium and Luna High whole-plan reviews confirming all code/wiring complete locally; the High review found screenshot coverage gaps, which were fixed and passed a new separate High re-review. Update progress, blockers, handoff and verification matrix with exact scope.
- [ ] Reload the configured unpacked extension and restart Codex MCP; record the six live tools and exact loaded identities.
- [ ] Complete live target, focus, screenshot, upload, navigation and interference regression evidence before marking Stage 1 exit complete.

## Stage exit

All S1 tasks must be implemented, wired and publicly exercised. The code/wiring and local acceptance scope is complete and independently reviewed. Stage exit remains open until the exact configured extension/MCP pair is reloaded and the live cases above pass; do not describe local completion as full Stage 1 exit.

## Research basis

- [Chrome debugger](https://developer.chrome.com/docs/extensions/reference/api/debugger) documents target/session routing and attachment constraints.
- [Native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging) documents asymmetric message limits; internal limits still require coordinated transport design.
- [Playwright actionability](https://playwright.dev/docs/actionability) supports action-specific preconditions, distinct from application outcome.
- [Windows](https://developer.chrome.com/docs/extensions/reference/api/windows) and [tabs](https://developer.chrome.com/docs/extensions/reference/api/tabs) distinguish window focus from tab activation.
