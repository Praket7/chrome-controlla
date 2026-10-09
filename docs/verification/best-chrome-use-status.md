# Best Chrome Use Verification Status

This file records implementation and evidence status without converting missing evidence into a product claim.

## V3 implementation status — 2026-10-09

V3 implementation is partial. The current branch has a six-tool MCP surface, mode/typing/interference/lease/reconnect primitives, guarded selected-tab extension batches, actual CDP FastKeys batches, IME composition and commit dispatch, deterministic task expansions, and a fail-closed v3 claim gate. Local tests qualify those code paths and contracts only.

The extension batch path now requires the negotiated capability, reauthorizes each selected-tab action, bounds each request to 64 actions and the shared message limit, and stops on the first failed guard. FastKeys submits real CDP key events without an intentional 60 ms delay. Its current path batches at most 16 characters per host round trip; partial receipts that may include prior key dispatch return `unknown`, and workflows stop on `unknown` or `not_dispatched`. Shared target leases use one server-wide extension namespace plus the observed tab ID, preventing session aliases from bypassing same-tab exclusion. Each action is bounded to 60 seconds and holds a 120-second target lease; an `unknown` result retains the lease through expiry. A workflow does not hold an exclusive lease across all steps, so safe interleaving depends on each subsequent revision-bound ref rejecting drift. This is not yet a general near-page DOM workflow executor or a 1,000-character real-browser performance qualification. IME dispatch uses composition and commit operations followed by a runtime readback; OS-level IME behavior remains unqualified.

The v3 client matrix verifies generated configuration and adapter contracts for Freebuff, OpenCode, Claude Code, Codex, ChatGPT Desktop, and generic MCP. It explicitly records real-client smoke status as unverified. The WebMCP descriptor route and bounded schema validator are unit-tested, but browser discovery and invocation are not wired into the selected-tab MCP route. Headless stress covers eight isolated real Chrome profiles locally and in the new Ubuntu lane; it does not establish full headed-background parity or crash/reconnect recovery.

The v3 plan remains open for real user-interference and multi-client browser races, complete mode/typing event semantics, app-level authenticated persistence and file acceptance, live client qualification, integrated WebMCP execution, and controlled competitor runs. The broad superiority gate remains blocked without complete real competitor evidence.

## Implemented engine foundations

- execution telemetry and compact runtime metrics
- semantic snapshots, short revision-bound references, and deltas
- verified fast-input policy with strict fallback classification
- bounded deterministic browser-workflow contract
- runtime-owned verification primitives
- persistent qualified-skill store and fail-closed replay policy
- targeted visual-probe references
- structured capability routing
- adaptive per-target scheduling
- compact MCP v2 tool surface
- packaged ChatGPT Desktop local plugin and v2 server
- difficult-browser-class qualification matrix
- executable Phase 11 adapter runner with per-run reset, bounded execution, independent post-run verification, and complete task/mode/concurrency identities
- full Phase 11 structured-competitor suite orchestration for Playwright MCP, Chrome DevTools MCP, agent-browser, Stagehand, and Browser Use, with computer-use kept on the separate vision track
- paired Phase 11 analysis that counts independently verified success and retains failed runs in latency accounting
- Phase 15 drift and fault-injection contract
- fail-closed release gate that requires a complete independently verified 12-task x 3-mode x 4-concurrency matrix against every declared structured competitor before a broad superiority claim is eligible

## Safety invariants

A release candidate is blocked from a broad superiority claim if any measured severe wrong-target event, authority violation, or silent stale-reference mutation occurs. Unknown mutation delivery is never silently retried. Caller-supplied evidence is not treated as independent verification. Structural drift quarantines qualified skills.

## Benchmark integrity

The frozen Phase 10 track is preserved. Phase 11 writes raw paired results separately and does not alter the preregistered Phase 10 acceptance thresholds. Each Phase 11 adapter is invoked through a bounded JSON command contract, the fixture is reset before each run, and a separate verifier determines final success after the acting adapter returns. Broad-claim evidence must include the full matrix for every structured baseline plus a reproducibility manifest covering Chrome version, viewport, model, prompt hash, token budget, fixture revision, and machine profile.

The claim path itself is exercised in CI with generated test evidence. Those generated rows are deleted after the test and are not benchmark evidence. The test confirms that complete evidence can pass the code path and that incomplete matrices or missing independent-verifier evidence are rejected.

## Evidence currently allowed

The repository contains narrow local fixture evidence for native shared-tab pairing, guarded fill, strict typing, fixture slide interactions, mock post behavior, navigation invalidation, bounded readback, release, cleanup, benchmark-runner contracts, and claim-gate behavior. This evidence qualifies those fixture and harness paths only.

## Evidence still external

Remote Google Slides, Canva, and CapCut Web mutation plus reload persistence remain pending until those applications are exercised with independent verification. Real competitive Phase 11 measurements against the declared adapter set are also external benchmark work. The executable harness exists, but no synthetic or CI-generated test rows may be presented as competitor evidence. Until a measured `phase11-results.json` from controlled real runs passes the claim gate, the repository must not describe Controlla as universally faster or better than every browser agent.

## Claim policy

The non-claim release check validates code, contracts, safety gates, benchmark configuration, the executable benchmark harness, and the fact that the claim gate is fail-closed. A public broad superiority claim requires running the release gate with measured Phase 11 paired results against every declared structured competitor. If the complete gate does not pass, only narrower measured claims may be made. Computer-use remains a separately reported vision/computer-control track rather than being mixed into the structured-browser headline gate.
