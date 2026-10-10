# Controlla Best Chrome Use — SDD Ledger

Starting point: `62e6407d0e75b83a1be8d69468476601a6e6da37`

## Completion state

- Persistent shared-Chrome session continuity and explicit selected-tab authority are preserved.
- Compact semantic snapshots, revision-bound short refs, changed-only deltas, and guarded reference resolution are implemented.
- Verified bulk fill/select and strict retained-node typing/press paths are implemented with stale-document checks and unknown-delivery semantics.
- Deterministic MCP v2 workflow actions cover snapshot, navigate, find, click, fill, type, press, select, wait, observe, extract, assert, verify, and checkpoint. Shared-route scripts fail closed.
- Runtime-owned verification is required for qualification-sensitive skill outcomes. Caller-supplied verification evidence is not accepted.
- Persistent reusable skill records support candidate, qualification, replay, drift quarantine, expiry, and runtime verification.
- Targeted visual probes capture bounded element/selector/region PNG crops and discard stale captures after document identity changes.
- Agent-facing MCP v2 exposes session, snapshot, find, act, extract, workflow, probe, verify, and skill tools while retaining v1 compatibility.
- ChatGPT Desktop/local MCP integration and benchmark/release documentation are present.
- One-time patch transport used during implementation removed itself from the tree. The source was formatted before the final verification run.
- The strict typing path has also been normalized for the Rust 1.99 clippy gate before this final matrix run.

## Safety invariants

- Never silently broaden selected-tab authority.
- Never treat stale semantic refs as valid mutations.
- Never retry an unknown mutation outcome without fresh observation.
- Never accept caller-authored evidence as independent verification.
- Prefer semantic state first and bounded visual probes only when semantics are insufficient.
- Public superiority claims remain gated on reproducible benchmark evidence rather than implementation status alone.

## Verification gate

The final branch is only considered complete after the repository CI matrix passes on Linux, macOS, and Windows, including rustfmt, clippy with `-D warnings`, workspace tests, package checks, client/app/benchmark/release checks, extension tests, and dependency/package validation.

## V3 execution update — 2026-10-09

- Ruling: reject all V3 data/action routes unless the session was created through the V3 browser route — legacy sessions bypassed mode authority; the cost if wrong is rejecting callers that paired through legacy V2, which is intentional for V3-only tools. The regression fixture now expects the unpaired route to fail before dispatch.
- Ruling: route bounded task and opaque artifact actions through `browser` to keep exactly six packaged tools — the v3 plan requires a tiny default surface and all operations already use bounded discriminated inputs; the cost is a slightly wider `browser` schema, held below the existing 48 KiB budget.
- Ruling: extension native-host reconnect is capped at eight attempts and survives service-worker suspension, then requires extension reload — retries cannot continue indefinitely; the cost is manual recovery after the bound. CDP disconnect invalidates the transport and does not replay uncertain mutations.
- Fresh local verification passed on source commit `54ec59f5eeb7df1bbca9ba9b53b7aa8933f93dce`: Rust format, warning-denied workspace Clippy, full workspace tests, all ignored installed-Chrome tests (Chrome 154.0.8037.99), extension reconnect/full stress, client/app/benchmark/release/v3/docs/provenance checks, dependency check, and clean-prefix package check. Exact-head hosted push run [38013128986](https://github.com/Praket7/chrome-controlla/actions/runs/38013128986) and PR run [38013131341](https://github.com/Praket7/chrome-controlla/actions/runs/38013131341) both passed Linux, macOS, Windows, and headless/background stress.
- Remaining gates: six-tool packaged surface, headed-background and cross-mode live parity, full production extension FastKeys event/latency matrix, OS IME, authenticated app persistence/file acceptance, adversarial cross-process agent races, live client and WebMCP qualification, and controlled competitor measurements. These are not converted into passes by local fixtures; the broad claim remains fail-closed.
