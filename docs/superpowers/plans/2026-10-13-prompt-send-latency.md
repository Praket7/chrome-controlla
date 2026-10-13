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

- [x] P0-00: reproduce and fix cross-platform `check:clients` failures; add contract regressions and run the downstream client/app/benchmark/release/V3 checks.
- [x] P0-01: add monotonic prompt-handoff-to-final-result spans and counters to configured client adapters, benchmark contracts, and versioned reports; unavailable GUI/client measurements remain explicitly null.
- [x] P0-02: use the existing six-tool `browser` task entry as the typed one-call capsule through routing, observation, action, and verification.
- [x] P0-03: enforce qualified-route zero-model budgets and bounded novel-task planning/observation budgets in route receipts and execution paths.
- [x] P0-04: keep just-in-time identity/actionability checks at browser mutation dispatch, with event-driven invalidation inside batches.
- [x] P0-05: select routes from bounded, verified site/intent/tier/warmth/size latency cohorts; cohort warmth is scoped to the active browser session and qualified route.
- [x] P0-06: piggyback structured readback and outcome verification in the execution call where the browser exposes sufficient evidence.
- [x] P0-07: retain the warm broker, event-updated target/document inventory, and singleflight attach; distinguish live-session task warmth from external cold-browser startup.
- [x] P1-01: cache WebMCP metadata per document, invalidate on lifecycle changes, and revalidate the selected tool before execution.
- [x] P1-02: adapt observation richness from bounded measured task/route utility while retaining deterministic filtering.
- [x] P1-03: consume bounded visual probes locally before returning control when structured evidence resolves the target.
- [x] P1-04: expand DAG parallel reads only when measured expected savings exceed setup/alignment costs.
- [x] P1-05: keep speculation read-only/reversible and disable it under competing task load.
- [x] P1-06: vary skill qualification/expiry by risk and require typed input provenance; temporal ordering alone is not dependency evidence.
- [x] P1-07: preserve deterministic six-tool order/schema and compact evidence-handle receipts.
- [ ] Gate: run all local checks, verify exact-head GitHub Actions green across OSes, then obtain the requested fresh Luna Medium plan review. Record unavailable live qualification separately.

## Candidate implementation status — 2026-10-13

This source candidate closes the listed P0/P1 code paths: the existing typed browser task is the one-call capsule; qualified routes carry route-specific zero-model budgets and fail their receipt gate when reported internal calls exceed it; the cost router learns only verified exact cohorts across single-site research and active-page tasks; measured observation and DAG histories drive bounded lower-cost choices; WebMCP discovery and invocation share a per-document catalog invalidated by toolchange; and induced recipes must retain typed-task dataflow provenance. The browser runtime still performs freshness checks at mutation dispatch and returns a same-call verifier receipt.

The prompt trace boundary is implemented at configured client-process adapters and benchmark reports. It cannot observe a user's GUI SEND instant or external model turns without a supported host event driver; those values remain explicitly unavailable rather than inferred. Targeted visual probes run inside the same browser task call and are consumed locally when structured evidence resolves the target; when pixels still require model judgment, the bounded crop is returned in that call for the caller model to interpret. This does not promise zero caller model turns for visually novel work.

The remaining release gates are evidence/configuration, not implied complete by fixtures: exact-head hosted CI after this candidate is pushed; supported host-event traces for all six named clients; the 19-class live-site qualification; and repeated paired Prompt SEND → verified and final-answer comparisons against the registered baselines. Current host discovery reports OpenCode/Codex CLI available, Claude Code missing, Freebuff interactive-only, ChatGPT Desktop requiring an interactive driver, and generic MCP protocol-only. No matched external comparison or universal speed claim is authorized by the local test results.

Task-route warmth is scoped to the current browser session and separates a first-seen site (`warm_browser_first_seen_site`), a previously seen site (`warm_browser_seen_site`), and a route with a prior independently verified qualified execution (`warm_qualified_route`). A V3 task cannot truthfully record a cold browser because it requires a live session. Physical cold-browser startup must come from the external client trace and remain a separate cohort; these values must never be combined.

Induced mutation recipes now retain only typed task-input provenance (target/value bindings and declared typed outcomes). Caller step order is not treated as a causal data dependency. Cross-step dependencies are recorded only where a typed task contract defines them, such as save-after-fill in a form draft.
