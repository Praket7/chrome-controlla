# Phase 11 latency research notes

This is design evidence for the benchmark and runtime roadmap. None of the reported speedups below are Controlla results.

## Findings that change the measurement plan

- **Attribute time across the host/client/server boundary.** ProMCP measures latency and payload across MCP components and reports that planning/schema injection or result synthesis can dominate in some host/server configurations. This supports measuring tool-catalog readiness, first tool dispatch, browser result, independent verification, and final answer separately. Its workload is a mixed MCP benchmark, not a browser-control comparison, so its percentages should not be projected onto Controlla. [Findings of ACL 2026 paper](https://aclanthology.org/2026.findings-acl.1967/)
- **Keep tool catalogs stable and ordered.** The MCP tools specification recommends deterministic tool ordering and supports pagination/list-change behavior. Controlla should keep the six eager tools stable, avoid routine discovery, and measure the size of schemas delivered to clients. This is protocol guidance, not a measured latency win. [MCP tools specification](https://modelcontextprotocol.io/specification/2026-07-28/server/tools)
- **Compile deterministic work under one task call, with live guards.** Agent JIT Compilation selects plans using expected cost and schedules local execution; its reported results are from a bounded five-app evaluation. The relevant design signal is to keep deterministic navigation/read/verification inside one task call and retain immediate guards at mutations. The paper's speed ratios are not Controlla estimates. [ICML/PMLR 2026 paper](https://arxiv.org/abs/2605.21470)
- **Validate final state independently.** WebArena uses programmatic task validators over repeatable web environments. Controlla's live-site benchmark should pair realistic production qualification with resettable fixtures and independent postcondition checks, recording false-success as well as success. A fixture pass does not establish production-site behavior. [WebArena paper](https://arxiv.org/abs/2307.13854)
- **Speculation must be bounded to safe, cheap reads.** SPAgent reports task-specific speed gains from adaptive speculation in search agents. This motivates measuring whether speculative reads save wall time under load; it does not justify speculative mutations or imply the same gains for browser tasks. [SPAgent paper](https://arxiv.org/abs/2511.20048)

## Measurement consequences

1. Name the exact clock origin. The current public pilot starts immediately before codex exec is spawned; it cannot observe prompt dispatch inside Codex, so its times are process-spawn-to-event, not exact prompt-SEND latency.
2. Keep browser operation time, independent verification, and final-answer time distinct. Capture input/output tokens separately and preserve failed runs in the denominator.
3. Alternate adapter order and use identical site state, browser build, model, task semantics, and independent verifier. Report sample counts and uncertainty; a five-pair, one-site pilot is still not a release benchmark.
4. Separate cold browser, warm browser/cold route, and warm qualified route cohorts. Do not average them.
5. Do not make a competitive claim until the full preregistered workload, client adapters, resets, independent verifiers, and hosted candidate CI all pass.
