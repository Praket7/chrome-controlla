# Best Chrome Use v3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn Controlla PR #2 from a strong verified Chrome execution engine into a genuinely state-of-the-art agent browser runtime for real Chrome, headless Chrome, background automation, high-speed human-like typing, robust concurrent control, and first-class use from Freebuff, OpenCode, Claude Code, and Codex.

**Architecture:** Preserve the v2 safety core, then add four missing layers that current leaders emphasize: an in-browser or near-browser low-latency execution plane, explicit browser-mode abstraction for foreground/background/headless operation, a richer typed input subsystem with both fast block insertion and high-rate per-keystroke dispatch, and a client-agnostic compact integration surface. The runtime should keep semantic state and verification as the default, use visual probing only when needed, and fail closed when the user or another client changes browser state.

**Tech Stack:** Rust workspace, Chrome DevTools Protocol, Chrome extension/service worker bridge, rmcp stdio MCP, Node packaging/integration helpers, GitHub Actions, JSON benchmark harnesses.

**Spec:** `docs/design/best-chrome-use-v2.md` plus this plan's research-driven v3 deltas.

## Execution ledger — 2026-10-09

This ledger supersedes the initial unchecked template only where it cites current executable evidence. The plan is **partially implemented; v3 is not complete or release-qualified**.

| Workstream | State | Evidence / remaining gate |
|---|---|---|
| Browser modes, interaction epochs, leases, reconnect | Partial | Policy/runtime primitives and contract tests exist. Leases key shared targets by the server-wide shared-extension namespace and tab ID, so session aliases cannot bypass them. Same-client overlapping mutations are rejected; each act has a 60-second bound, its lease lasts 120 seconds, and unknown outcomes retain the lease through expiry. Workflows lease each mutation separately, not transactionally across the whole sequence; fresh refs must reject interleaving drift. Cross-mode live parity, process-crash lease recovery, and hostile concurrent-client browser tests remain open. |
| Near-page batching and typing | Partial | The selected-tab extension batch transport now has a bounded action count, per-action authorization, early stop, and a Rust FastKeys caller. Partial receipts distinguish a first guard rejection from a batch that already sent keys; workflows stop on unknown/not-dispatched actions. FastKeys has no intentional delay and uses one host batch per up to 16 characters. General DOM workflow batching, 1,000-character real-page latency/event qualification, and the complete event semantics matrix remain open. |
| IME | Partial | MCP v3 dispatches composition, commit, and readback operations; focused unit contracts pass. Real OS IME and application-specific composition qualification remain open. |
| Compact tool surface | Contract verified | `tools/list` contract requires six tools; packaged MCP discovery is checked by `scripts/package-check.sh`. |
| Client adapters | Config contract verified | `npm run check:v3` covers six config contracts. The matrix now labels live client smokes `unverified`; no broad live qualification is claimed. |
| Work/research primitives | Partial | Deterministic task expansions and bounded evidence receipts have tests. Full authenticated multi-tab research, persistent app forms, and upload/download acceptance are not qualified. |
| WebMCP | Partial | The selected-tab `act` and deterministic `workflow` surfaces can explicitly discover and invoke `document.modelContext` tools. The [WebMCP draft](https://github.com/webmachinelearning/webmcp/blob/main/index.bs) defines `getTools()`/`executeTool()` and the optional `AbortSignal` execution option; Controlla bounds the call, passes a timeout-triggered signal, and fails unknown if delivery is uncertain. An in-process fixture covers discovery, invocation, timeout signal, and schema drift. Real-browser qualification, application cooperation with cancellation, external client cancellation, and page-tool effect classification remain open. |
| Headless/background recovery | Partial | An ignored real-Chrome stress test opens and closes eight isolated headless profiles; Ubuntu CI now has a headless/background stress lane. Crash/reconnect injection and headed-background user-interference parity remain open. |
| Competitor evidence and release gate | Blocked on external measurements | Harness contracts and fail-closed claim tests pass. No controlled real competitor results exist; broad superiority claims remain disabled. |
| Repository-controlled verification | Pass on pushed head | Rust, Node, package, extension, dependency, live-Chrome, and headless-stress checks passed locally at `5ea60c1d21fad882478af78e1136f5125f524356`; hosted Linux/macOS/Windows and headless/background stress passed at the same head in run 37998710107. See `docs/verification-matrix.md`. The overall v3 plan remains partial and is not release-qualified. |

## Global Constraints

- Preserve explicit selected-tab authority and never broaden mutation rights implicitly.
- Preserve revision-bound semantic references and reject stale or replaced targets before mutation.
- Unknown mutation delivery is never silently retried.
- Caller-authored verification is never accepted as independent runtime verification.
- Keep the compact semantic route as default; screenshots are bounded fallback only.
- Add no architecture that requires a second model for deterministic operations.
- Support foreground headed Chrome, background headed Chrome, and headless Chrome with one stable execution contract.
- Support Freebuff, OpenCode, Claude Code, and Codex through stdio MCP and/or CLI skill adapters without changing core semantics.
- Maintain Linux, macOS, and Windows CI.
- Broad "best" claims remain blocked until controlled real competitor evidence passes the claim gate.

## Review Focus

- Human edits or navigation occurring during a queued action must cause fresh-state rejection or safe re-grounding, never wrong-target execution.
- Headless and background modes must preserve the same verification and stale-reference guarantees as foreground mode.
- High-speed per-keystroke typing must preserve order, modifiers, input/change/keyboard event semantics, and detect focus loss mid-stream.
- Concurrent tabs and concurrent agent clients must not globally serialize unrelated work or allow one client to steal another client's active target silently.
- Client adapters must expose a minimal tool surface and must not bloat model context with dozens of schemas.

---

### Task 1: Browser Mode Abstraction for Foreground, Background, and Headless

**Files:**
- Create: `crates/controlla-browser/src/browser_mode.rs`
- Modify: `crates/controlla-browser/src/providers.rs`
- Modify: `crates/controlla-runtime/src/lib.rs`
- Modify: `crates/controlla-runtime/src/mcp/v2_full.rs`
- Test: `crates/controlla-runtime/tests/browser_modes.rs`

**Interfaces:**
- Produces: `BrowserMode::{Foreground, Background, Headless}` and a single launch/attach contract that preserves target identity and verification across modes.

- [ ] **Step 1: Write failing tests for mode parity**

Assert that the same navigate/snapshot/fill/verify workflow succeeds in all three modes and that target/document identity checks behave identically.

- [ ] **Step 2: Run the mode tests and confirm failure**

Run: `cargo test -p controlla-runtime --test browser_modes --locked`
Expected: FAIL because mode abstraction does not yet exist.

- [ ] **Step 3: Implement `BrowserMode` and mode-specific launch policy**

Add exact launch/attach behavior for foreground headed, background headed/minimized, and headless sessions without changing semantic-state or authority semantics.

- [ ] **Step 4: Run the mode tests**

Run: `cargo test -p controlla-runtime --test browser_modes --locked`
Expected: PASS.

- [ ] **Step 5: Commit**

Commit message: `feat: add foreground background and headless browser modes`

### Task 2: In-Browser Low-Latency Execution Plane

**Files:**
- Modify: `extensions/chrome-controlla/background.js`
- Modify: `crates/controlla-runtime/src/shared_page/guard.js`
- Modify: `crates/controlla-runtime/src/shared_page/outcome.js`
- Modify: `crates/controlla-runtime/src/browser_workflow.rs`
- Test: `extensions/chrome-controlla/test-background.cjs`
- Test: `crates/controlla-runtime/tests/v2_stdio.rs`

**Interfaces:**
- Produces: a bounded batch executor that can run multiple deterministic DOM/CDP-adjacent actions inside the browser bridge with one host round trip while rechecking authority/document identity at mutation boundaries.

- [ ] **Step 1: Add failing batch-execution tests**

Cover 20 deterministic actions, mixed read/mutation sequences, stale document invalidation mid-batch, and user navigation during execution.

- [ ] **Step 2: Verify failure before implementation**

Run extension and runtime targeted tests.

- [ ] **Step 3: Implement batch execution**

Keep one request/one response for a bounded action list. Re-read live document/target identity before each mutation and return partial receipts plus the first unsafe transition.

- [ ] **Step 4: Add latency telemetry**

Record host round trips, in-browser action count, batch execution time, and fallbacks.

- [ ] **Step 5: Run tests and commit**

Commit message: `feat: execute deterministic browser batches near the page`

### Task 3: Dual Typing Engine — Ultra-Fast Block and High-Rate Individual Keystrokes

**Files:**
- Modify: `crates/controlla-browser/src/input_strategy.rs`
- Create: `crates/controlla-browser/src/typing.rs`
- Modify: `crates/controlla-runtime/src/mcp/v2_full.rs`
- Modify: `crates/controlla-runtime/src/browser_workflow.rs`
- Test: `crates/controlla-runtime/tests/typing_modes.rs`

**Interfaces:**
- Produces: `TypingMode::{Block, FastKeys, HumanKeys, Ime}` with explicit dispatch semantics.
- `Block`: O(1) or chunked insertion for ordinary controls.
- `FastKeys`: actual ordered key events with effectively zero intentional delay, batched to minimize host round trips.
- `HumanKeys`: actual per-key events with configurable bounded delay/jitter.
- `Ime`: composition-safe route.

- [ ] **Step 1: Write failing semantic typing tests**

Assert final values plus `keydown`, `beforeinput`, `input`, `keyup`, selection movement, modifier behavior, and focus-loss detection.

- [ ] **Step 2: Add performance tests**

Require Block to avoid O(characters) protocol calls and FastKeys to dispatch a 1,000-character ASCII payload without a 60ms-per-character floor.

- [ ] **Step 3: Implement the typing engine**

Remove the current fixed 60ms strict delay as the only strict path. Keep a human-like mode separate from a zero-delay fast-key mode.

- [ ] **Step 4: Add automatic strategy selection**

Ordinary fill uses Block. Trusted-event/custom controls prefer FastKeys. Sensitive or intentionally human-paced flows may select HumanKeys. IME-required controls use Ime.

- [ ] **Step 5: Run tests and commit**

Commit message: `feat: add block fast-key human-key and ime typing modes`

### Task 4: User-Change Resistance and Cooperative Concurrency

**Files:**
- Create: `crates/controlla-runtime/src/interaction_epoch.rs`
- Modify: `crates/controlla-runtime/src/jobs.rs`
- Modify: `crates/controlla-runtime/src/scheduler.rs`
- Modify: `crates/controlla-runtime/src/semantic_state.rs`
- Modify: `extensions/chrome-controlla/background.js`
- Test: `crates/controlla-runtime/tests/user_interference.rs`

**Interfaces:**
- Produces: monotonic interaction epochs for target, focus, selection, document, viewport, and semantic-state changes; mutations carry the epoch they were grounded against.

- [ ] **Step 1: Add interference tests**

Simulate human click, typing, focus shift, tab change, navigation, SPA rerender, and another CDP client modifying the page between observation and action.

- [ ] **Step 2: Implement epoch capture and mutation preflight**

Reject or re-ground when relevant epochs move. Do not treat harmless unrelated changes as fatal if the exact target/postcondition remains valid.

- [ ] **Step 3: Implement cooperative target leases**

Allow unrelated tabs to execute concurrently while serializing conflicting mutations to the same target/document.

- [ ] **Step 4: Run tests and commit**

Commit message: `feat: resist live user and multi-client browser changes`

### Task 5: Session and Target Ownership for Multiple Agent Clients

**Files:**
- Create: `crates/controlla-runtime/src/client_sessions.rs`
- Modify: `crates/controlla-runtime/src/mcp/v2.rs`
- Modify: `crates/controlla-runtime/src/mcp/v2_full.rs`
- Modify: `crates/controlla-runtime/src/shared_page_tests.rs`
- Test: `crates/controlla-runtime/tests/multi_client.rs`

**Interfaces:**
- Produces: client/session IDs, explicit target ownership, read sharing, and conflict-safe mutation leases.

- [ ] **Step 1: Add multi-client tests**

Cover two agents on different tabs, two agents reading one tab, conflicting writes to one control, client disconnect, and stale lease recovery.

- [ ] **Step 2: Implement client/session ownership model**

Ensure one client cannot silently take a mutable target from another. Reads may share when safe.

- [ ] **Step 3: Add cleanup on disconnect and crash**

Leases expire safely without expanding authority.

- [ ] **Step 4: Run tests and commit**

Commit message: `feat: add conflict-safe multi-agent browser sessions`

### Task 6: Compact Universal Agent Surface

**Files:**
- Modify: `packages/chrome-controlla/plugin/skills/browser-control/SKILL.md`
- Modify: `crates/controlla-runtime/src/mcp/v2_full.rs`
- Create: `docs/design/compact-agent-surface-v3.md`
- Test: `crates/controlla-runtime/tests/v2_stdio.rs`

**Interfaces:**
- Produces exactly six default agent tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, `verify`; advanced diagnostics are hidden behind capability negotiation or a secondary namespace.

- [ ] **Step 1: Add tool-schema budget tests**

Assert the default tool count and maximum serialized schema size.

- [ ] **Step 2: Collapse overlapping tools behind action discriminators**

Keep the high-level surface tiny while preserving internal specialized handlers.

- [ ] **Step 3: Add result-size budgets and semantic deltas**

Prevent accidental context blowups from full trees or oversized images.

- [ ] **Step 4: Run tests and commit**

Commit message: `feat: shrink the default browser agent command surface`

### Task 7: First-Class Freebuff, OpenCode, Claude Code, and Codex Adapters

**Files:**
- Create: `integrations/freebuff/`
- Create: `integrations/opencode/`
- Create: `integrations/claude-code/`
- Create: `integrations/codex/`
- Modify: `integrations/generate-config.mjs`
- Modify: `tests/clients/check-configs.mjs`
- Modify: `docs/clients.md`

**Interfaces:**
- Produces install/config artifacts and skills that all launch the same `controlla-v2` stdio server and preserve one long-lived browser session.

- [ ] **Step 1: Add config-generation tests for all four clients**

Assert executable path, environment propagation, persistent server lifetime, permissions guidance, and exact tool names.

- [ ] **Step 2: Implement OpenCode, Claude Code, and Codex configs**

Use native MCP registration conventions for each client.

- [ ] **Step 3: Implement Freebuff adapter**

Provide the thinnest compatible MCP/skill layer available in the project while keeping Controlla as the browser execution engine.

- [ ] **Step 4: Add smoke-contract docs and tests**

Each client must complete navigate, snapshot, fill, verify, and multi-call persistence with the same semantics.

- [ ] **Step 5: Commit**

Commit message: `feat: add freebuff opencode claude code and codex integrations`

### Task 8: Research, Assignment, and Work Automation Primitives

**Files:**
- Create: `crates/controlla-runtime/src/task_primitives.rs`
- Modify: `crates/controlla-runtime/src/capability_router.rs`
- Modify: `crates/controlla-runtime/src/browser_workflow.rs`
- Test: `crates/controlla-runtime/tests/task_primitives.rs`

**Interfaces:**
- Produces bounded primitives for multi-page research, table/list extraction, citation URL capture, form completion, file download/upload, repeated item workflows, and save-without-submit behavior.

- [ ] **Step 1: Add realistic task tests**

Cover research across 10 tabs, extracting cited facts, filling a multi-page school/work form without final submission, uploading a document, downloading an export, and revisiting a previous authenticated page.

- [ ] **Step 2: Implement task primitives as deterministic workflow expansions**

Do not add a second autonomous reasoning loop. Expand high-level requests into existing verified operations.

- [ ] **Step 3: Add evidence receipts**

Return URLs, extracted snippets/fields, affected controls, and verification results without dumping full page state.

- [ ] **Step 4: Run tests and commit**

Commit message: `feat: add research and productivity workflow primitives`

### Task 9: Native Page Tools / WebMCP Route

**Files:**
- Create: `crates/controlla-browser/src/page_tools.rs`
- Modify: `crates/controlla-runtime/src/capability_router.rs`
- Modify: `crates/controlla-runtime/src/mcp/v2_full.rs`
- Test: `crates/controlla-runtime/tests/page_tools.rs`

**Interfaces:**
- Produces discovery/invocation of page-exposed structured tools when Chrome exposes them, ahead of UI driving in the capability hierarchy.

- [ ] **Step 1: Add fixture page-tool tests**

Assert tool discovery, JSON-schema validation, invocation, timeout, cancellation, and fallback to semantic UI when no page tool exists.

- [ ] **Step 2: Implement page-tool routing**

Treat page-provided tools as privileged untrusted capabilities and keep output size/authority bounded.

- [ ] **Step 3: Run tests and commit**

Commit message: `feat: prefer structured page tools before ui automation`

### Task 10: Headless/Background Reliability Hardening

**Files:**
- Create: `crates/controlla-runtime/tests/headless_stress.rs`
- Modify: `crates/controlla-browser/src/providers.rs`
- Modify: `crates/controlla-runtime/src/runtime_observer.rs`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces crash/reconnect/restart handling that cannot loop forever and never replays an uncertain mutation.

- [ ] **Step 1: Add failure-injection tests**

Cover renderer crash, CDP disconnect, service-worker restart, browser restart, headless startup failure, reconnect success followed by immediate close, and stale transport handles.

- [ ] **Step 2: Implement bounded reconnect state machine**

Cap retries, distinguish read retry from mutation uncertainty, and surface actionable terminal states.

- [ ] **Step 3: Add CI headless stress lane**

Run repeated headless workflows on Linux and one headed/background smoke test where supported.

- [ ] **Step 4: Run tests and commit**

Commit message: `fix: harden headless and background browser recovery`

### Task 11: Real Competitive Benchmark Expansion

**Files:**
- Modify: `bench/tasks/phase11-realistic.json`
- Modify: `bench/runners/phase11-suite.mjs`
- Modify: `bench/analysis/phase11-analysis.mjs`
- Create: `bench/tasks/phase12-agent-workloads.json`
- Create: `bench/analysis/phase12-analysis.mjs`
- Test: benchmark runner tests

**Interfaces:**
- Produces controlled comparison against Stagehand v4, Playwright MCP/CLI, Browser Use harness/CLI, Chrome DevTools MCP, agent-browser, and at least one strong autonomous browser agent track such as Ramain if reproducibly runnable.

- [ ] **Step 1: Add workloads that expose current gaps**

Include 1,000-char typing, block fill, live user interference, background headed, headless, 8-tab concurrency, research with citations, long form completion, authenticated persistence, WebMCP/native tool route, dynamic SPA rerender, shadow DOM, nested iframe, and file upload/download.

- [ ] **Step 2: Add measurements**

Track success, verified success, severe safety failures, wall time, model round trips, browser round trips, command count, tokens, action count, stale-target recoveries, and user-interference recoveries.

- [ ] **Step 3: Add warm-repeat benchmark**

Compare first run versus qualified-skill replay/action cache.

- [ ] **Step 4: Require complete raw evidence before claim eligibility**

Keep generated CI rows ineligible.

- [ ] **Step 5: Commit**

Commit message: `bench: expand best-browser evidence to real agent workloads`

### Task 12: Client-Specific End-to-End Qualification

**Files:**
- Create: `apps/client-qualification-matrix.json`
- Create: `apps/check-client-qualification.mjs`
- Modify: `package.json`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces explicit qualification states for Freebuff, OpenCode, Claude Code, Codex, ChatGPT Desktop, and generic MCP.

- [ ] **Step 1: Define qualification gates**

Require startup, persistent session, headless, background, foreground, typed input modes, multi-tab, user interference, upload/download, research extraction, and safe mutation verification.

- [ ] **Step 2: Add contract tests and optional real-client lanes**

Fail closed when a client version or config is unverified.

- [ ] **Step 3: Commit**

Commit message: `test: qualify supported agent clients explicitly`

### Task 13: Release Gate v3

**Files:**
- Modify: `scripts/check-best-chrome-release-gate.mjs`
- Modify: `scripts/check-best-chrome-release-gate.test.mjs`
- Modify: `docs/verification/best-chrome-use-status.md`
- Modify: `docs/design/best-chrome-use-v2.md`

**Interfaces:**
- Produces a v3 release gate requiring browser-mode parity, typing performance/semantics, interference resistance, multi-client safety, supported-client qualification, and real benchmark evidence.

- [ ] **Step 1: Add failing gate tests for each missing evidence class**

- [ ] **Step 2: Add explicit numeric thresholds**

Preserve v2 safety thresholds and additionally require materially lower command count and browser round trips on multi-step workloads, no fixed per-character delay in FastKeys, and no reliability regression across foreground/background/headless modes.

- [ ] **Step 3: Update public claim policy**

Permit only measured, reproducible category claims unless the complete v3 suite passes.

- [ ] **Step 4: Run release tests and commit**

Commit message: `feat: gate best-browser claims on v3 evidence`

### Task 14: Full Verification and PR Review

**Files:**
- Modify: `.superpowers/sdd/2026-10-09-controlla-best-chrome-use.md`
- Modify: `docs/verification/best-chrome-use-status.md`

**Interfaces:**
- Produces final evidence that implementation and test gates pass before PR #2 is mergeable again.

- [ ] **Step 1: Run rustfmt, clippy, workspace tests, extension tests, package tests, client checks, app qualification, benchmark harness tests, release-gate tests, and dependency/package checks**

- [ ] **Step 2: Run Linux, macOS, and Windows CI**

Expected: all green.

- [ ] **Step 3: Run real Phase 11/12 competitor measurements where credentials/runtimes permit**

Do not substitute generated rows.

- [ ] **Step 4: Run whole-branch code review**

Focus on wrong-target risk, reconnect semantics, typing event correctness, multi-client races, and adapter privilege boundaries.

- [ ] **Step 5: Update PR #2 body with exact evidence and remaining unsupported claims**

- [ ] **Step 6: Mark ready only when all implementation gates pass**

Commit message: `docs: record v3 best-chrome verification evidence`
