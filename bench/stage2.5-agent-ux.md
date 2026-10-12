# Stage 2.5 — Agent orchestration latency qualification

Stage 2 optimized browser execution. Stage 2.5 measures the larger quantity the user actually experiences: **prompt -> first useful browser command -> independently verified completion**.

A browser operation taking ~1.4 seconds is not a fast user experience if the agent spends minutes discovering MCP tools, rereading skills, searching repository source, inspecting schemas, taking duplicate snapshots, or navigating through a homepage that could have been bypassed.

## Research basis

The Stage 2.5 design follows convergent results from current agent research and harness guidance:

- **Skim: Speculative Execution for Fast and Efficient Web Agents** (arXiv:2605.16565) exploits predictable site structure, synthesizes likely destination URLs, verifies the fast path, and falls back when confidence is insufficient.
- **WALT: Web Agents that Learn Tools** (arXiv:2510.01524) replaces repeated low-level browser reasoning with reusable high-level site operations.
- **PAFFA: Premeditated Actions For Fast Agents** (arXiv:2412.07958) uses reusable verified action APIs to reduce repeated LLM/browser reasoning.
- **Agent JIT Compilation for Latency-Optimizing Web Agent Planning and Scheduling** (arXiv:2605.21470) compiles task descriptions into executable tool programs instead of serial ReAct turns.
- **AOSpec** (arXiv:2608.00881), **PASTE** (arXiv:2603.18897), and **Speculative Macro Commit** (arXiv:2609.03236) attack the serialized model/tool critical path with guarded speculation and verification.
- **PreAct** (arXiv:2606.17929), **EchoPath** (arXiv:2609.16635), **WikiSkill** (arXiv:2608.27454), and **WebCoach** (arXiv:2511.12997) support reuse of previously verified trajectories/skills instead of solving repeated browser tasks from scratch.
- OpenAI Codex guidance recommends small `AGENTS.md` files and progressive-disclosure skills. The agent should begin from a compact stable entry point rather than rereading large manuals or source before routine work.

The design principle is therefore:

> **qualified memory / safe speculative route -> verification -> deterministic local program -> semantic planning -> visual fallback**

rather than repeated `reason -> observe -> reason -> observe` loops.

## Stage 2.5 hot-path gates

For ordinary known or safely routable browser tasks:

| Metric | Gate |
|---|---:|
| prompt -> first Controlla browser command | <= 5 s target |
| MCP discovery before browser work | 0 |
| schema lookups before browser work | 0 |
| `browser describe` before browser work | 0 |
| repository/source reads for tool usage | 0 |
| documentation/memory reads for tool usage | 0 |
| broad snapshots when a targeted read suffices | 0 |
| navigation detours when a direct route is available | 0 |
| duplicate observations | 0 |
| model-visible observations before first mutation | <= 1 |

A schema/tool mismatch is a recovery exception, not normal startup behavior. Automatic client-side schema refresh should absorb schema drift without forcing the model to inspect schemas.

## ESPN newest-Bills regression

Prompt:

`Open ESPN and navigate to the most recent Buffalo Bills story.`

Expected execution shape:

1. Resolve or safely synthesize the Bills-specific public destination.
2. Open it directly.
3. Verify that the page is the Buffalo Bills destination.
4. Read the narrow Bills-story candidate set once.
5. Choose the newest candidate using the strongest observed publication evidence.
6. Click once.
7. Verify the destination when the click receipt does not already prove it.
8. Release the session and leave the story tab open.

The following are explicit failures when the direct route is available:

- ESPN homepage first
- generic NFL homepage first
- MCP discovery or schema lookup
- reading `MEMORY.md`, `SKILL.md`, `mcp.rs`, `v3_full.rs`, `observe.rs`, or similar source to learn routine arguments
- two broad snapshots of the same page
- repeated `extract`/snapshot probing caused by uncertainty about tool syntax

## Safe speculative public routes

A first-time route does not need to be observed through menus if it can be synthesized with high confidence from a stable public site pattern. The speculation is allowed only when:

- the destination is HTTP(S)
- it has no embedded credentials or secret-bearing query parameters
- it is a public navigation/read route, not a submit/delete/purchase/account side-effect endpoint
- page identity can be verified cheaply after open
- mismatch causes one semantic fallback, not repeated guessing

Only independently verified successes are eligible for persistent route memory. Route memory is exact-origin scoped, bounded, expiring, and quarantined on structural drift, safety failure, or poor rolling verification.

## Full-run accounting

Every measured run should separately report:

- `prompt_to_first_browser_ms`
- `total_prompt_to_verified_ms`
- measured browser work
- orchestration overhead = total prompt-to-verified minus measured browser work
- schema/MCP discovery calls
- repository/source reads
- documentation/memory reads
- describe calls
- broad vs targeted snapshots
- navigation detours
- duplicate observations
- model-visible observations before the first mutation
- browser command count
- success and independent verification

The Stage 2.5 scorer is `bench/runners/stage2.5-agent-ux-score.mjs`, with regression tests in `stage2.5-agent-ux-score.test.mjs`. Task definitions are in `bench/tasks/stage2.5-agent-ux.json`.

## Release evidence

Stage 2.5 is not qualified by browser-only timing. A release claim requires matched end-to-end agent traces from the exact candidate revision. The scorer can prove trace-policy compliance, but it cannot prove a real Codex/Chrome run occurred. Live qualification must preserve raw traces and exact revision identity.
