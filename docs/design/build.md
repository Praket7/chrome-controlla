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

- [ ] Inspect workspace instructions, Git identity/auth and existing repositories before cloning. Do not overwrite a user checkout or discard uncommitted changes.
- [ ] Clone upstream into a separate source directory. Record fetched HEAD, remote URL and dirty state. Compare browser/core/bridge/package changes to the researched SHA; determine whether reported regressions correspond to an older binary or another route.
- [ ] Inspect licenses and dependencies. Create a new local repository named `chrome-controlla`, preserving notices and provenance, with no credentials or browser profiles.
- [ ] If executing this prompt includes authorized GitHub repository creation, use the authenticated user’s intended owner and create **a separate repository**, never rename/delete Comptrol. Default private when visibility is unspecified; record that choice. If the name exists, inspect ownership/content and stop before overwriting. Do not guess an alternate name or push into an unrelated repo.
- [ ] Select and pin Rust, Node/reference runner, MCP SDK/protocol and Chrome for Testing versions supported by the environment. Document rationale and supported OS targets.
- [ ] Extract minimal browser engine and tests, not the monolithic app registry. Create provenance mapping and a dependency check that fails if native desktop platform crates/adapters re-enter the distribution.
- [ ] Add a CI baseline: formatting, static checks, focused Rust tests, fixtures, package/docs checks. Confirm a clean checkout can build without upstream globally installed.
- [ ] Commit `chore: establish Chrome-only source extraction and provenance`.

**Gate:** actual source reconciliation and clean build, not merely a GitHub repo URL. Owner/visibility/publication decisions are recorded. No implementation superiority claim.

## Phase 1 — one registry, reliable CLI and installation

**Files:** `registry.rs`, `policy.rs`, CLI `args.rs`/`doctor.rs`, schemas, package launcher, `tests/e2e/registry.rs`, `tests/e2e/cli_help.rs`, `tests/e2e/installation.*`.

**Interfaces:** `evaluate_capability(ctx: &CapabilityContext, action: &str) -> CapabilityDecision`; `doctor(config: &Config) -> DiagnosticReport`; dispatch consumes the same decision function.

- [ ] Write failing tests for CC-01–04 and B01–04: direct-only, bridge-only, unconfigured, denied, revoked, minimal PATH, and help with open stdin.
- [ ] Implement early argument parsing. Help/version/list-schema never initialize browser runtime or mutate state. Invalid flags fail clearly.
- [ ] Implement one evaluator with structured reasons; catalog is a snapshot, dispatch reevaluates. Add principal and policy revision to diagnostic correlation without secrets.
- [ ] Report heartbeat and authenticated round-trip separately. Preserve storage/path/permission errors instead of mapping all to false. Test different process environments and daemon state paths without claiming one is the tester’s proven cause.
- [ ] Resolve the launcher relative to the installed package or explicit path. Make cold install failures distinguish download, architecture, permission and missing executable.
- [ ] Run focused tests and clean-install smoke; inspect packaged contents.
- [ ] Commit `feat: unify capability decisions and standalone CLI packaging`.

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
- [ ] Commit `feat: isolate Chrome sessions and track tab ownership`.

**Gate:** correct target identity and cleanup, with honest platform boundaries. Headless Chrome availability is not headless Canva/CapCut qualification.

The local/code review gate passed on 2026-10-06. Live extension attachment, independent in-tab cleanup observation, headed Chrome behavior, and native focus/cursor/clipboard observation remain unqualified; the target inventory check is a point-in-time snapshot.

## Phase 3 — jobs, durable idempotency and uncertain outcomes

**Files:** `journal.rs`, `jobs.rs`, `errors.rs`, jobs tool; `tests/e2e/{idempotency,jobs,crash_recovery}.rs`.

**Interfaces:** `admit(principal, session, request, key) -> Admission`; `record_dispatch(principal, session, op, correlation) -> DispatchClaim { acquired, operation }`; `reconcile(principal, session, op_id) -> Operation`; `wait(principal, session, op_id, after_revision, max_wait_ms) -> Operation`.

- [x] Implement local journal contracts for B05–07/B18–19; controlled fixture counting covers duplicate prevention after an accepted effect. No live endpoint or dispatch route exists yet.
- [x] Bind canonical JSON request identity to principal/session/key. Same key/body replays the original operation; changed body conflicts.
- [x] Use SQLite immediate transactions and unique constraints. Separate-connection simultaneous admission and dispatch-claim tests permit one sender; operations are scoped by principal/session on reads and mutations. Process-kill injection at every journal boundary remains open.
- [x] Separate bounded caller wait from durable operation state, persist a bounded 60-second default job deadline, and expose monotonic revisions. Broader execution limits remain unapplied because no worker route consumes them.
- [ ] Test real resolve/reject/never-resolve promises and navigation during evaluation over both CDP and bridge routes. No jobs execution route exists yet; local async lifecycle fixtures cover only persisted states.
- [x] Cancellation blocks queued dispatch; expiry before dispatch becomes `failed` with `deadline_error=deadline_exceeded` and `delivery=not_sent`. Expiry/recovery after dispatch becomes `unknown` while preserving delivery as `sent` only when transport acknowledgement was recorded, otherwise `unknown`.
- [x] Commit reviewed local journal gate as `feat: persist browser jobs and reconcile uncertain effects`.

**Gate:** local journal fixtures refuse duplicate dispatch on replay; remote exactly-once is not claimed. The exact 12-second Promise/CDP/bridge case remains untested because execution routes do not exist.

## Phase 4 — reliable input, guards and interference

**Files:** `input.rs`, runtime `identity.rs`/`policy.rs`, `tests/fixtures/input/`, `tests/e2e/interference.rs`.

**Interfaces:** `validate_step(target, dependencies) -> GuardDecision`; `perform_input(target, InputAction) -> DispatchEvidence`; `on_external_change(event) -> InvalidationSet`.

- [ ] Write B09–13/B22–25/B36 fixtures: account change, field edit, unrelated churn, overlay, frame change, Unicode, masked controls, drag, focus/clipboard and foreground requirement.
- [ ] Implement semantic locators and unambiguous matching. Add typed fill/insert/sequential keys/click/drag operations with postconditions.
- [ ] Implement navigation/account/document guards, semantic dependency invalidation, geometry/hit checks, and conservative fallback when relevance cannot be proved.
- [ ] Keep event handlers/remote effects in mind: DOM observations cannot guarantee atomic input. Inject a change between final validation and dispatch; measure the residual race and document stronger server revision paths where available.
- [ ] Implement internal clipboard and artifact insertion. No system clipboard mutation in strict background mode. No disabling human input to preserve a lease.
- [ ] Run real-OS disruption tests where available and mark the rest unverified.
- [ ] Commit `feat: guard input against stale targets and user interference`.

**Gate:** relevant detectable interference yields before subsequent mutation; input fixtures verify actual values/positions. No “interference-proof” claim.

## Phase 5 — compact observations and complete extraction

**Files:** `observe.rs`, `extract.rs`, observe tool, virtual-list fixtures, `tests/e2e/extraction.rs`.

**Interfaces:** `observe(target, selector, fields, budget) -> Observation`; `extract(target, ExtractionSpec) -> ExtractionResult`.

- [ ] Create fixtures for 42 virtualized records, 21-item separate bucket, blocked expansion, recycled nodes, duplicate labels, stale count, infinite feed and wrong-account 404.
- [ ] Implement bounded DOM/AX reads, semantic deltas and targeted screenshot crops with freshness, omissions and truncation fields.
- [ ] Implement schema-guided extraction, stable-ID deduplication, pagination/section traversal, and explicit terminal evidence. Keep parsing near the page; output large results as artifacts.
- [ ] Test `complete`, `partial`, and `unknown` classification against ground truth. Matching a count and repeated no-change scrolls alone must not certify completion.
- [ ] Measure payload and latency against full DOM/snapshot baselines without dropping necessary facts.
- [ ] Commit `feat: extract structured data with coverage evidence`.

**Gate:** all records or explicit missing coverage; never the tester’s visible-row-only false completeness.

## Phase 6 — constrained scripts and guarded workflow compiler

**Files:** `controlla-script`, `sdk/`, `compiler.rs`, `scheduler.rs`, workflow schema, execute tool, script/security tests.

**Interfaces:** `compile(request, capabilities) -> WorkflowGraph`; `run(graph, budget) -> OperationReceipt`; broker API `call(session_handle, operation, args) -> TypedResult`.

- [ ] Spike candidate script runtimes for wall/CPU/memory interruption, async host calls and isolation. Record the result and select one; do not stall the deterministic graph path on scripting research.
- [ ] Create failing tests for B27/B31/B32, cross-session handles, module import, network/file/process escape and output limits. Protect broker authority independently from JS parsing.
- [ ] Implement typed IR nodes and conservative effect/read/write metadata. SDK scripts may use bounded dynamic control flow; unsupported static analysis gets conservative scheduling rather than a fake proof.
- [ ] Implement batching boundaries, local condition waits, checkpointing, compact return values and script-generated artifact output. Count all underlying calls.
- [ ] Add dependency-scoped invalidation and effect-aware splitting. Start with rules, not RL.
- [ ] Compare fixed batch, code mode, and guarded compiler on fixtures; profile local overhead before optimizing Rust internals.
- [ ] Commit `feat: execute bounded browser programs with guarded checkpoints`.

**Gate:** useful multi-action scripts without ambient host authority; cancellation and partial receipts survive script failure.

## Phase 7 — outcome verification and workflow cache

**Files:** `verifier.rs`, `cache.rs`, `artifacts.rs`, B28–30 tests, rubric schemas.

**Interfaces:** `verify(target, predicate, evidence_scope) -> VerificationResult`; `qualify(workflow, suite) -> Qualification`; `lookup(signature) -> QualifiedWorkflow | Miss`; `quarantine(id, reason)`.

- [ ] Test persuasive fake success, old screenshot, wrong revision, stale save, truncated download and changed control semantics with independent ground truth.
- [ ] Implement deterministic field/object/state verifiers and artifact validators. Use visual review for visual criteria; record inconclusive when necessary.
- [ ] Bind receipts to target, app/account, revision, observer/time and predicate. Keep process correctness and outcome correctness separate.
- [ ] Implement versioned workflow cache with training/validation provenance, environment and authority preconditions, expiration/requalification and quarantine.
- [ ] Validate cold/warm behavior and include preparation/recovery costs. A historical success never authorizes a present mutation.
- [ ] Commit `feat: verify outcomes and retire stale workflows`.

**Gate:** no release-suite false completion; visual/aesthetic uncertainty is preserved, not coerced into pass.

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

- [ ] Implement supported MCP protocol negotiation and transport behavior against pinned SDK conformance tests. Do not mix “latest” protocol semantics with legacy initialization assumptions.
- [ ] Add authenticated remote endpoint, principal/audience-bound authorization and optional paired outbound local bridge. No public unauthenticated browser control; validate tenant and target binding.
- [ ] Generate Freebuff, OpenCode v1/v2 as supported, Claude Code and ChatGPT setup instructions for actual versions. Do not assume every platform has the same config nesting or local process support.
- [ ] In each available real client, run session setup, observation, guarded edit, long job, reconnect/reconcile, structured result/artifact and cleanup. Record client version and server SHA. A raw JSON-RPC smoke alone does not qualify a client.
- [ ] Turn the companion master-guide draft into exact runnable documentation. Generate shared facts from registry/schema. Implement `guide` and the canonical resource; include short bootstrap instructions in session/tool responses.
- [ ] Run B35 and the 20-task fresh-agent usability suite. Fix tool ambiguity, missing recovery instructions and invented flags rather than adding a larger wall of prose.
- [ ] Commit `feat: qualify MCP clients and ship executable master guide`.

**Gate:** each advertised client has live evidence or is explicitly marked unavailable/unqualified; guide and schemas agree.

## Phase 10 — controlled competition and optimization

**Files:** `bench/tasks`, `bench/baselines`, `bench/analysis`, immutable run manifests, `bench/reports/`.

- [ ] Pin Playwright MCP, Chrome DevTools MCP, Stagehand, Browser Use, Vercel agent-browser and relevant other baseline revisions. Use their supported configurations; do not handicap batching/caching or assign mismatched models.
- [ ] Where actual Computer plugin/Claude Chrome/Cowork access is available, define a documented end-to-end track. If they cannot be run, mark missing instead of inventing results.
- [ ] Run the 30-template pilot, estimate variance, preregister primary metrics/thresholds and held-out split. Include generic tasks, extraction, interference, multi-tab and design tasks; disclose design-review sample sizes separately.
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

- [ ] Run necessary full integration/security/package/docs checks on the release commit. Use actual platform runners for cross-platform claims.
- [ ] Test fresh installation, version upgrade, rollback, stale daemon/extension mismatch, uninstall, and user-data preservation. No credential/browser-profile deletion on uninstall without explicit request.
- [ ] Review packaged files for desktop baggage, secrets, stale docs, test-only capabilities and unqualified marketing claims.
- [ ] Prepare a release candidate with reproducible artifacts. Create/push to the authorized new repository; never push Chrome Controlla changes into Comptrol by accident.
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
