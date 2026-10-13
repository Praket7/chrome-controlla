# Prompt SEND to Verified Outcome Implementation Plan

**Goal:** Reduce real user-perceived time from prompt submission to a compact, independently verified browser result while preserving the six-tool surface and mutation safety.

**Architecture:** Measure end-to-end spans at the client/benchmark boundary, then move predictable work into one browser goal call with route-specific budgets, browser-local freshness checks, same-call verification, and warm broker inventory. Use measured route latency and risk to choose among the existing execution tiers. Add no top-level MCP tools.

**Tech Stack:** Rust workspace, Node.js client and benchmark adapters, GitHub Actions.

**Spec:** User-provided “Controlla Stage 3 Audit — Prompt SEND → Verified Outcome” dated 2026-10-13.

## Global constraints

- Preserve exactly six eager MCP tools and stable compact schemas.
- Qualified paths use zero internal model calls and zero model-visible observations.
- Mutations are never blindly retried after unknown delivery.
- Page-provided tool metadata and content remain untrusted.
- Keep cold-browser, warm-browser/cold-site, and warm-qualified-route measurements separate.
- Do not make a comparative performance or “best Chrome use” claim without matched paired external evidence.

## Review focus

- Windows/macOS/Linux path and process behavior must agree.
- Prompt and tool telemetry must distinguish observed counts from unavailable external model telemetry.
- Route budgets must be enforced at the dispatch boundary, not merely reported.
- Browser-local epoch checks must still run immediately before every mutation.
- Persisted route/skill evidence must be bounded, risk-scoped, and invalidated on drift.

---

## Ordered tasks

- [ ] P0-00: fix cross-platform `check:clients` failures from run 38437418756; reproduce locally, add regression coverage, and verify all other CI gates execute.
- [ ] P0-01: add monotonic prompt-send-to-final-result spans and counters to client adapters, benchmark contracts, and versioned reports; include unavailable fields explicitly.
- [ ] P0-02: extend existing `browser.goal` with typed task constraints/postconditions and one-call execution through route, observation, action, and verifier.
- [ ] P0-03: enforce zero-model/zero-observation budgets for qualified tiers and bounded planning/observation budgets for novel tasks.
- [ ] P0-04: keep just-in-time identity/actionability validation inside browser-side mutation batches using event-updated state.
- [ ] P0-05: use bounded site/intent/tier/warmth/size measurements to select minimum expected wall-time subject to success and risk limits.
- [ ] P0-06: piggyback strong structured readback and verification in the execution call wherever available.
- [ ] P0-07: maintain a warm broker and event-updated target/document inventory; use singleflight on cold attach; report cold/warm strata separately.
- [ ] P1-01: cache WebMCP metadata per document, invalidate on lifecycle changes, and revalidate the selected tool before execution.
- [ ] P1-02: adapt observation richness from measured task and route utility while retaining deterministic filtering.
- [ ] P1-03: consume bounded visual probes internally before returning control to the model.
- [ ] P1-04: expand DAG parallel reads only when measured expected savings exceed setup/alignment costs.
- [ ] P1-05: make speculative reads bounded, reversible, and load-aware.
- [ ] P1-06: vary skill promotion/expiry thresholds by mutation risk and require data-flow grounded dependencies.
- [ ] P1-07: preserve deterministic six-tool order/schema and compact evidence-handle receipts.
- [ ] Gate: run all local checks, push, verify exact-head GitHub Actions green across OSes, then run the requested fresh Luna Medium plan review. Record unavailable live qualification separately.

## Candidate implementation status — 2026-10-13

This source candidate closes the listed P0/P1 code paths: the existing typed browser task is the one-call capsule; qualified routes carry route-specific zero-model budgets and fail their receipt gate when reported internal calls exceed it; the cost router learns only verified exact cohorts across single-site research and active-page tasks; measured observation and DAG histories drive bounded lower-cost choices; WebMCP discovery and invocation share a per-document catalog invalidated by toolchange; and induced recipes must retain typed-task dataflow provenance. The browser runtime still performs freshness checks at mutation dispatch and returns a same-call verifier receipt.

The prompt trace boundary is implemented at configured client-process adapters and benchmark reports. It cannot observe a user's GUI SEND instant or external model turns without a supported host event driver; those values remain explicitly unavailable rather than inferred. Targeted visual probes run inside the same browser task call and are consumed locally when structured evidence resolves the target; when pixels still require model judgment, the bounded crop is returned in that call for the caller model to interpret. This does not promise zero caller model turns for visually novel work.

The remaining release gates are evidence/configuration, not implied complete by fixtures: exact-head hosted CI after this candidate is pushed; supported host-event traces for all six named clients; the 19-class live-site qualification; and repeated paired Prompt SEND → verified and final-answer comparisons against the registered baselines. Current host discovery reports OpenCode/Codex CLI available, Claude Code missing, Freebuff interactive-only, ChatGPT Desktop requiring an interactive driver, and generic MCP protocol-only. No matched external comparison or universal speed claim is authorized by the local test results.

Task-route warmth is scoped to the current browser session and separates a first-seen site (`warm_browser_first_seen_site`), a previously seen site (`warm_browser_seen_site`), and a route with a prior independently verified qualified execution (`warm_qualified_route`). A V3 task cannot truthfully record a cold browser because it requires a live session. Physical cold-browser startup must come from the external client trace and remain a separate cohort; these values must never be combined.

Induced mutation recipes now retain only typed task-input provenance (target/value bindings and declared typed outcomes). Caller step order is not treated as a causal data dependency. Cross-step dependencies are recorded only where a typed task contract defines them, such as save-after-fill in a form draft.
