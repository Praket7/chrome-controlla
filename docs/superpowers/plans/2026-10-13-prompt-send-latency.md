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

