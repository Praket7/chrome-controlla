# Chrome Controlla: research-backed improvement roadmap

Prepared 2026-10-08. Read [handoff.md](handoff.md) for the exact checkout, code map, evidence boundaries and source findings. This is a **new roadmap**, not a replacement for [the original improvement specification](docs/design/improvements.md), [build plan](docs/design/build.md), or [research](docs/design/research.md).

> Status update, 2026-10-09: A1–A8 received independent Sol review; the four findings were fixed and regression-checked. Durable Chrome `webNavigation.documentId` binding now flows from inventory through attach and commands with an explicit capability handshake. The extension rechecks current authorization after the asynchronous document lookup, reserves tabs synchronously across native and popup pairing, and waits for a pending per-tab debugger detach before reattaching. Regression fixtures cover these interleavings. Workspace, Clippy, direct-CDP isolated Chrome, packaging, dependency, docs, provenance, client/app, and benchmark checks pass. Final Sol source/fixture review approved the fixes and did not rerun tests. The enabled local MCP entry was served by the live Hotload child and the current extension paired with a local fixture. Live fill/type/click/post-mock, navigation invalidation and re-pair, observation, release, and tab cleanup passed; file-input upload remained blocked fail-closed. These checks do not establish external app persistence or real-site workflows. See [the verification matrix](docs/verification-matrix.md) for exact evidence and limits. App save/export workflows, a calibrated verifier/cache, fair held-out comparison, the full client matrix, and a published release remain future work.

## 1. The central recommendation

Build a dependable closed loop: **identify the intended target → perform a bounded action → independently verify the requested result → retain enough evidence to recover safely**. Make that loop inexpensive enough to reuse across workflows. Finish one real application workflow before broadening the advertised capability set.

The current repository already has useful foundations: a compact Rust runtime, native shared-tab bridge, target/session model, journal, bounded observations, guarded workflow mechanics, and test infrastructure. Its main weakness is the distance between those foundations and a user's completed task. Repeated connection trouble, plans presented alongside execution tools, missing save/export observers and incomplete client qualification matter more than another list of commands.

“Best Chrome use” is a goal, not an established result. Rust, an MCP server, semantic references and workflow caching are not sufficient novelty: competing systems already offer several of these. The defensible opportunity is **verified completion with low interaction cost, low user disruption and reliable recovery**, measured against capable baselines.

### A measurable product contract

| Dimension | Measure | Required reporting discipline |
| --- | --- | --- |
| Completion | Independently verified task success, not dispatch success | Report task denominator, unsupported/blocked cases and verifier error. |
| Reliability | Repeated-run success; false success; duplicate/uncertain side effects | Report adverse outcomes separately; no success credit for a plausible trace alone. |
| Speed | End-to-end latency, p50/p95, setup/recovery time | Include verification, retries and user setup; separate warm/cold sessions. |
| Efficiency | Model input/output tokens, bytes observed, tool round trips, total cost | Count actual wire/schema overhead and all repair turns; keep prices/configurations dated. |
| User coexistence | Foreground steals, clipboard changes, dropped/corrupted user input | Measure with an independent observer on supported environments. |
| Recoverability | Time and success after worker/tab/extension/client restart | Distinguish recovery without replay from action replay. |
| Coverage | Qualified provider × OS × app × action × client combinations | Unsupported is a first-class result; don't average it out. |
| Quality | Saved artifact meets task-specific content/layout/export rubric | Use deterministic checks plus calibrated human/visual review where necessary. |

Choose numerical release thresholds before evaluation. Do not tune the threshold after seeing held-out results. Use a Pareto comparison of success, cost and latency rather than a single marketing score that hides tradeoffs.

## 2. Research method and what was actually consulted

Fresh research used the requested **Parallel Search, Exa, alphaXiv, SciSpace and GitHub** capabilities. Primary official documentation and repositories were fetched; alphaXiv full text was read for the verifier/evaluation papers below. SciSpace corroborated workflow-reuse literature, including WALT; weaker generic hits were excluded from architectural conclusions. Superpowers guided source debugging/review and evidence discipline.

MagicPath's integration instructions and component search were consulted. The Controlla component search returned no existing design. No paid/generated remote design project was created just to satisfy a research request. Section 12 gives a concrete interface specification that a successor can implement or prototype with MagicPath if desired.

Source statements below are not Controlla results. Research preprints, experimental browser APIs and vendor repositories have different evidence strength. Dates and API support must be refreshed before implementing unstable contracts.

### Sources connected to this codebase

| Source | What it supports | Concrete consequence for Controlla |
| --- | --- | --- |
| [Playwright MCP](https://github.com/microsoft/playwright-mcp) | Current project guidance distinguishes MCP's persistent introspection from CLI/skills efficiency for coding agents. | Evaluate CLI and MCP over the same core. A small tool count alone does not prove token efficiency. |
| [Playwright actionability](https://playwright.dev/docs/actionability) | Per-action uniqueness, visibility, stability, event reception, enabled/editable checks and retrying assertions. | Replace generic “element exists” checks with action-specific contracts and bounded condition waits. Apply to direct and shared routes. |
| [agent-browser](https://github.com/vercel-labs/agent-browser) | Native Rust CLI, semantic/accessibility references, persistent browser commands and common automation capabilities. | Benchmark against this class of compact tooling; do not claim Rust or references are new. Compare observation/delta behavior and recovery. |
| [Chrome DevTools for agents](https://developer.chrome.com/docs/devtools/agents/get-started) | Official agent-oriented MCP/CLI/skills and browser diagnostics/performance access. | Include modern semantic/debugging baselines, not just screenshot clicking. Use bounded traces for diagnosis rather than logging all page data. |
| [Chrome debugger API](https://developer.chrome.com/docs/extensions/reference/api/debugger) | Restricted CDP domains; frame/target distinctions; flat sessions and child-target attachment behavior. | Model frame, execution context and debugger session explicitly; test same-process iframes and OOPIFs. A tab ID alone is insufficient. |
| [Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging) | Native-host manifests/origins and framed stdin/stdout transport, including asymmetric size limits. | Validate actual extension origin; bound message allocation and transfer sizes; diagnose setup by layer. |
| [MV3 service worker lifecycle](https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle) | Worker lifecycle varies with active connections/features; active debugger sessions keep workers alive in supported Chrome versions. | Test detach/idle/restart explicitly. Do not depend on accidental worker longevity or assume every disconnect requires reinstall. |
| [WebMCP early preview](https://developer.chrome.com/blog/webmcp-epp) | Experimental declarative/imperative page-side structured tools. | Add only an optional capability-gated route for cooperating sites; page-provided tools remain untrusted. Do not assume universal deployment. |
| [Slides batchUpdate](https://developers.google.com/workspace/slides/api/reference/rest/v1/presentations/batchUpdate) | Revision-guarded updates and response revision semantics. | Use revision-aware app operations/readback where authenticated API access is supported; label API-assisted results separately from browser-only. |
| [Canva Design Editing](https://www.canva.dev/docs/apps/design-editing/) | Snapshot-based edits and explicit sync; unsynced edits do not establish a persisted change; page/locked-element constraints. | Design a scoped Canva app integration or browser route with explicit save confirmation; planner JSON is not execution. |
| [MCP security guidance](https://modelcontextprotocol.io/specification/latest/basic/security_best_practices) | Audience, confused-deputy and credential-boundary concerns. | Keep local authentication/client scopes real; a single local bearer key is not multi-tenant isolation. |
| [BrowserGym](https://arxiv.org/abs/2412.05467) | A relevant benchmark/environment reference identified in literature discovery. | Use established task/evaluation concepts; adapting or extending it needs a reproducible environment, not a paper citation as evidence. |
| [WALT](https://arxiv.org/abs/2510.01524) and [PAFFA](https://arxiv.org/abs/2412.07958) | Relevant workflow/action reuse literature; WALT was corroborated by SciSpace. | Treat reuse as a hypothesis to test under drift and held-out tasks. Do not transplant reported gains into this project. |
| [The Art of Building Verifiers for Computer Use Agents](https://arxiv.org/abs/2604.06240) | Full-text preprint: explicit nonoverlapping criteria, separate rubric generation/scoring, selective evidence and calibration against human judgment. | Build verifier calibration and abstention before allowing cache admission or training rewards. Avoid invented task criteria and success from apparent effort. |
| [Auditing Web Agent Evaluation on WebArena-Lite](https://arxiv.org/abs/2610.01491) | Full-text recent preprint: human audit exposes evaluator errors and trajectory failures beyond final-state labels. | Retain original and adjudicated labels, locate first consequential failure, detect loops and measure partial progress without calling it success. |

The two recent papers are research evidence, not standards or proof of universal improvements. This roadmap deliberately does not claim their published performance numbers for Controlla. Other discovered papers on transferable skills, verified environments and commitment planning remain reading candidates, not premises used here.

## 3. Fix the foundations before adding intelligence

### 3.1 A connection that explains itself

**Problem:** the user repeatedly had to reload/restart without a reliable distinction between MCP availability, native-host registration, extension connection, debugger attachment and target health. `Page.getFrameTree` timing out on one target was easy to confuse with a broken entire installation.

**Change:** implement a layered health report in `doctor.rs`, `native_setup.rs`, `native_host.rs`, `providers.rs` and the extension. Keep checks read-only unless an explicit repair is requested.

Report separately:

1. Client can discover/call the current server schema.
2. Runtime executable/build and native manifest path agree.
3. Manifest allows the actual loaded extension ID.
4. Extension handshake version/protocol is compatible.
5. A fresh native inventory exists.
6. Selected tab/document is still the authorized target.
7. A bounded read-only browser command returns.

Return a small fault code and one appropriate next action per failed layer: e.g. manifest mismatch, stale extension build, unavailable host, selected target unresponsive. Never tell the user to restart everything because one tab read failed. Include timings and sanitized correlation IDs; exclude tokens and page text by default.

**First repairs:** source findings A1–A3 in the handoff. Add per-peer handshake deadlines, cancellation-safe pending cleanup, and target binding around attachment. Version runtime/extension/protocol separately and expose build hashes; 0.1.0 on every iteration cannot diagnose a stale installation.

**Acceptance:** automated install in a path with spaces; wrong extension origin; obsolete binary path; silent socket; killed native host; sleeping/waking browser; detached debugger; extension reload; one hung tab beside a healthy tab. A healthy tab must remain usable and each failure must produce the correct layer/action. No blind write replay during reconnect.

### 3.2 Precise target identity across frames and navigation

A proposed target reference should bind session generation, tab/target, frame, execution context/document generation, and the selected element identity when acting on an element. Use browser-provided identity where possible, not a fabricated revision counter that misses navigation. Define same-origin and cross-origin navigation policy deliberately.

Chrome's debugger model distinguishes same-process frames from out-of-process child targets. A flat-session implementation must propagate the correct session ID and recursively cover permitted children where required. Do not infer frame identity from URL alone. Repeated identical URLs and same-selector replacement are normal.

Retain element identity from observation to dispatch; invalidate on detach/replacement/context destruction. Re-resolve after invalidation only through a fresh observation and intent check. Fix direct-route A6 before exposing it more broadly. For actions with irreversible effects, an old reference must never quietly select a new matching button.

**Acceptance:** same-URL reload, SPA navigation, cross-origin iframe, nested OOPIF, target replacement, same-selector node substitution, popup creation, back/forward restoration, simultaneous tabs with identical titles. Verify stale references fail before mutation and only authorized targets remain attached.

### 3.3 One semantic contract, explicit provider capabilities

Today direct-library and shared-MCP behavior diverge. Introduce the smallest common typed operation contract needed by actual callers; reuse existing models rather than building a new framework. Route direct/shared providers through the same semantic validation and result vocabulary where possible. Keep provider-specific transport behind that contract.

Each action advertises supported environments and preconditions. Absent capabilities return `unsupported` with a useful supported alternative. Do not silently substitute a different browser-control product and then count the result as a Controlla test.

Start with observe, fill/type, click, select, scroll and file insertion, then drag/rich editors after demand and tests. Avoid exposing dozens of weak operations merely for breadth. Add a contract test suite that runs the same supported cases against each provider and records explicit skips.

## 4. Observation that is small, fresh and sufficient

### 4.1 Query the information needed for the decision

Use bounded observation requests: scope, requested fields, visibility policy, byte/node budget, completeness requirement and cursor. Return relevant semantic controls/text with stable references; attach document/context generation and truncation reasons. A short response that silently omits the sought item is worse than a larger honest response.

Use a progressive strategy:

1. Scoped semantic/AX query for the task.
2. Targeted DOM facts where AX lacks meaning.
3. Screenshot/crop only for spatial/visual ambiguity or quality review.
4. App-specific structured inventory when supported and authorized.

Prefer changed-region/delta observations after actions. A delta must name its base snapshot/revision and become invalid when that base expires; offer a full refresh. Do not use screenshot hashes alone as proof that a saved state is unchanged.

### 4.2 Extraction needs a completeness contract

Extend shared extraction after the direct route is understood. Explicitly distinguish visible viewport, loaded DOM, expanded section, virtualized list and app/server inventory. “Not found” requires searched scope; “all rows” requires evidence of the end condition.

Resumable cursors should bind query, document identity, sort/order and expiry. Reject cursors after incompatible mutations instead of silently duplicating/skipping records. For virtualized lists, track item identities and progress, cap scroll attempts, and stop with partial coverage when end conditions cannot be established.

**Acceptance:** large tables, hidden sections, collapsed menus, duplicate text, infinite/virtual scroll, shadow DOM, frames, dynamic insertion between pages, cursor expiry, oversized content. Measure recall/completeness and bytes/tokens, not only runtime.

## 5. Good typing means correct editor behavior

The existing paced ASCII fixture is useful but narrow. A fixed minimum inter-key delay does not establish human-equivalent input and should not become an anti-detection claim. Optimize for correct events, editor compatibility, responsiveness and user coexistence.

### Recommended input modes

| Mode | Use | Required verification |
| --- | --- | --- |
| Fill/replace | Ordinary editable fields where app semantics support it | Editable/readonly checks, value, expected input/change behavior, selection where relevant. |
| Sequential keys | Apps that depend on keyboard events/shortcuts | Ordered keydown/input/keyup semantics, modifier release, expected final content, bounded pacing. |
| Text insertion | Unicode text where key-by-key physical composition is unnecessary | Correct Unicode/graphemes and editor mutation semantics; no claim of native IME. |
| Composition/IME | Explicit composition-dependent workflows | Composition update/commit/cancel, candidate interaction where needed, independent actual OS IME tests. |
| Rich-editor operation | Contenteditable/Docs-like specialized editing | Selection and document structure, undo grouping, persistence and reload, not just DOM innerText. |

Keep synthetic composition tests separate from native OS IME qualification. Consider grapheme clusters, surrogate pairs, combining marks, emoji, RTL text, dead keys, selection replacement, multiline text and shortcuts. Text normalization must be intentional; do not corrupt user-specified exact content.

Before every bounded batch, check target/focus/selection and expected editor revision. Observe after the batch; yield if the user moved focus or changed content. Never steal focus back repeatedly in a race with the user. On a release failure, perform only a scoped bounded cleanup and retain uncertainty (A7). Enforce readonly at admission and mutation (A5).

Make pacing a bounded route/editor setting informed by observed backpressure and validation. Compare fixed 60ms, faster bounded batches and app-specific insertion under identical tasks. Count correctness failures and disruption; reject faster strategies that lose text.

**Acceptance:** plain input/textarea, controlled web component, contenteditable, rich editor, iframe editor, readonly/disabled fields, replacement during typing, user keystroke/caret/focus interruption, lost release, composition canceled by navigation. Persist/reopen in the target app before claiming app-level typing works.

## 6. Durable jobs without fictional exactly-once effects

The journal's delivery and outcome separation is valuable. Preserve it while fixing A4. A transport acknowledgement can establish that bytes were accepted without establishing the application result. A deadline can expire after a browser effect happened. Retrying because the caller received no answer can duplicate a side effect.

### Operation record proposal

Extend existing records only as needed with principal/session, request digest, operation/step identity, target/document identity, dispatch correlation, delivery status, outcome status, evidence references, and retry policy. Distinct workflow steps need distinct claims; the same step/correlation must not acquire a new claim after acknowledgement or restart.

Classify operations for recovery:

- Pure observation: safe bounded repeat after target revalidation.
- Idempotent state setting: reconcile current state and expected version before deciding whether another write is needed.
- Non-idempotent action: unknown until independently reconciled; do not replay automatically.

Cancellation means stop admitting further work and report what is already dispatched. It cannot undo arbitrary application effects. Record partial completion, release owned resources, and do not erase uncertainty. Recovery should reconstruct identity and evidence, not just restart the worker at the previous line.

**Acceptance:** kill before claim, after claim/before send, after send/before ack, after ack/before outcome, after saved effect/before receipt, during cleanup and during reconnect. Verify no duplicate step claims, no automatic ambiguous replay, preserved evidence and stable client polling. Test both worker process and browser/native-host boundaries.

## 7. Independent verification is the dependency for everything after Phase 6

Current verifier/cache mechanics do not provide a production app-success path. Prioritize this before performance caching or learning.

### Evidence hierarchy

1. Authoritative app readback with identity/revision, where supported.
2. Saved-state observation after reopening/reloading the exact artifact.
3. Actual downloaded/exported bytes with format/content/integrity checks.
4. Independent DOM/AX observation of an outcome.
5. Calibrated visual review for appearance-only requirements.

A success toast or the same script returning its own input is weaker than independent readback. Hashing staged input bytes verifies staging, not app upload acceptance. A screenshot of an export button proves neither export completion nor file validity.

Define task-specific criteria before scoring the outcome, without seeing the execution trace when possible. Do not invent aesthetic/behavior criteria absent from the user task. Separate process restrictions (e.g. no foreground stealing) from result requirements (e.g. deck saved). Both can matter, but effort under a broken environment is not completion.

Use deterministic verifiers for identities, counts, values, revisions, file types and bytes. Use a visual judge only where semantics genuinely require it; give it selected evidence per criterion. Require `pass`, `fail` or `unknown` with reasons. Missing evidence must not become pass. Calibrate against human labels and measure false positives and false negatives, as both recent verifier/evaluation papers motivate.

Store evidence provenance: producer, target/document generation, observation time, code/build version and scope. Keep sensitive evidence local/redacted by default. The acting script cannot self-issue a trusted qualification token merely by claiming success.

**Acceptance:** incorrect save toast, stale screenshot, wrong document with same title, partial upload, corrupt export, optimistic local state lost on reload, adversarial page text claiming success, verifier disagreement. Demonstrate that production verification can pass a real correct outcome and reject/abstain on these negatives.

## 8. Guarded workflow reuse, with measured value

WALT/PAFFA make workflow reuse relevant, but the useful unit is a guarded task pattern with explicit preconditions and evidence requirements, not a saved coordinate macro.

Use existing `workflow.rs` and `cache.rs` as the starting point:

1. Compile only the supported bounded action subset.
2. Bind workflow version, app/provider capability, input schema and target/document conditions.
3. Check preconditions and invalidate on navigation, identity drift, user interference or incompatible app state.
4. Execute bounded steps with journaled identities and independent outcome checks.
5. Admit a cache entry only after qualified evidence and explicit scope.
6. Quarantine on failures/drift, count failures, and require requalification before reuse.

Observation/execution caches need different keys and lifetimes. Never reuse an authorization grant just because a workflow matches. Avoid caches containing credentials or personal page contents. Revalidation cost counts against measured speedup.

The QuickJS child process is trusted-local and default-off today. Process separation plus a timeout is not a complete untrusted-code sandbox. Before accepting arbitrary scripts, define filesystem/network/subprocess rights and enforce OS resource limits on supported platforms; test memory exhaustion, blocking calls, output flooding and child cleanup. If that is out of scope, keep the trusted-local restriction explicit and ship declarative workflows first.

**Experiment:** compare primitive actions, bounded uncached workflows and qualified reuse on matched tasks, including app drift and interrupted runs. Report setup, verification and retirement overhead. Retain reuse only where net success/cost/latency improve without increasing false success or duplicate effects.

## 9. Finish real apps as vertical slices

### Slides first: structured, inspectable output

Use `slides_deck.rs` plans as inputs to a real scoped executor; don't rewrite the planner first. Choose browser-only or an explicitly authenticated API-assisted track. For API-assisted operations use documented revision controls; preserve browser-only results as a separate category.

Minimal complete workflow: create disposable deck → add title/content and one intentional layout → inspect object/text inventory → save → reopen exact deck → verify structure/content → export → validate PDF/PPTX bytes and rendered result. Add image insertion/positioning, font overflow, alignment and multi-slide structure after that passes. Preserve document identity through every step.

### Canva second: respect design model and save semantics

Canva Design Editing APIs operate within supported app/integration contexts; they are not automatically callable from an arbitrary logged-in tab. Decide the integration route and required access before implementing. In a browser route, target UI actions through supported contracts and independently verify persistence. In an app route, respect page types, locked content, snapshot lifetime and sync semantics from the current documentation.

Minimal workflow: disposable design → text and shape/image change → sync/save confirmation → close/reopen → verify design state → export → inspect actual artifact. Add typography/layout rubrics and human review for professional quality. A visually plausible transient canvas is insufficient.

### CapCut third: acknowledge media/timeline complexity

Current `capcut_recipe.rs` validates a plan, not a timeline. Start with a tiny consented test clip and one trim/text overlay; verify timeline state, media availability, save/reopen and rendered export. Check codec/container, duration, visible overlay and audio where specified. Browser-only hidden implementation APIs should not be assumed stable or supported.

If no qualified automation route can independently verify timeline/export state, report that specific capability unavailable; continue other app gates rather than fabricating completion. Define export-time budgets separately from interactive command timeouts.

### File/artifact transport across all three

Use existing scoped handles, bounded size/type checks and byte hashes. Connect them to the actual chooser/upload route and observe upload acceptance plus persisted app association. For output, wait for real download completion, read the completed file and validate its format/content. A known path/handle must not grant arbitrary filesystem access. Test canceled upload, oversized file, duplicate filename, partial download, wrong MIME/extension and artifact expiry.

## 10. Interfaces and clients: one core, honest support

Generate guide snippets/capability tables from the same contracts used by the runtime where practical. Keep the guide compact and let clients discover details on demand. Benchmark an ergonomic CLI/skills path alongside MCP; Playwright's own guidance makes this a serious efficiency comparison.

Avoid turning the server into a large schema catalog that every model must ingest. Start with task-shaped discover/observe/act/job/artifact interfaces and measured payloads; do not force an arbitrary tool count. A single giant polymorphic tool can be harder to use than several small typed tools.

Real-client acceptance matrix should include initialization/schema negotiation, one actual browser read, one verified reversible write, a long-running job with polling, cancellation, artifacts, reconnect and a failure. Record actual server tool invocation, not model prose saying it used a tool. The existing OpenCode guide call is useful but covers only one narrow cell. Freebuff connected status did not prove Controlla calls; investigate available tools/auth/runtime separately.

Local stdio/native bridge can avoid hosting charges, but availability depends on the client supporting a local process. Hosted-only clients require a supported connection route and may impose their own costs or restrictions. Do not promise that any ChatGPT surface can load arbitrary local MCP tools or that every tunnel is free. Keep public HTTP exposure separate from local desktop setup; use official client contracts and current authentication guidance.

## 11. Evaluation that could justify a strong claim

Keep the existing 30-task pilot and frozen held-out manifests as a starting point. A task catalog is not an experiment. Before running:

1. Verify each task is executable, resettable and has an independent outcome oracle.
2. Pin model/provider/version, reasoning settings, permissions, browser version, hardware, client, server/baseline commits and task seed.
3. Use the same model/prompt budget and comparable tool affordances for component comparisons. For product-level comparisons where models differ, label that separately.
4. Include capable Playwright MCP/CLI, agent-browser and official DevTools-style semantic baselines where compatible. Do not intentionally cripple them into screenshot-only agents.
5. Record warm/cold starts, account/network conditions and tool/schema bytes. Freeze retry budgets and timeout accounting.
6. Run the pilot for debugging; lock changes before held-out evaluation. If held-out failures guide a change, that set is no longer untouched—report it and establish a new holdout for new claims.

Use matched repeated trials and appropriate confidence intervals; task-level dependence matters. Publish per-task paired outcomes and a failure taxonomy. Keep original evaluator labels and separately adjudicated human labels, with reasons and blinding where possible. Trace the first consequential error, not just the final exception. Include partial progress for diagnosis, not success inflation.

### Required ablations

| Comparison | Question |
| --- | --- |
| Full snapshots vs scoped observations vs deltas | Does lower observation cost preserve sufficient information? |
| Primitives vs bounded workflow vs qualified reuse | Is speedup from reuse real after verification and drift costs? |
| Generic input vs editor-aware mode/pacing | Which modes preserve content and reduce disruption? |
| Acting-route checks vs independent verifier | Does independent evidence reduce false success? |
| No recovery vs journaled reconciliation | Are ambiguous effects handled without duplication? |
| Shared vs dedicated/headless provider | What changes in reliability, user interference and cost? |
| MCP vs CLI over the same runtime | What interface overhead actually matters? |

Add stress tasks for navigation during pairing, unexpected overlays, user typing, renderer stall, worker/service restart, stale cursor, partial uploads and save loss. Report these separately from ordinary task completion so a few easy successes do not hide critical failures.

Do not train on evaluation traces or optimize prompt/workflow templates against held-out outcomes. Phase 11 remains optional. Only after reliable verification and consented/redacted data should imitation/reuse ranking or learning be compared to the deterministic baseline. Bad success labels amplify bad behavior.

## 12. User experience: remove the repeated setup burden

The user should see the current connection layer and target state, not a form asking for internal transport details on every session.

### Minimal dashboard / extension popup specification

- **Connection:** “Connected”, “Runtime unavailable”, “Extension update needed”, or “Selected tab not responding”, with build compatibility detail behind an expandable section.
- **Tabs:** current selected targets with human-readable page names; selection scope and navigation invalidation policy visible. No automatic selection of unrelated tabs.
- **Activity:** current bounded job, last independently verified result and whether it is still running, paused for user interference, or unknown.
- **Controls:** connect/retry read-only health check, release selected tabs, stop admitting new work; explain that stopping cannot undo completed effects.
- **Repair:** one specific action backed by the failing layer. A connection token field belongs only in advanced/manual fallback.
- **Evidence:** compact result with a link to local sanitized details and artifact, not walls of raw CDP logs.

Default to background interaction when the provider/action supports it; explicitly indicate foreground-required operations. Never claim a separate native cursor/clipboard unless the implementation and independent observation establish it. Keyboard/focus changes in shared tabs can still affect the user.

Accessibility requirements: keyboard operability, focus order, accessible control names, readable status beyond color, clear error recovery and preserved user selection during refresh. Test empty inventory, many tabs, long titles, stale selection and an unavailable service.

MagicPath can prototype these states later. Feed it this narrow state specification, not an instruction to invent a broad analytics dashboard. Implement using the existing extension UI unless a separate desktop surface has a demonstrated need.

## 13. Concrete work packages and dependencies

These packages refine the original phase plan; they do not erase its gates. Each ends with implementation, focused failing-then-passing regression, independent review and an evidence update. Priorities reflect dependencies, not promised calendar estimates.

| Package | Priority / dependencies | Primary locations | Completion evidence |
| --- | --- | --- | --- |
| W0: source/claim reconciliation | First | docs, provenance, CI | One exact candidate manifest; stale checkboxes qualified; extension/client/app/bench checks in CI; no inherited dirty diff lost. |
| W1: bridge lifecycle and target binding | First, W0 | providers, native host/setup, extension, doctor | A1–A3 regression tests; handshake/restart/hung-tab matrix; diagnostics identify correct layer; fresh native setup without manual token. |
| W2: effect journal and cancellation | First, W0 | jobs, workflow, mcp | A4 test; per-step unique claims; kill/cancel/reconcile cases preserve uncertainty and prevent duplicate admission. |
| W3: input contracts/provider parity | W1 | input, providers, mcp | A5–A7 tests; shared/direct contract matrix; readonly, replacement, focus/user interference, key-release and Unicode cases. |
| W4: bounded observation/extraction | W1 | observe, providers, mcp | Shared extraction/cursor parity for supported scopes; virtualized/hidden/frame tests; measured completeness and bytes. |
| W5: production evidence and downloads | W1/W2 | verifier, artifacts, mcp, app observers | Real correct result passes; stale/wrong/partial outcomes fail or abstain; downloaded bytes validated; calibrated false-success analysis. |
| W6: first Slides vertical slice | W3/W5 | slides_deck, apps, mcp | Actual edit/save/reopen/export with identity/revision and visual/content checks. |
| W7: safe workflow integration | W2–W5 | workflow, cache, scheduler | Shared supported operations; cancellation/resume/invalidation; cache only after qualified evidence; drift quarantine. |
| W8: Canva then CapCut | W6 lessons, W3/W5 | canva, capcut_recipe, apps | Each app independently completes its minimal persistent/exported workflow; unsupported routes explicit. |
| W9: client/interface acceptance | W1/W2/W5, parallel with app work | guide, CLI/MCP, tests/clients, HTTP | Actual client calls, verified write/job/artifact/reconnect cases; costs measured for CLI and MCP. |
| W10: evaluation | Stable W6/W7/W9 | bench | Frozen configurations, pilot then held-out paired trials, intervals, failures and ablations; no unsupported superiority claim. |
| W11: release | Qualified support subset + W10 claims | packaging, CI, release docs | Exact binary/extension compatibility, clean install on advertised platforms, rollback, publication/install evidence when authorized. |
| W12: optional learning | W5/W10 + consent | separate experiment, not core default | Untouched evaluation and reproducible gains without reliability/privacy regression; otherwise defer. |

A practical first milestone is **reliable pairing plus a persisted Slides artifact**, not closing every checkbox at once. Native Windows/Linux IME and all professional app workflows require real environments/access; document unsupported cells instead of substituting local macOS fixtures.

## 14. How to work efficiently without lowering the bar

Use small implementation tasks with stable ownership. A low-effort Luna implementer can handle a bounded regression/fix after reading the relevant callers and tests; use an independent stronger reviewer for journal/identity/security boundaries. Parallelize nonoverlapping source work and read-only audits, not competing native bridge sessions or broad edits to `mcp.rs`.

Run focused checks first, then the affected suite; reserve full browser qualification for meaningful candidate builds. Reuse evidence only when the code/contract/environment affecting it is unchanged. Batch read-only research and inspect selected primary sources; don't generate another hundred-source bibliography that never changes an implementation decision.

For each package, record: exact commit/dirty diff, environment/build identity, intended behavior, failing case, test result, independent review, remaining boundary. Reviewers must challenge success semantics and caller integration, not just style. Avoid monolithic rewrites of working transport or input code merely to reduce file length.

## 15. Ideas worth exploring only as hypotheses

1. **Adaptive observation budgets:** choose the smallest evidence set that resolves the next decision, then escalate only when uncertainty remains. Measure missed information and recovery cost, not only token savings.
2. **Reusable workflows with evidence requirements:** cache a workflow together with preconditions and required outcome observations. Test whether this reduces total cost under drift compared with unguarded reuse.
3. **Progress-aware recovery:** track task subgoals and repeated observations to stop scrolling/exploration loops. Validate that stopping does not prematurely abandon solvable tasks.
4. **Optional site-provided tools:** use experimental WebMCP when actually available and appropriate, with schema validation, policy enforcement and independent outcome observation. Page text/tool metadata cannot change user authorization.
5. **Route selection from measured qualification:** choose semantic DOM, app API, visual interaction or cached workflow based on supported operation and measured failure history. Do not introduce a learned router before a simple deterministic policy has a baseline.

None is claimed novel or superior by this document. Their value is the experiment they enable and the failure modes they avoid.

## 16. Reject these shortcuts

- Claiming all phases complete because CI is green.
- Treating a plan, transport receipt, optimistic app state or success toast as saved completion.
- Replaying a timed-out write without reconciliation.
- Re-resolving a stale selector and silently writing to a replacement node.
- Hiding unsupported native IME/platform/client cases behind a generic capability flag.
- Using arbitrary internal app endpoints or copied credentials as a substitute for a supported integration.
- Removing browser consent/indicators through bypass configuration to improve perceived setup speed.
- Optimizing command count while increasing schema tokens, retries or false success.
- Training on private traces by default or tuning against the held-out evaluation set.
- Calling the project “best” before a fair, reproducible comparison.

## 17. Definition of a credible next release

A release can be useful before every original ambition is achieved. It should expose only qualified operations, reliably diagnose setup, preserve target identity and uncertain outcomes, support at least one independently verified real-app workflow, and install cleanly on its advertised platforms. Publish the support matrix and failures alongside performance results.

The next AI should pursue the entire authorized plan, but completion language must follow evidence. The strongest route to an excellent Chrome agent is to make each advertised result trustworthy, then demonstrate that the same trust can be delivered with fewer tokens, less time and less user interruption.
