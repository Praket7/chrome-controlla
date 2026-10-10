# Best Chrome Use Verification Status

This file records implementation and evidence status without converting missing evidence into a product claim.

## V3 implementation status — 2026-10-09

V3 implementation is partial. The packaged MCP exposes exactly six tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. Task primitives and opaque private-headless artifact registration/read are bounded `browser` action variants, so the six-tool list stays small. A packaged contract test enforces the exact names and 48 KiB serialized-schema ceiling. The branch has mode/typing/interference/lease/reconnect primitives, guarded selected-tab extension batches, actual CDP FastKeys batches, IME composition and commit dispatch, deterministic task expansions, and a fail-closed v3 claim gate. Local tests qualify those code paths and contracts only. The v3 gate rejects negative or non-finite measurements before applying protocol-call and schema-size limits, and requires separate verified, versioned live-smoke records for each of the six supported clients; generated config contracts cannot satisfy that release check.

Visible native text controls marked `data-requires-trusted` now carry `requires_trusted_events` in the shared snapshot. A narrowly guarded click may focus that exact safe text control, and the real-Chrome shared-extension regression verifies the active element and continued redaction of nested protected markers. V3 selects FastKeys for marked ASCII controls and IME for non-ASCII text when no mode is specified; an explicit mode remains authoritative. Fill still refuses marked controls. The real-browser fixture verifies snapshot/focus, and the V3 protocol fixture verifies focus-loss stopping; the packaged extension dispatcher itself remains unqualified.

An ignored real-Chrome V3 MCP regression now verifies default marked-field FastKeys through the shared-extension provider protocol: it sent 1,000 ASCII characters in 1,143.2 ms and observed 4,000 ordered trusted key events with final readback. A keydown handler that moves focus caused an `unknown` receipt before text reached the other field. Its local CDP peer does not execute the packaged extension's `dispatchBatch`, so production extension latency remains unqualified.

The extension batch path now requires negotiated batching and deadline capabilities, reauthorizes each selected-tab action, bounds each request to 64 actions and the shared message limit, and stops on the first failed guard. Its monotonic deadline check stops before dispatching subsequent actions after the batch budget expires. FastKeys submits real CDP key events without an intentional 60 ms delay. Each FastKeys batch now carries up to 10 characters (60 guarded commands) per host round trip and rechecks target, focus, selection, and value between keydown/character/keyup. A real-Chrome regression reproduced a keydown handler moving focus and verified the guard stops before text reaches the newly focused field. Partial delivery remains `unknown`. An isolated local real-Chrome CDP smoke dispatched 1,000 ASCII characters in 445.9 ms, but bypasses the shared extension dispatcher; end-to-end bridge latency and the complete event matrix remain unqualified. Partial receipts that may include prior key dispatch return `unknown`, and workflows stop on `failed`, `unknown`, or `not_dispatched` receipts, including failed runtime verification. Shared target leases use one server-wide extension namespace plus the observed tab ID, preventing session aliases from bypassing same-tab exclusion. Each action is bounded to 60 seconds and holds a 120-second target lease; an `unknown` result retains the lease through expiry. V3 workflow find now fails unless exactly one control matches, workflows halt on failed verification as well as unknown or not-dispatched delivery, and `press` and page-tool steps require immediately following runtime verification. A workflow does not hold an exclusive lease across all steps, so safe interleaving depends on each subsequent revision-bound ref rejecting drift. General near-page DOM workflow batching and the complete event matrix remain open. IME dispatch uses composition and commit operations followed by a runtime readback; OS-level IME behavior remains unqualified.

The V3 browser route now binds all V3 session actions to sessions created through that route. The extension caps native-host reconnection at eight attempts and persists the count across service-worker suspension; after exhaustion it asks the user to reload the extension. CDP disconnects invalidate transport state and never replay mutations. These tests establish bounded behavior in the extension and transport contracts, not live browser-process crash recovery or cross-process leases.

The v3 client matrix verifies generated configuration and adapter contracts for Freebuff, OpenCode, Claude Code, Codex, ChatGPT Desktop, and generic MCP. It explicitly records real-client smoke status as unverified. The selected-tab `act` and workflow surfaces explicitly discover and invoke `document.modelContext` tools. Calls require a fresh unconsumed snapshot, bounded supported schema/input/output, schema revalidation at dispatch, and identity-bound unknown-delivery handling. Unsupported/unavailable tools stop with a semantic-UI fallback receipt. [Chrome's WebMCP overview](https://developer.chrome.com/docs/ai/webmcp) describes the API as a proposed standard and notes its local human-in-the-loop focus; its [imperative API docs](https://developer.chrome.com/docs/ai/webmcp/imperative-api) document `AbortSignal` cancellation support. Controlla sends a timeout-triggered signal and tests it in-process. Whether real page tools honor cancellation, external client cancellation, and live WebMCP behavior remain unverified. Headless stress covers eight isolated real Chrome profiles locally and in the new Ubuntu lane; it does not establish full headed-background parity or crash/reconnect recovery.

Fresh local verification on source commit `54ec59f5eeb7df1bbca9ba9b53b7aa8933f93dce` passed: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked`; and `cargo test --workspace --locked -- --ignored` with installed Chrome 154.0.8037.99. The ignored Chrome run included real-CDP tests, eight-profile headless stress, shared-extension regression, and private-headless research/form/repeat/upload-selection/download-readback fixtures. The extension reconnect-only and full stress harnesses, all `check:clients/apps/bench/release/v3/docs/provenance` npm checks, dependency check, and clean-prefix package check also passed. Exact-head hosted push run [38013128986](https://github.com/Praket7/chrome-controlla/actions/runs/38013128986) and PR run [38013131341](https://github.com/Praket7/chrome-controlla/actions/runs/38013131341) both passed Linux, macOS, Windows, and headless/background stress.

The plan remains partial for full foreground/background/headless parity, production extension-path FastKeys latency and the full event matrix, OS-level IME, authenticated app persistence and uploaded-file acceptance, live client qualification, cross-process multi-agent browser races, real WebMCP cancellation, and controlled competitor runs. The broad superiority gate remains disabled without complete real competitor evidence.

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
