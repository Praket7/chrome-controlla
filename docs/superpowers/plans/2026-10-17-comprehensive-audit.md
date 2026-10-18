# Controlla repository audit and implementation

Goal: reliable, efficient Chrome control across foreground, background and private headless sessions, with exact target identity, bounded observations, secret isolation and explicit permission for consequential actions.

Baseline: `0fb0dfd356402ec81cd9d47a422461c931448aa6`, verified against GitHub main on 2026-10-17. Work runs in the existing isolated checkout on `codex/comprehensive-chrome-audit`.

The user explicitly requested implementation without intermediate permission stops. Execute inline, retaining failures and requesting independent code review. Preserve the six-tool contract. Research is evidence for engineering decisions, not proof of comparative performance.

## Work ledger

- [x] Verify repository identity, clean checkout and GitHub baseline.
- [x] Consult GitHub, Exa, alphaXiv, SciSpace and Parallel Search.
- [x] Inventory tracked files and reconcile requested capabilities against executable code, not older status documents.
- [x] Run baseline Rust and JavaScript suites; retain failures.
- [x] Fix hidden screenshot renderer evidence: skip suspended animation frames for hidden targets, require a real renderer sample, preserve document/lifecycle checks, and bound fallback to one capture retry.
- [x] Fix watcher completeness on incremental capacity overflow; incomplete inventory cannot authorize actions.
- [x] Audit and improve tab/window/group operations and the compact agent instructions.
- [x] Resolve concrete findings from independent runtime review.
- [x] Run relevant regressions, formatting, lint, repository checks and isolated real Chrome fixtures.
- [x] Update progress, blockers and verification matrix with exact evidence and unqualified requirements.
- [x] Independent final review and final diff inspection.

## Review focus

1. Failed renderer reads must never become successful freshness evidence.
2. Hidden captures must not focus or activate Chrome, and must not wait on suspended RAF callbacks.
3. Incremental tab/window/group overflow must invalidate completeness until fresh inventory.
4. Tab topology mutations must validate exact selected identities, serialize writes and report uncertain delivery without replay.
5. Runtime and extension command contracts must agree; passwords must not enter general observations or durable workflows.

## Verification contract

For each nontrivial fix, add a failing behavioral regression, observe failure, implement, and rerun. Full Rust and extension suites are the local regression gates. Live user-account behavior, third-party anti-bot checks, client prompt-submit telemetry, smaller-model task success and matched competitor benchmarks require separate measured evidence. Never label structural improvements as world-leading performance.

## Independent task expansion after baseline audit

Ruling: use Luna Medium implementation subagents for the independent proxy and benchmark fixes, with one scoped review each and final branch review. The user has explicitly constrained all future subagents to Luna Medium. Main-agent runtime work remains inline. All work stays on this branch; no shared-branch merge or release is part of this pass.

### Proxy cancellation and concurrency

Files: `packages/chrome-controlla/plugin/mcp-agent-ux-launch.cjs`, `agent-ux-proxy.cjs`, `packages/chrome-controlla/test-agent-ux.cjs` only.
Allow a later cancellation request and independent tool requests to reach the child while an earlier tool call is pending. Keep schema refresh serialized/single-flight, context identities exact and response IDs correlated. Pending mutations must fail closed on schema replacement/child exit. Add real child-process regressions with a held workflow and concurrent cancel, and existing-context concurrent calls. Run the full package agent-UX suite.

### Benchmark integrity

Files: `bench/adapters/phase11-*.mjs`, `bench/runners/phase11-*.mjs`, relevant tests, `scripts/run-phase11-suite.mjs` and its new tests.
A visit must not satisfy a mutation task. Unsupported task/adapter combinations must produce unqualified/missing evidence, never fabricated success. Mark uninstrumented safety counters missing. Do not label sequential work as concurrency; either actually run isolated concurrent tasks or reject unsupported levels. Qualified provenance must bind a clean Git tree, full extension build assets and real adapter/browser versions; refuse unsupported evidence rather than inventing identifiers. Add fail-then-pass negative controls and run the affected benchmark suites. Preserve reports as historical evidence.

| Pair/task | Interface and conflict check |
| --- | --- |
| Runtime/proxy | Same six tools; proxy dispatch changes do not alter runtime schema. Disjoint files. |
| Runtime/benchmark | Benchmark must report missing measurements, not infer success from new receipts. Disjoint files. |
| Proxy | Concurrent dispatch and correlated responses tested against held child requests; no global context fallback races. |
| Benchmark | Strict evidence may render old convenience runs ineligible; this is intended and must be documented. |
