# Stage 3 — Fast execution, long workflows and credible comparison

> **For agentic workers:** Use superpowers:executing-plans task by task. Completion requires measured public-client behavior and independent review.

**Goal:** Minimize actual prompt SEND to verified outcome and final answer, while preserving success rate and expanding useful task coverage.

**Architecture:** Maintain persistent target/event state. Compile one typed task into a bounded local program with just-in-time guards and same-call verification. Choose qualified routes by measured latency, success and risk, with compact evidence and explicit fallbacks.

**Tech stack:** Existing Rust workflow VM/DAG, capability and observation routers, skill/evidence stores, native broker, extension, Node client adapters and benchmark reports.

**Spec:** Background-window design plus companion Stage 1 and Stage 2 plans in this directory.

## Global constraints

- Six stable eager tools; no seventh “execute” tool and no routine model-visible discovery.
- Optimize total user-visible wall time, not only tab-open microseconds or external tool count.
- An unknown internal model-call count cannot qualify a zero-model path.
- Semantic execution first; targeted visual when useful; no mandatory full screenshot or model summarizer per action.
- Independent mutations are not automatically safe to parallelize. No speculative irreversible action.
- No reduced success rate hidden by averaging, exclusions, retries or counting only successful runs.
- A working scaffold, simulated client, stale commit or unrelated browser profile is not live qualification.
- No universal fastest/best claim without matched contemporary competitor evidence.

## Evidence motivating this stage

The live one-call headless task still consumed 35 browser commands for the successful canonical example-page route. A warm background click used 18 commands and 17 host round trips; touch used 21/20. Headless upload used 126 commands and still missed acceptance. The 128,633-byte Canva semantic result was labeled compact. Deterministic routes still returned unknown internal-model counts and an inconclusive model-budget gate.

These are samples, not latency distributions. Cold startup and concurrent audit work confound timings. The audit did not run matched competitor workloads or measure the actual prompt-SEND boundary.

## Review focus

1. Process-launch timestamp mislabeled as user submission.
2. A one-call wrapper hiding dozens of transport messages or internal model turns.
3. Cached route/skill surviving account, structure or permission drift.
4. Fast success-only averages concealing failure/retry tails.
5. Overlapping spans, partial counters and warm/cold mixing producing misleading metrics.

## S3-01 — Full causal trace from the client boundary

**Files:** scripts/client-host-adapters.mjs, scripts/run-live-client-smokes.mjs, bench/runners/phase11-runner.mjs, runtime metrics, broker and extension dispatch.

**Interface:** TraceEvent {trace_id, span_id, parent_span_id, operation_id, clock_domain, monotonic_time, boundary, provenance, build_id}. Clock synchronization uses recorded offsets/uncertainty; cross-process monotonic values are not blindly subtracted.

- [ ] Observe actual client prompt submission when supported. Otherwise label process_start_to_outcome and leave prompt_sent unavailable.
- [ ] Record prompt_sent, first_tool_decision, MCP_received, first_browser_op, last_browser_op, verified_outcome, result_delivered and final_answer.
- [ ] Count external calls, external model turns, model-visible observations, internal model calls, CDP commands, native host messages and schema/input/output/image bytes separately.
- [ ] Record queue, startup, attach, readiness, action, verification, artifact and serialization spans. Overlapping spans are not additive elapsed time.
- [ ] Classify cold browser, warm browser/cold site, warm qualified route and reconnect separately.
- [ ] Preserve partial-counter flags and missing boundaries. Gate on missingness rather than substituting zero.
- [ ] Keep prompts/credentials out of process arguments and diagnostics; use supported stdin/IPC channels.
- [ ] Verify trace with a deterministic delayed fixture and client adapter hook, including failure before first tool.

**Acceptance:** One timeline explains the entire delay with documented unobservable intervals; no process-start measurement is called prompt-SEND latency.

## S3-02 — Hard zero-model execution budgets

**Files:** capability_router.rs, workflow_vm.rs, skill_runtime.rs, task receipts.

**Interface:** RouteBudget {internal_models, visible_observations, browser_commands, wall_deadline, fallback_count}; counters increment at actual invocation boundaries.

- [ ] Qualified skill, qualified site tool, deterministic semantic program and verified speculative read each allow zero internal model calls and zero model-visible observations.
- [ ] Novel tasks allow at most one initial planning observation/pass where feasible; visual fallback gets a separately bounded budget.
- [ ] Instrument a central model invocation boundary. If a route has no model integration, emit an explicit measured zero with provenance.
- [ ] Do not derive actual counts from missing child receipt fields. Aggregate known invocation counters and propagate completeness.
- [ ] Stop before violating a budget and return the required escalation with existing evidence. Successful over-budget work fails performance qualification.
- [ ] Test every tier, nested workflow, fallback, cancellation and external-client telemetry absence.

**Acceptance:** The current null/inconclusive deterministic receipts become measured zero or a concrete failing gate. Predictions never substitute for counters.

## S3-03 — One-call task program with local verification

**Files:** v3_full.rs browser goal/task, workflow_vm.rs, browser_workflow.rs, workflow_dag.rs, capability_registry.rs.

**Interface:** TaskCapsule {target_policy, typed_steps, variables, constraints, postconditions, budget, cleanup}; Result {status, compact_receipt, evidence_handles, unresolved_state}.

- [ ] Resolve/reuse the target, choose capability, ground required elements, execute and verify inside one call.
- [ ] Support bounded typed sequences, branching on observed evidence, loops with limits and cross-tab references. Use the existing VM where possible.
- [ ] If exposing script-like execution, provide a constrained capability API, not arbitrary filesystem/network/credential access or page-supplied code execution.
- [ ] Preserve per-mutation just-in-time identity/actionability checks within the browser-side batch.
- [ ] Watch application outcomes before action; reconcile once locally on uncertainty. No automatic replay of possibly delivered writes.
- [ ] Return one compact verified receipt and opaque evidence handles. Do not send each internal snapshot to the model.
- [ ] Defer skill induction/learning persistence until after the completed result can be delivered.
- [ ] Test navigation plus newest-story, multi-field form, download-to-upload and seven-slide capture manifests.

**Acceptance:** Qualified warm tasks use one external Controlla call and no schema/repository calls. Long authoring/research tasks may need genuine model decisions; do not pretend those can be zero-turn deterministic jobs.

## S3-04 — Remove redundant browser transport work

**Files:** native_host.rs, extension command broker, mcp snapshot/act/task paths and scheduler.

- [ ] Trace the recorded 18-command click, 35-command navigation/extract and 126-command upload paths; attribute each command to a purpose.
- [ ] Reuse current document inventory and scoped evidence when its invalidation epoch is unchanged.
- [ ] Batch compatible preflight/readback operations; keep final mutation validation adjacent to dispatch.
- [ ] Subscribe to navigation, DOM invalidation, focus, resize, file and target events. Use bounded polling only as a fallback with explicit reason.
- [ ] Singleflight cold attach/setup; keep the broker alive and event inventory warm. Bound idle resource use and cleanup.
- [ ] Avoid repeating whole-page snapshots for a small output predicate.
- [ ] Prototype event-driven pairing notification with polling fallback only after measuring cold-pairing contribution.
- [ ] Test event loss, reconnect, queue contention and stale cache; no speed gain may come from omitting identity checks.

**Initial proposed budgets for the local warm fixture:** click/readback at most 6 native host round trips; navigation plus extraction at most 12; upload acceptance at most 20 excluding chunk payload transfers. These are engineering targets to validate, not claims about all sites. If unattainable, provide a command-by-command justification and revised measured target.

## S3-05 — Compact stable model context

**Files:** six tool schemas/descriptions, plugin browser-control SKILL.md, observation_router.rs and evidence serializers.

- [ ] Deduplicate common tool instruction text and preserve deterministic order/schema stability.
- [ ] Keep dynamic page evidence out of the static instruction prefix.
- [ ] Default receipts target at most 2 KiB and targeted semantic observations at most 12 KiB for small tasks; explicit larger requests report why.
- [ ] Retain references, relevant labels, identity, coverage and actionability; remove duplicate deltas and null fields.
- [ ] Preserve text and visual evidence needed for authoring/review behind retrievable handles, not lossy summaries.
- [ ] Learn observation selection from latency and success by task class; do not insert an LLM reducer on the hot path.
- [ ] Test task-selection accuracy and false uniqueness as bytes decrease.

**Acceptance:** Measurably smaller client context with no meaningful success regression. A smaller snapshot that misses the relevant control fails the gate.

## S3-06 — Empirical routing and bounded speculation

**Files:** capability_router.rs, observation_router.rs, capability_registry.rs and workflow_dag.rs.

**Interface:** CostCohort(site_key, intent, provider, warmth, task_size); Stats {sample_count, success, false_success, EMA, p50, p95, uncertainty}.

- [ ] Select expected lowest verified wall time subject to minimum success/risk constraints. Require sample counts and uncertainty; avoid overfitting one site.
- [ ] Account for model turns, broad observations, visual processing, browser cold start, schema setup, transport and verification.
- [ ] Cache WebMCP tool metadata by document and lifecycle version; revalidate the selected tool immediately before execution. Descriptions/results remain untrusted.
- [ ] Speculate only cheap read-only operations; cancel losers and disable under load. Include duplicated work in cost.
- [ ] Grow parallel DAG read branches only when predicted savings exceed setup/alignment cost. Keep tiny tasks serial.
- [ ] Persist bounded stats, expire drifted cohorts and fall back deterministically when data is sparse.
- [ ] Test slow site, overload, drift, outliers, cold-cache misses and adversarial page tool descriptions.

**Acceptance:** Routing decisions expose measured rationale and fallback count without a model-visible discovery loop.

## S3-07 — Reusable skills and long-horizon task state

**Files:** skill_store.rs, skill_induction.rs, skill_runtime.rs, workflow_dag.rs, page-action memory and artifacts.

- [ ] Induce workflows from data-flow-supported dependencies, not temporal adjacency.
- [ ] Separate training and held-out qualification; vary inputs, navigation paths, accounts and UI structures.
- [ ] Apply risk-specific qualification and expiry: reads can qualify sooner; drafts require stronger state checks; consequential submissions need strongest evidence and explicit postconditions.
- [ ] Quarantine structure/account drift and unseen branches. Never replay an unknown mutation.
- [ ] Checkpoint long tasks using target/artifact/evidence identities and completed postconditions. Resume from verified state, not guessed prior progress.
- [ ] Keep document generation and factual/source checking distinct from browser transport; the user's four PDFs plus deck workflow needs both.
- [ ] Learn asynchronously after result delivery and bound storage/retention.

**Acceptance:** Repeated qualified workflows avoid per-step model decisions; interrupted long workflows resume without duplicate posts or losing human edits.

## S3-08 — Matched benchmark and release evidence

**Files:** existing phase11/phase12 runners and analysis, client-host adapters, qualification/release scripts.

- [ ] Pin candidate source SHA, dirty-tree patch hash if applicable, binary hash, extension hash, Chrome build, OS/machine, model/settings, client adapter and account fixture.
- [ ] Compare Controlla baseline and candidate, Playwright MCP, Chrome DevTools MCP/CLI, agent-browser, Stagehand and Browser Use deterministic mode where supported.
- [ ] Use the same prompt intent, model, machine, network class, account state, independent verifier and allowed capabilities. Record unsupported cells; do not replace signed-in workflows with public example pages.
- [ ] Cover direct destination/newest, rich editor, form, upload/download, virtual extraction, OOPIF, multi-tab research, repeated workflow, drift recovery and persisted mutation.
- [ ] Cross foreground/background/headless and the Stage 2 window/lifecycle matrix. Track focus violations.
- [ ] Run at least 30 measured trials per feasible task/mode/cohort after separately recorded warmup, randomized paired order; predeclare exclusions and timeouts. Expand sample size when intervals remain inconclusive.
- [ ] Report success, false success, timeouts, unknown delivery, prompt-to-verified and prompt-to-final p50/p95, calls, turns, bytes, CDP/host commands, retries/fallbacks and cleanup.
- [ ] Include all attempts and failed-run costs. Publish both success-conditioned latency and all-attempt time-to-success/timeout; never rank a fast failing agent as best.
- [ ] Bootstrap paired intervals at task/run level; report reliability across repeated runs (for example all-success across five repeats), not only a lucky successful attempt.
- [ ] Independently verify server/app persistence and artifacts. The implementation's own receipt cannot be its sole oracle.
- [ ] Qualify all six configured client adapters through real submission/result hooks; simulated adapters remain separately labeled.
- [ ] Verify exact candidate CI on Ubuntu/macOS/Windows including downstream app/bench/release/V3 gates. Local green is not hosted green.

**Release rule:** Candidate beats the baseline on median and p95 prompt-to-verified with a predeclared success noninferiority margin and no false-success regression. Set the margin before collection (proposed 1 percentage point; insufficient statistical power is inconclusive). External superiority claims require matched external evidence and must name the tested task/mode/client population. No universal “best Chrome use” claim follows.

## S3-09 — Packaging, instructions and operational finish

- [ ] Check extracted release packages include every extension page/asset and match tested binaries.
- [ ] Add one compact compatibility handshake and useful reload-needed diagnostics; ordinary tasks avoid setup work when already compatible.
- [ ] Rebuild and reload the exact configured MCP/extension, then run the supported matrix.
- [ ] Update docs/progress.md, docs/blockers.md and docs/verification-matrix.md with implementation, fixture, live, CI and benchmark states separately.
- [ ] Obtain a fresh independent reviewer against every checkbox and public-route evidence, then address findings.
- [ ] Push only when requested and release gates pass. Never turn a reviewer's approval into a substitute for missing measurements.

## Three-stage dependency and acceptance map

| User requirement | Owner | Evidence needed |
|---|---|---|
| Existing tab, correct account, choose window | S1-06; S2-01/08 | Stable identity, no duplicate, no focus violation |
| Covered-window screenshots | S1-02; S2-02 | Fresh nonempty correct compositor pixels |
| Every slide reviewed | S2-07 | Complete ordered image/slide manifest |
| Click/scroll/drag/touch/type | S1-01; S2-03/04 | Correct event and independent postcondition |
| Upload/pull files anywhere supported | S1-03; S2-05 | Hash/bytes, app acceptance and persistence level |
| Google sign-in/passwords | S2-06 | Correct origin/account and redacted evidence |
| Human interaction and multiple agents | S1-07; S2-08 | Target-local concurrency and conflict recovery |
| Fewer calls and faster completion | S3-01 through S3-08 | Full causal trace and matched distributions |
| Clear instructions, no setup loops | S1-08; S3-05/09 | Stable six-tool surface and public-client tests |

## Research reviewed and how it affects the design

Research used four Exa query angles (34 requested result slots, not 34 independently verified papers), two Parallel Search searches, alphaXiv discovery/content and SciSpace literature search. Prefer the primary sources linked below over aggregator summaries. No source's benchmark is a Controlla result.

- [Agent JIT Compilation](https://arxiv.org/abs/2605.21470): motivates local compiled execution, reuse and cost-aware scheduling. Do not copy headline speedup into Controlla claims.
- [Observation reduction study](https://arxiv.org/abs/2605.29397): motivates measuring success alongside context reduction; faster reduced observations can lose task success.
- [Web-agent reliability study](https://arxiv.org/abs/2604.17849): motivates repeated-run consistency instead of one-shot success.
- [WALT](https://arxiv.org/abs/2510.01524): motivates qualified high-level site tools with a fallback path.
- [ReUseIt](https://arxiv.org/abs/2510.14308): motivates reusable workflows with explicit qualification.
- [WAREX](https://arxiv.org/abs/2510.03285): motivates perturbation testing and recovery.
- [Stagehand persistent agent interface](https://docs.stagehand.dev/v4/integrations/agent-frameworks/overview): useful comparison for script/action execution and persistent sessions.
- [Playwright best practices](https://playwright.dev/docs/best-practices): supports locator/actionability and web-state assertions as deterministic foundations.
- [Chrome DevTools MCP setup](https://developer.chrome.com/docs/devtools/agents/get-started/configuration): useful existing-browser provider comparison.
- [agent-browser repository](https://github.com/vercel-labs/agent-browser): benchmark the pinned implementation; an individual bug report does not establish general comparative inferiority.

## Completion definition

All three plans require functioning code and wiring, regression coverage, exact-runtime live qualification and clear supported-mode boundaries. “Fastest” is an empirical claim to earn after these gates. None of the measured failures is fixed by relabeling unknown as success, hiding unsupported modes, deleting identity guards, or adding another layer of instructions.

