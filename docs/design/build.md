# Chrome Controlla implementation plan and autonomous build prompt

> **For agentic workers:** Use `superpowers:executing-plans` if available to implement this plan task by task. Use subagents only when the user or applicable instructions authorize them. Every unchecked item is unfinished; a written plan, scaffold, passing mock, or accepted command is not completion.

**Goal:** Create a separate Chrome-only repository and deliver a reliable, efficient browser MCP whose claims are supported by reproducible tests and real client/app acceptance.

**Architecture:** Extract the useful Comptrol Rust browser engine into a persistent broker. Add a typed workflow runtime, scoped script worker, unified capability registry, durable jobs, compact observations and independent outcome verification. Build Adaptive Guarded Workflow Compilation (AGWC) as an experimentally evaluated optimization over that sound foundation.

**Tech stack:** Rust/Tokio/CDP/SQLite; optional Chrome extension provider; bounded JavaScript worker; TypeScript fixture/reference runner using a pinned Playwright version; stdio and authenticated Streamable HTTP. Select exact supported runtime/SDK versions in Phase 0 and lock them. Do not assume an untested embedded JS engine is secure.

**Spec:** [research.md](research.md), [improvements.md](improvements.md), [MASTER_GUIDE.md](../MASTER_GUIDE.md). These design documents live under `docs/design/`; the master guide lives at `docs/MASTER_GUIDE.md`.

## Copy this assignment to the implementing AI

You are responsible for building **Chrome Controlla**, a new repository derived selectively from `https://github.com/Praket7/Comptrol`. Treat this document and its three companion documents as the project’s controlling specification, while respecting higher-priority instructions, user authorization, actual APIs and observed evidence. “Holy grail” means complete traceability and persistence, not permission to invent facts or follow an obsolete instruction after it is disproved.

Do the work, not merely another plan. Retrieve the source, reconcile its current revision with the researched baseline, create a clean Chrome-only repository, implement the phases, run meaningful tests, validate the actual clients and apps where credentials/environment permit, and produce reproducible release artifacts and a measured comparison. Continue fixing failed gates. Do not stop at a stub, mocked adapter, cosmetic rename, successful handshake or a flattering performance claim.

Aim for the most effective Chrome tool on the declared workload. You cannot prove universal superiority. Do not mark the project complete because you ran out of time or because one demo worked. If a necessary gate is blocked by unavailable credentials, hardware, paid entitlement, external approval or an unsupported product surface, document the exact blocker and completed independent work. Do not manufacture passing evidence, endlessly retry blocked work, or lower acceptance targets after seeing held-out results.

The master guide is part of the product. Every usage path, error and mode must tell a fresh AI agent exactly what to do next. Keep it consistent with the live schema and tested examples.

## Global constraints

- Chrome-only scope as defined in improvements.md; scoped browser-app adapters allowed, generic desktop/shell/native editor control excluded.
- Preserve source attribution and required license notices. Do not copy unrelated assets or secrets.
- Researched source SHA: `38a9d7eae0808a04b66e57d8658167c7667b4935`; do not assume it is current at execution time.
- Default dedicated profile; shared browser is explicit; strict background never silently activates OS focus/cursor/clipboard.
- Five proposed MCP tools: `session`, `observe`, `execute`, `jobs`, `guide`; changes require a written compatibility decision and guide updates.
- State and result enums, error names, limits and metrics follow improvements.md; version changes explicitly.
- No claim of exactly-once arbitrary web effects, ACID browser transactions, universal clipboard isolation, or immunity to human changes.
- No blind retry of uncertain effects. No page text can expand authority. No raw CDP endpoint exposed publicly.
- All performance includes setup, compilation, guard, verification, cache and recovery costs; report inner actions and model calls.
- Client/product/API compatibility is version-specific. Latest documentation can describe protocol changes; pin negotiated versions and test old/new client paths explicitly.
- No runtime training or workflow discovery on private user activity without opt-in. No live irreversible actions for benchmark convenience.
- Publishing packages, deploying a gateway and changing repo visibility follow the user’s actual authorization and environment policy. Prepare all reviewable work before asking for any missing final approval.

## Review focus

1. Crash after remote effect or dispatch claim but before journal completion: preserve unknown and refuse replay until an independent observer exists. Owned by Phase 3.
2. Human/account change after validation but before dispatch: narrow and measure the race; refuse stale subsequent work. Owned by Phase 4.
3. Apparently complete list with virtualized/hidden rows and stale counts: explicit partial coverage. Owned by Phase 5.
4. Canva sync writes changes while being used as a read probe: separate freshness and mutation flow. Owned by Phase 8.
5. Different client protocol/config and local versus remote Chrome identity: live client-specific contracts. Owned by Phase 9.

## Execution discipline and durable progress

Create `docs/progress.md`, `docs/decisions.md`, `docs/verification-matrix.md` and `docs/blockers.md` at the start. Track CC-01 through CC-24 and B01 through B36. Record tests with command, timestamp, commit, environment, result and evidence artifact, with secrets removed. Every phase closes with a focused commit and an updated matrix.

Before changing a component, write a meaningful failing test for its contract, observe the failure, implement the smallest sound solution, run the focused tests, and then applicable integration tests. Do not write tests that simply repeat the implementation or run every expensive suite after a documentation edit. Resume from the ledger after context loss. Never replace failed results with a fresh empty ledger.

Use approved existing permissions. Ask only for missing decisions or unavoidable access. Continue independent work while blocked. Do not create a recurring automation or infinite autonomous loop unless separately requested.

## Proposed repository structure

```text
chrome-controlla/
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  LICENSE
  NOTICE                         # if applicable to imported code
  README.md
  AGENTS.md
  crates/
    controlla-browser/src/{lib,connection,targets,frames,input,observe}.rs
    controlla-runtime/src/{lib,registry,policy,identity,journal,jobs,scheduler,compiler,verifier,extract,cache,bridge,artifacts}.rs
    controlla-mcp/src/{main,stdio,http,tools,errors}.rs
    controlla-cli/src/{main,args,doctor}.rs
    controlla-script/src/{main,protocol,limits}.rs
  extensions/chrome-controlla/
  packages/chrome-controlla/       # distribution launcher and platform artifacts
  sdk/                           # typed broker calls available to scripts
  schemas/{capabilities,workflow,result,errors}.json
  apps/{slides,canva,capcut-web}/
  docs/{MASTER_GUIDE,progress,decisions,verification-matrix,blockers}.md
  docs/design/{research,improvements,build}.md
  docs/recipes/
  integrations/{freebuff,opencode,claude-code,chatgpt}/
  tests/{fixtures,e2e,clients,security,guide}/
  bench/{tasks,baselines,analysis,reports}/
  scripts/{check,docs-check,package-check}.sh
  provenance/extraction.json
  .github/workflows/{ci,release}.yml
```

These are planned paths, not a demand to create empty modules before use. Consolidate small modules if it improves clarity while preserving contract ownership and documentation. The initial browser extraction can retain upstream filenames until parity is established.

## Shared interface contract

Define serializable types once in the runtime and generate JSON schemas/SDK types. IDs are opaque server-issued values, not caller-chosen browser authorities.

```text
SessionSpec { mode, background, profile_ref?, allowed_origins, limits }
SessionHandle { session_id, principal_id, browser_instance, capability_revision }
TargetRef { session_id, target_id, navigation_epoch, frame_id?, document_id?, account_id? }
CapabilityContext { principal, session, target?, policy_revision }
CapabilityDecision { supported, configured, reachable, authorized, qualified, reason, revision }
WorkflowRequest { session_id, idempotency_key, target, workflow|script|workflow_ref, limits, postconditions }
WorkflowGraph { nodes, edges, declared_resources, version }
OperationReceipt { operation_id, status, delivery, verification, replayed, target, evidence, metrics, cleanup }
ExtractionResult { records|artifact_id, schema, source, filters, unique_count, expected_count?, completeness, end_evidence, missing }
```

Make workflow/script/workflow_ref mutually exclusive in schema. Validate unknown fields and limits. Encode principal identity from authenticated transport, never from an untrusted request body. Define artifact handles as principal/session scoped. `completed` must imply that the requested postconditions are satisfied at the declared evidence level.

## Phase 0 — retrieve, reconcile, and establish the new repository

**Files:** `provenance/extraction.json`, `docs/decisions.md`, the four progress files, root manifests/license and copied specs.

**Consumes:** upstream Git URL and researched SHA. **Produces:** clean Chrome-only repo, pinned toolchain and source inventory.

- [x] Inspect workspace instructions, Git identity/auth and existing repositories before cloning. Do not overwrite a user checkout or discard uncommitted changes.
- [x] Clone upstream into a separate source directory. Record fetched HEAD, remote URL and dirty state. Compare browser/core/bridge/package changes to the researched SHA; determine whether reported regressions correspond to an older binary or another route.
- [x] Inspect licenses and dependencies. Create a new local repository named `chrome-controlla`, preserving notices and provenance, with no credentials or browser profiles.
- [x] If executing this prompt includes authorized GitHub repository creation, use the authenticated user's intended owner and create **a separate repository**, never rename/delete Comptrol. Default private when visibility is unspecified; record that choice. If the name exists, inspect ownership/content and stop before overwriting. Do not guess an alternate name or push into an unrelated repo.
- [x] Select and pin Rust, Node/reference runner, MCP SDK/protocol and Chrome for Testing versions supported by the environment. Document rationale and supported OS targets.
- [x] Extract minimal browser engine and tests, not the monolithic app registry. Create provenance mapping and a dependency check that fails if native desktop platform crates/adapters re-enter the distribution.
- [x] Add a CI baseline: formatting, static checks, focused Rust tests, fixtures, package/docs checks. Confirm a clean checkout can build without upstream globally installed.
- [x] Commit `chore: establish Chrome-only source extraction and provenance`.

**Gate:** actual source reconciliation and clean build, not merely a GitHub repo URL. Owner/visibility/publication decisions are recorded. No implementation superiority claim.

## Phase 1 — one registry, reliable CLI and installation

**Files:** `registry.rs`, `policy.rs`, CLI `args.rs`/`doctor.rs`, schemas, package launcher, `tests/e2e/registry.rs`, `tests/e2e/cli_help.rs`, `tests/e2e/installation.*`.

**Interfaces:** `evaluate_capability(ctx: &CapabilityContext, action: &str) -> CapabilityDecision`; `doctor(config: &Config) -> DiagnosticReport`; dispatch consumes the same decision function.

- [x] Write tests for CC-01–04 and B01–04: direct-only, bridge-only, unconfigured, denied, revoked, minimal PATH, and help with open stdin.
- [x] Implement early argument parsing. Help/version/list-schema never initialize browser runtime or mutate state. Invalid flags fail clearly.
- [x] Implement one evaluator with structured reasons; catalog is a snapshot, dispatch reevaluates. Add principal and policy revision to diagnostic correlation without secrets.
- [x] Report heartbeat and authenticated round-trip separately. Preserve storage/path/permission errors instead of mapping all to false. Test different process environments and daemon state paths without claiming one is the tester's proven cause.
- [x] Resolve the launcher relative to the installed package or explicit path. Make cold install failures distinguish download, architecture, permission and missing executable.
- [x] Run focused tests and clean-install smoke; inspect packaged contents.
- [x] Commit `feat: unify capability decisions and standalone CLI packaging`.

**Gate:** catalog/dispatch matrix agrees under the same context; every help path exits without side effects; install works without Comptrol.

## Phase 2 — persistent sessions, modes, identities and tabs

**Files:** browser `connection.rs`, `targets.rs`, `frames.rs`; runtime `identity.rs`, `scheduler.rs`, `bridge.rs`; extension provider; session tool; B08/B13/B20/B21/B26 fixtures.

**Interfaces:** `create_session(spec, principal) -> SessionHandle`; `resolve_target(handle: &TargetRef) -> Result<ResolvedTarget, StaleTarget>`; `release_session(id, cleanup_policy) -> CleanupReceipt`.

- [x] Test target closure/reuse, browser reconnect, navigation epochs, same-process frames and OOPIF swaps before porting/reworking handlers.
- [x] Retain persistent connections and event-driven target tracking. Invalidate handles on the relevant generation change; never replace target ID with active-tab selection.
- [x] Implement dedicated headed/headless providers and explicit shared extension attachment. Enforce grants independently for each provider. Live extension attachment remains unqualified.
- [x] Add owned/borrowed/adopted tab ledger and cleanup receipts. Preserve user tabs and changed ownership, including after crashes.
- [x] Use per-target actors plus declared shared-resource locks. Run 1/4/8-tab fixture tests; prove a hung page does not stall all tabs.
- [ ] Qualify platform focus/cursor/clipboard behavior with an independent observer. Unsupported platform/mode cells remain unqualified.
- [x] Commit `feat: isolate Chrome sessions and track tab ownership` (`f0fd586`, reviewed and pushed; hosted CI passed).

**Gate:** correct target identity and cleanup, with honest platform boundaries. Headless Chrome availability is not headless Canva/CapCut qualification.

The local/code review gate passed on 2026-10-06. Live extension attachment, independent in-tab cleanup observation, headed Chrome behavior, and native focus/cursor/clipboard observation remain unqualified; the target inventory check is a point-in-time snapshot.

## Phase 3 — jobs, durable idempotency and uncertain outcomes

**Files:** `journal.rs`, `jobs.rs`, `errors.rs`, jobs tool; `tests/e2e/{idempotency,jobs,crash_recovery}.rs`.

**Interfaces:** `admit(principal, session, request, key) -> Admission`; `record_dispatch(principal, session, op, correlation) -> DispatchClaim { acquired, operation }`; `reconcile(principal, session, op_id) -> Operation`; `wait(principal, session, op_id, after_revision, max_wait_ms) -> Operation`.

- [x] Implement local journal contracts for B05–07/B18–19; controlled fixture counting covers duplicate prevention after an accepted effect. Workflow dispatch is exercised through local MCP fixtures; no live browser or external HTTP-client effect is claimed.
- [x] Bind canonical JSON request identity to principal/session/key. Same key/body replays the original operation; changed body conflicts.
- [x] Use SQLite immediate transactions and unique constraints. Separate-connection simultaneous admission and dispatch-claim tests permit one sender; operations are scoped by principal/session on reads and mutations.
- [x] Separate bounded caller wait from durable operation state, persist a bounded 60-second default job deadline, and expose monotonic revisions. The workflow worker consumes declared step, time, observation, and output bounds.
- [x] Exercise fulfilled, rejected, delayed, and never-settling JavaScript promises through the pinned QuickJS async broker, including rejection propagation to scripts. This is local worker evidence only.
- [x] Verify a broker-backed QuickJS Promise resolving after exactly 12 seconds through the local worker wrapper. This does not qualify browser routes.
- [x] Kill/reopen subprocesses at seven durable journal fixture cut points (admitted, running/not-sent, claim-before-effect, effect-before-ack, acknowledged, checkpointed, completed); confirm recovered state and replay refusal. This is journal-fixture evidence, not live Chrome dispatch or every runtime boundary.
- [ ] Qualify exact 12-second resolve/reject and navigation during evaluation through live CDP and extension-bridge browser routes; inject kills at remaining runtime/browser dispatch boundaries.
- [x] Cancellation blocks queued dispatch; expiry before dispatch becomes `failed` with `deadline_error=deadline_exceeded` and `delivery=not_sent`. Expiry/recovery after dispatch becomes `unknown` while preserving delivery as `sent` only when transport acknowledgement was recorded, otherwise `unknown`.
- [x] Commit reviewed local journal gate as `feat: persist browser jobs and reconcile uncertain effects`.

**Gate:** local journal fixtures refuse duplicate dispatch on replay, including seven tested durable cut points, and the QuickJS worker exercises async Promise outcomes. Remote exactly-once, live browser Promise parity, navigation invalidation during evaluation, and untested runtime/browser process-kill boundaries remain unqualified.

## Phase 4 — reliable input, guards and interference

**Files:** `input.rs`, runtime `identity.rs`/`policy.rs`, `tests/fixtures/input/`, `tests/e2e/interference.rs`.

**Interfaces:** `validate_step(target, dependencies) -> GuardDecision`; `perform_input(target, InputAction) -> DispatchEvidence`; `on_external_change(event) -> InvalidationSet`.

- [x] Add B09–13/B22–25/B36 local fixtures for account/dependency changes, navigation, Unicode, explicitly marked masked/event-dependent controls, contenteditable, overlay interception, DOM drag, strict-background text, and foreground-required mouse/native routes. Installed Chrome covers the supported DOM controls; a separate read-only macOS observer records frontmost app, cursor, and clipboard change count.
- [x] Implement fail-closed semantic locators, typed fill/insert/sequential keys/click/drag routes, value/caret postconditions, geometry/hit checks, and caller-supplied navigation/account/document/dependency revisions. Application identity and edit observers remain adapter work.
- [x] Inject navigation between final validation and dispatch; verify the write is withheld. Record mock protocol timing and the remaining non-atomic CDP/page/server race; mock timing is not a real Chrome bound.
- [x] Implement a bounded per-session in-memory text clipboard and guarded insertion through CDP; strict-background mode never reads/writes the OS clipboard. Never disable human input to preserve a lease.
- [x] Add bounded artifact-byte registration and opaque session/principal-scoped handles; select a verified `input[type=file]` through guarded CDP using private temporary files. Return `Selected` evidence only. Caller host paths are never accepted, and app acceptance/persistence is not claimed.
- [x] Run isolated installed-Chrome fixtures for ordinary inputs, fail-closed masked/event-dependent markers, contenteditable/password, overlay interception, DOM drag bounds, stale-value interference, and guarded IME composition. A native snapshot around strict-background text confirms unchanged foreground app, cursor, and clipboard change count on this macOS host.
- [x] Repeat the installed-Chrome focus-handler interference fixture three times; each guarded write yields and preserves the externally changed value. This remains controlled fixture evidence.
- [ ] Measure the real Chrome check-to-dispatch window and qualify repeated live-user interference; the websocket fixture measures only its mock protocol window.
- [x] Commit Phase 4 implementation and fixture work (`e1a8f10`, `197b8a6`, `8784f1b`, `d211c50`, `e69674b`, `a50d088`).

**Gate:** relevant detectable interference yields before subsequent mutation; input fixtures verify actual values/positions. No “interference-proof” claim.

## Phase 5 — compact observations and complete extraction

**Files:** `observe.rs`, `extract.rs`, observe tool, virtual-list fixtures, `tests/e2e/extraction.rs`.

**Interfaces:** `observe(target, selector, fields, budget) -> Observation`; `extract(target, ExtractionSpec) -> ExtractionResult`.

- [x] Create policy fixtures for 42 virtualized records, 21-item separate bucket, blocked expansion, recycled nodes, duplicate labels, stale count, infinite feed and wrong-account 404; add a real Chrome virtualized 42-record run.
- [x] Implement bounded CSS/DOM observations and schema-guided extraction with stable-ID deduplication, caller-declared multi-section traversal, explicit terminal evidence, freshness, omissions, and truncation fields. Add selected-node partial AX, byte-preflighted PNG crops, bounded single-use resumable cursors, and declared hidden-section expansion checks.
- [x] Test `complete`, `partial`, and `unknown` against fixture evidence; count and repeated no-change alone do not certify completion.
- [x] Record a single synthetic payload/latency comparison as a diagnostic only; repeated and representative performance qualification remains open.
- [x] Verify Phase 5 features with mocked MCP/CDP tests and an isolated Chrome synthetic page covering AX, crop, resume/stale cursor, and expanded/blocked sections. This is fixture evidence, not broad app qualification.
- [x] Commit Phase 5 implementation and fixture work (`c656c1c`, `44041af`, `1fd49f9`).

**Gate:** all records or explicit missing coverage; never the tester’s visible-row-only false completeness.

## Phase 6 — constrained scripts and guarded workflow compiler

**Files:** `controlla-script`, `sdk/`, `compiler.rs`, `scheduler.rs`, workflow schema, execute tool, script/security tests.

**Interfaces:** `compile(request, capabilities) -> WorkflowGraph`; `run(graph, budget) -> OperationReceipt`; broker API `call(session_handle, operation, args) -> TypedResult`.

- [x] Spike candidate runtimes for interruption, memory bounds and asynchronous host calls. Wasmtime remains a WebAssembly engine, not a JavaScript runtime. A runnable pinned `rquickjs 0.14.0` probe and official crate API confirm interrupt handlers, memory/stack limits and futures-based async host calls. See [rquickjs 0.14.0 API](https://docs.rs/rquickjs/0.14.0/rquickjs/), [Wasmtime interruption](https://docs.wasmtime.dev/examples-interrupting-wasm.html), and [Wasmtime ResourceLimiter](https://docs.wasmtime.dev/api/wasmtime/trait.ResourceLimiter.html). `node:vm` is not treated as a security boundary.
- [x] Add red-first compiler and worker tests for cross-session handles, unavailable imports and ambient file/network/process access, loop interruption, heap/output limits, and workflow step/wait/output bounds.
- [x] Enforce the script deadline around pending async JavaScript as well as active bytecode; a never-settling Promise regression returns an error and releases the worker future.
- [x] Implement typed bounded graph/script IR and a fresh-context QuickJS worker. Its only host function is an asynchronous, per-call reauthorized read broker; scripts are local-trust-only and disabled unless `CHROME_CONTROLLA_ENABLE_TRUSTED_SCRIPTS=1`. QuickJS runs in a bounded child process with a cleared environment, but no kernel sandbox or OS RSS/CPU quota is enforced; untrusted and production scripts remain gated pending those controls and platform qualification.
- [x] Add bounded asynchronous MCP admission, a 60-second durable job deadline, resumable `workflow_status`, persistent checkpoints/partial receipts, pre-dispatch operation claims/counting, unknown delivery after timeout, and receipts bound to operation, target, and revisions. Startup takes an exclusive state-directory lifetime lock before recovery: accepted and running/not-sent work fails as not sent; running work after a dispatch claim becomes unknown while preserving delivery/checkpoints; browser effects are never retried. Script broker calls reserve their declared output budget against the workflow-wide cap before dispatch. Client disconnect does not cancel an accepted job; there is no workflow-cancel tool.
- [x] Accept one explicit {kind:"artifact",filename,media_type,bytes:[...]} return from the gated sole-script workflow step. Validate exact fields, filename, allowlisted media types, and 1–12 KiB byte bounds; store bytes, SHA-256, and operation/principal/session binding inline in the existing journal receipt for `workflow_status` retrieval and idempotent replay. No artifact file is written.
- [x] Add dependency-scoped invalidation and effect-aware splitting as deterministic planning rules. The fixture covers direct/transitive dependents and isolates non-read effects; declared effects remain hints, not proof that page code has no side effects.
- [x] Add a deterministic fixture comparison for fixed batching, bounded code, and compiler boundaries. It reports operation/recovery counts only; browser overhead and wall-clock profiling remain open.
- [x] Force-kill a subprocess after two journaled fixture dispatch claims, reopen the journal, and verify the checkpoint and unknown delivery survive while replay is rejected. This is journal/claim-layer recovery evidence only; it does not kill the MCP service during a real Chrome dispatch.
- [x] Commit the Phase 6 implementation and review fixes (`3271bb6`, bounded deterministic workflows; `5ce2100`, async receipts; `dc16a0d`, aggregate output and restart recovery; `1831f9a`, journal ownership lock).

**Gate:** a useful deterministic read graph, dependency-scoped invalidation/effect-aware planning rules, count-only fixture comparison, gated trusted-local JavaScript path, and bounded inline script artifacts are implemented through the stdio MCP server. The QuickJS child process is not qualified as an OS security boundary, so untrusted/production script execution remains blocked. Arbitrary writes, mutation batching, file-backed/download artifacts, measured browser overhead, and OS-level resource isolation are not claimed.

## Phase 7 — outcome verification and workflow cache

**Files:** `verifier.rs`, `cache.rs`, `artifacts.rs`, B28–30 tests, rubric schemas.

**Interfaces:** `verify(operation_binding, predicate, evidence) -> VerificationResult`; `qualify(definition, observed_suite) -> opaque QualificationToken`; `insert(token)`; `lookup(workflow_id, current_definition, now) -> QualifiedWorkflow | Miss | Quarantined`; `quarantine(id, reason)`.

- [x] Add offline independent-state fixtures that reject a persuasive fake save against stale ground truth, old revision/stale observation, truncated known artifact, and changed control semantics. The visual predicate stays inconclusive; this is synthetic fixture evidence, not a production observer.
- [ ] Test old screenshot claims and representative app save/download outcomes through a production independent observer; no safe observer or release suite exists yet.
- [ ] Implement a production outcome observer for independent field/object/state evidence. The local verifier now evaluates predicates only after a sealed runtime-observer receipt; its issuer exists only in tests, so caller-supplied evidence remains `inconclusive` and production observations cannot pass. Visual predicates remain inconclusive.
- [x] Bind evidence claims to operation/provenance ID, principal/session/target/app/account/revision, observer label, time and predicate hash. These fields do not establish observer trust.
- [x] Add `artifact_verify` for an opaque registered artifact: runtime re-reads its private session file and compares byte length/SHA-256 with the registration-time receipt. This verifies local staging integrity only; it is not an app-state or download observer.
- [x] Implement the canonical cache contract and in-memory quarantine/expiry mechanics. Insert/unquarantine requires an opaque successful training/validation token; no production observer or suite runner can mint one yet, so production cache admission is unavailable.
- [ ] Validate cold/warm performance and include preparation/recovery costs. Current fixture tests establish lookup behavior only; a historical success never authorizes a present mutation.
- [x] Commit `feat: verify outcomes and retire stale workflows` (local fixture implementation; release gate remains open).

**Gate:** no release-suite false completion; visual/aesthetic uncertainty is preserved, not coerced into pass.

**Pre-live app planning slice (2026-10-06):** read-only MCP tools compile a 10-slide revision-bound Slides request, a five-page identity/session/version-bound Canva design plan, and a CapCut timeline recipe validated against caller-supplied licensed-asset metadata. The fixed proposals are unit- and stdio-tested, but none connects to vendor APIs or mutates a browser. They do not provide authoritative identity/session observation, OAuth, edit dispatch, app readback, persistence, export, or quality review. Acceptance briefs and separate correctness/visual rubrics are present under `apps/`; all live evidence remains pending. See `docs/review/phase8-offline-planners.md`.

## Phase 8 — qualify professional web-app workflows

**Files:** `apps/slides/`, `apps/canva/`, `apps/capcut-web/`, app fixtures, live evidence manifests, guide recipes.

**Interfaces:** every app module exports `capabilities(context)`, `plan(task, exact_document)`, `execute(plan)` and `verify(outcome)` through the same runtime policy/journal; no bypass channel.

- [ ] Implement Slides supported object operations with explicit OAuth, document/object IDs and revision controls. Keep browser-only and API-assisted routes distinct in results.
- [ ] Implement Canva scoped Connect operations and supported Apps SDK bridge. Test one-minute session expiry, unsupported/locked pages, and sync-as-write behavior. Stub tests are insufficient for live qualification.
- [ ] Implement bounded CapCut Web UI recipes for known media import, trimming/splitting, caption correction, basic timeline and export. If no route supports an operation, report it unsupported; do not invent an official API.
- [ ] Create representative design briefs: a 10-slide editable deck; a consistent multi-page Canva set; a short captioned video. Use owned test content and test documents.
- [ ] Read back structure, verify persistence after reload, export, and inspect every affected page or relevant video segment. Test collaborator changes and wrong project/account.
- [ ] Record task-specific design rubric and blinded quality review. Correct controls and aesthetic quality get separate scores.
- [ ] Qualify foreground/background/headless and each supported platform/entitlement; mark blocked cells explicitly.
- [ ] Commit `feat: qualify scoped Slides Canva and CapCut Web workflows` only for implemented features, with accurate per-app status.

**Gate:** no broad professional-app claim based solely on fixtures, template fill or a successful export-job submission. Missing credentials block live qualification, not all other work.

## Phase 9 — all clients and the master guide

**Files:** MCP stdio/http, `integrations/`, `docs/MASTER_GUIDE.md`, recipes, `tests/clients/`, `tests/guide/`, generated schemas/help.

**Interfaces:** identical operation/result semantics across supported transports; `guide(topic, server_version) -> GuideSection`; versioned client config generators.

**Current local slice (2026-10-07):** `docs/clients.md` records local stdio examples for Freebuff/Codebuff, OpenCode v1/v2, and Claude Code, plus ChatGPT's local-stdio limitation. A read-only `guide` tool and canonical `controlla://guide/{topic}/{server_version}` resources serve `clients` and `master` Markdown for exact server version `0.1.0`; pinned rmcp child-process tests exercise every initialize revision advertised by the pinned SDK and its current discovery lifecycle, resource listing/reading, tool schemas, and invalid requests over stdio. A four-group, 20-task fresh-agent B35 run passed against the packaged local MCP schemas after guide corrections. An opt-in authenticated Streamable HTTP endpoint is implemented for loopback only. A 2026-10-07 bounded live process smoke completed HTTP initialize, tool discovery, a versioned guide call, and missing-bearer rejection; OpenCode 1.18.5 separately connected over stdio and discovered tools, without a tool action. No supported external-client tool action or HTTP-client acceptance is claimed.

The B35 task descriptions, individual outcomes, schema evidence and limitations are recorded in [route evaluation](../review/phase9-b35-routes.md), [professional-app cases](../review/phase9-b35-apps.md), [long-task cases](../review/phase9-b35-long-tasks.md), and [extraction/file-selection cases](../review/phase9-b35-extract-file.md). They are task/evaluation records, not live browser or client acceptance.

- [x] Cover every initialize revision advertised by pinned rmcp and its latest discovery lifecycle over packaged local stdio, including tool discovery checks; other transports and external-client behavior remain open. Do not mix latest discovery semantics with legacy initialize assumptions.
- [x] Add an opt-in authenticated loopback Streamable HTTP endpoint with endpoint-bound audience, server-owned principal, Host/Origin checks, bounded bodies, and MCP protocol validation. It binds only to `127.0.0.1` and is started only by the explicit `serve-http` CLI command.
- [x] Run a bounded authenticated loopback service process and verify initialize, tool discovery, a versioned guide call, and missing-bearer rejection. This is a protocol harness, not client acceptance.
- [ ] Run supported external-client tool actions and HTTP-client acceptance; implement/qualify an optional paired outbound bridge. Do not expose public unauthenticated browser control.
- [x] Record dated local stdio setup examples for Freebuff/Codebuff, OpenCode v1/v2, Claude Code, and ChatGPT's local-process limitation in `docs/clients.md`; examples are schema-checked but installed client versions and client acceptance remain open.
- [x] Generate dated client-specific configuration examples with per-client state isolation and validate their shapes against the checked-in schemas. Installed versions and client acceptance remain open; do not assume every platform has the same config nesting or local process support.
- [x] Add an installer for JSON-based Freebuff/OpenCode configuration files; it preserves unrelated entries and refuses ambiguous or conflicting entries. It atomically replaces the file and best-effort detects edits before replacement, but does not serialize with arbitrary external writers. Claude's CLI command remains a human-reviewed output, and no live client config was changed.
- [ ] In each available real client, run session setup, observation, guarded edit, long job, reconnect/reconcile, structured result/artifact and cleanup. Record client version and server SHA. A raw JSON-RPC smoke alone does not qualify a client.
- [x] Turn the companion master guide into runnable setup, observation, extraction/resume, workflow-polling, and artifact/file-selection examples. Checks parse examples and verify supported names. The initialize bootstrap lists runtime-derived tool names and directs clients to `tools/list` for schemas. Descriptive facts remain partly human-maintained. Version-bound `guide` tool and `controlla://guide/{topic}/{server_version}` resources serve `clients` and `master`.
- [x] Run the 20-task fresh-agent B35 usability suite in four five-task groups; 20/20 passed after guide corrections. Evaluators queried packaged MCP schemas over isolated stdio and did not connect a browser or app. Real-client acceptance remains open.
- [ ] Commit `feat: qualify MCP clients and ship executable master guide`.

**Gate:** each advertised client has live evidence or is explicitly marked unavailable/unqualified; guide and schemas agree.

## Phase 10 — controlled competition and optimization

**Files:** `bench/tasks`, `bench/baselines`, `bench/analysis`, immutable run manifests, `bench/reports/`.

- [x] Pin exact Playwright MCP, Chrome DevTools MCP, Stagehand, Browser Use and Vercel agent-browser package artifacts/source revisions in `bench/baselines/versions.lock.json`. The pins are not installations or benchmark runs.
- [ ] Freeze the supported model/browser configuration and install supported baselines before the pilot; model identity/version is unresolved. Keep batching/caching available and avoid mismatched model configurations.
- [ ] Where actual Computer plugin/Claude Chrome/Cowork access is available, define a documented end-to-end track. If they cannot be run, mark missing instead of inventing results.
- [ ] Run the 30-template pilot, estimate variance, preregister primary metrics/thresholds and held-out split. Include generic tasks, extraction, interference, multi-tab and design tasks; disclose design-review sample sizes separately.
- [x] Bind offline clone-reset and independent readback-predicate fixtures to each of the 30 preregistered pilot templates. These validate harness contracts only; they do not run the candidate, Chrome, a model, or visual review.
- [ ] Run at least 100 held-out task templates with five reset runs each for controlled comparison, plus 20 critical workflows × 10 independent runs. If sample size/power is inadequate, expand before claiming a win.
- [ ] Produce task-clustered intervals, success noninferiority analysis, p50/p95 latency, tokens, model/MCP/browser calls, failed runs, interventions, focus/clipboard disruption, cleanup and cold/warm cache costs.
- [ ] Run the AGWC ablations from research.md. Regressions require fixes or removal of the optimization, not selective reporting.
- [ ] Profile the largest measured bottleneck. Examples: excessive observation bytes, round trips, global locking, redundant verification, slow reconnect. Do not assume Rust is faster enough to dominate model latency.
- [ ] Commit `bench: publish reproducible browser task comparisons`.

**Gate:** claim only the measured workload/version result. Proposed target: ≥30% fewer model round trips, ≥25% lower median time, success noninferiority within two percentage points, relevant confidence intervals, and no severe authority/wrong-target violation in the release suite. A missed target remains a missed target.

## Phase 11 — optional learning experiments

**Files:** `experiments/scheduler/`, `experiments/world-model/`, dataset manifests, evaluation reports. Keep optional dependencies out of the default runtime.

- [ ] Obtain consented synthetic/test-account traces; redact secrets; separate train/validation/test by task/site/template/time where practical.
- [ ] Start with a contextual bandit choosing among already authorized routes and checkpoint policies. Compare against strong deterministic rules.
- [ ] Add transition-focused world-model predictions only where uncertainty justifies their inference cost. Predictions may help select an action or observation, never satisfy an actual postcondition.
- [ ] If foundation-model RL is justified, document model license, training/data rights, hardware, budget, verifiable reward construction and independent evaluator. Use safe fixtures; prohibit reward hacking via suppressed errors or omitted verification.
- [ ] Optimize verified outcome with penalties for cost, interference, wrong target and false completion. Hard policy constraints cannot be traded for reward.
- [ ] Deploy in shadow mode; promote only after held-out wins and safety non-regression. Keep a rules-only fallback and immediate quarantine mechanism.
- [ ] Commit experiments and report positive or negative results honestly.

**Gate:** optional learning must earn its cost. Failure here does not justify pretending the deterministic product is unfinished; report the experimental result and retain the better architecture. Likewise, a good RL score cannot excuse an unreliable executor.

## Phase 12 — release and final handoff

**Files:** release workflow, package manifests, checksums/SBOM, support matrix, release notes, benchmark report, guide, rollback instructions.

- [x] Run the configured Rust, dependency, client, app-brief, benchmark, docs, provenance, extension, and package checks on the candidate source commit. Hosted run [37618717861](https://github.com/Praket7/chrome-controlla/actions/runs/37618717861) passed Windows, macOS, and Linux build/package jobs for `13c64e3`.
- [x] Test clean-prefix installation, package-version upgrade/rollback, uninstall, and unrelated user-data preservation on the current host. The lifecycle harness supports distinct binaries and verifies the installed executable digest at each version; see the dated evidence record for exact revisions and digests.
- [x] Prepare the local release-candidate bundle path: host package and extension archives, runtime dependency SBOM, artifact checksums, support matrix, release notes, and rollback instructions. The bundle remains unpublished and must be generated from a clean release commit.
- [x] Generate the unpublished macOS arm64 release-candidate bundle from clean source commit `2f4fb06`; verify the recorded SBOM and archive checksums. This does not qualify other hosts or consumer installation.
- [x] Reject stale or missing shared-extension versions during pairing and verify the extension releases its debugger attachment on mismatch. This local fixture does not qualify release-binary upgrade compatibility.
- [x] Test a local distinct-binary package upgrade/rollback on the current host, checking installed executable digests and user-data preservation. The macOS arm64 lifecycle passed; see [distinct-binary lifecycle evidence](../review/phase12-binary-lifecycle.md). This does not establish live MCP client/server compatibility.
- [x] Generate an unpublished local macOS arm64 candidate from clean commit `13c64e3`; verify npm package, extension, and SBOM checksums. The package remains unpublished and live acceptance is not run.
- [x] Verify fresh clean-prefix consumer installs on Windows, macOS, and Linux through `package-check.sh`, which installs each host-matched archive and runs the installed command shim. Hosted run [37628108314](https://github.com/Praket7/chrome-controlla/actions/runs/37628108314) passed all three jobs; this is not a published GitHub install.
- [ ] Verify stale/current MCP client/server behavior beyond the extension handshake, signed release artifacts/SBOM, and consumer installation from GitHub. No credential/browser-profile deletion on uninstall without explicit request.
- [x] Review packaged files for desktop baggage, secrets, stale docs, test-only capabilities and unqualified marketing claims; archive listings contain only expected CLI, license/notice, core binary, and Chrome extension files.
- [x] Prepare and checksum a local macOS arm64 release candidate from clean commit `2f4fb06`; push the feature branch to the authorized Chrome Controlla repository. The candidate is unpublished; no GitHub release was created.
- [x] Pass hosted build/package CI on the candidate source commit; run `37564319202` passed Windows, macOS, and Linux.
- [ ] Verify published Windows/Linux consumer installs; CI-built artifacts are not a public consumer installation.
- [ ] Publish/deploy only within actual authorization. If final approval is required, present exact version, diff, test report, visibility, costs and artifact destinations so approval is the last step.
- [ ] After any authorized publication, verify registry/release availability and a clean consumer installation. A successful upload command is not release verification.
- [ ] Deliver repo URL/commit, install instructions, master guide, capability matrix, benchmark results, known blockers, and exact verification boundaries. Record all 24 CC requirements and 36 B cases as passed/failed/blocked/unimplemented with evidence.

**Completion gate:** the scoped product is implemented and honestly qualified, required release work is finished or explicitly blocked, and no unsupported superiority claim remains. Do not say “best MCP ever.” State precisely what it beats, on what tasks, under which conditions, and what it still cannot do.

## Required test entrypoints

Create these entrypoints as their phases mature; they are planned commands, not currently runnable commands in this research workspace.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
./scripts/check.sh --suite regression
./scripts/check.sh --suite interference
./scripts/check.sh --suite extraction
./scripts/check.sh --suite security
./scripts/check.sh --suite clients
./scripts/docs-check.sh
./scripts/package-check.sh
```

`check.sh` must return nonzero for failure, and a separate explicit unavailable/blocked report for missing live environments; it must never translate skipped tests into pass. Store evidence per suite, version and OS. Benchmarks get dedicated manifests and commands generated from the pinned baseline installation, not ad hoc screenshots of a successful demo.

## Final self-review before claiming done

- Can a fresh agent find the right mode and next tool without guessing?
- Are catalog, doctor, dispatch and policy decisions from the same current context?
- Can a client timeout, daemon restart or user edit create a duplicate or wrong-target effect?
- Are virtualized lists complete or explicitly partial, with evidence?
- Does strict background preserve user input and clipboard on each advertised OS?
- Are app support, client support, fixtures, live tests and benchmark wins clearly separated?
- Does the new package contain only Chrome-focused functionality and required attribution?
- Were all internal operations, model calls, verification and cache costs counted?
- Does close prior art appear in the research and does the AGWC claim survive ablation?
- Are remaining blockers candidly listed with the smallest exact user/external action needed?

If any answer is unsatisfactory, fix the underlying issue or qualify the claim before final delivery.
