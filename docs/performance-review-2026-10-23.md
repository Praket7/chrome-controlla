# Chrome Controlla: performance, capability, and efficiency review

Reviewed October 23, 2026. Repository: [Praket7/chrome-controlla](https://github.com/Praket7/chrome-controlla). Pinned revision: `34924239529bdc6bf2cfa389d933a0bb75d828a4`.

The highest-value next step is to make the existing architecture do less repeated work per verified task. Controlla already has the important foundations: six public MCP tools, native messaging, batching, scoped semantic observations, per-target leases, workflow execution, adaptive concurrency, route learning, and explicit uncertain-outcome handling. A wholesale rewrite or a larger tool catalog would not address the clearest bottlenecks.

This is a repository-wide architectural and static review with targeted execution, not an exhaustive correctness proof or a live performance certification. I inventoried all 408 files and retrieved all 405 text files through GitHub. Their Git blob hashes match the pinned revision. Two documentation PNGs and one binary fixture were not downloaded. Deep inspection concentrated on the runtime, extension, observation, transport, scheduling, state, client, and benchmark paths. Proposals and historical reports were treated as context, not proof that a feature works today.

## What is already built

| Area | Implementation reviewed | Assessment |
|---|---|---|
| Agent interface | V3 MCP, agent UX proxy, browser-control skill, client launcher | Keep the six-tool contract. Workflow and task primitives already reduce model turns. |
| Browser transport | Native host, shared providers, extension dispatcher, session registry | Persistent native connection and batching already exist. Optimize exchanges inside operations. |
| Observation | Semantic snapshots, frame accessibility, scoped HTML, visual probes, observation router | Good bounded-output foundation; upstream collection can still be more expensive than the final response suggests. |
| Actions | Retained references, pointer/keyboard paths, widgets, rich editors, upload/download | Broad implementation; fixture support and live application acceptance are different maturity levels. |
| Coordination | Leases, cancellation, adaptive scheduler, workflow VM and DAG | Preserve mutation serialization; examine whether session-wide locks suppress independent-tab concurrency. |
| State and learning | Evidence store, checkpoints, action memory, qualified skills, cost histories | Strong provenance and drift controls; synchronous persistence and heuristic cost calibration deserve attention. |
| Quality and delivery | Rust/Node tests, CI matrix, packaging, client qualification, release gates | Extensive local infrastructure. Independent live outcomes and real-client timing remain the key gaps. |

## Ranked recommendations

### 1. Establish a trustworthy performance baseline before tuning heuristics

**Priority: P0. Confidence: high. Benefit: enables reliable optimization decisions.**

The [verification matrix](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/docs/verification-matrix.md) records a local simulated-bridge fixture:

| Task | Bridge request frames | CDP commands | Important repeated work |
|---|---:|---:|---|
| Warm click/readback | 16 | 17 | 7 `Page.getFrameTree`, 7 `Runtime.callFunctionOn` |
| Navigation plus extraction | 17 | 19 | 8 `Page.getFrameTree`, 6 `Runtime.callFunctionOn` |
| Upload acceptance | 47 | 53 | 20 `Page.getFrameTree`, 12 `Runtime.callFunctionOn` |

These are repository-recorded observations, not measurements from this review. The original frame targets of 6/12/20 were missed. The later 16/17/47 ceilings prevent regressions but do not establish desirable performance or a competitive win.

More fundamentally, [phase11-verifier.mjs](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/bench/adapters/phase11-verifier.mjs) deliberately returns `passed:false`: its fixture event log is writable by the acting browser and cannot independently prove task success. This is a correctly closed gate, but it blocks useful qualification.

Build a separate acceptance oracle that checks actual resulting state through an independently controlled fixture service or authorized app readback. An agent-emitted event must not create a passing result. Connect actual client prompt-submission and result-delivery hooks; keep missing model/token counters null. Measure cold and warm runs separately, retain failures, and use paired workloads with pinned versions. The existing runner already provides much of this structure.

**Acceptance:** a deliberately forged success event fails; a successful action with failed persistence fails; timeout and cancellation attempts remain in reports; at least 30 measured paired repetitions per chosen scenario, with warmups separate. Report p50/p95, success rate, false successes, browser commands, bytes, and caller model turns when observable. Thirty runs are a starting point, not enough to establish rare-failure reliability.

### 2. Fuse identity checks, preparation, action, and readback into bounded extension operations

**Priority: P1. Confidence: high that duplication exists; speedup unmeasured.**

[`shared_frame_identity`](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/mcp.rs#L2385) performs a CDP frame-tree request. `shared_identity_and_epoch` already batches an epoch read with identity, and the extension's [`dispatchBatch`](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/extensions/chrome-controlla/background.js#L1197) executes a bounded list. The improvement is to extend that existing mechanism across whole guarded operations, rather than add batching from scratch.

Create operation-specific commands that carry expected lease, tab, document, frame, and interaction identity. Resolve and validate the target near the browser, perform the allowed action, then return bounded verification evidence. Retain a final pre-dispatch guard and invalidate state on navigation, frame replacement, detach, topology changes, and conflicting edits. Use event-maintained identity as an optimization with authoritative reconciliation when uncertain; do not replace correctness checks with a time-to-live cache.

A batch is not a transaction: navigation or page script can interleave between CDP commands. Preserve step receipts and `not_dispatched` versus `unknown`; never automatically replay an uncertain mutation. Where supported, execution-context unique identity can further bind evaluations across process swaps. Chrome's debugger documentation explains the distinct same-process-frame and OOPIF session models.

**Acceptance:** fewer bridge exchanges and identity requests for the same verified task; unchanged wrong-target, navigation-race, cancellation, account-marker, and human-interference behavior. Start with ordinary click/readback, then navigation/extract, then upload.

### 3. Push observation budgets into collection, not just response compaction

**Priority: P1. Confidence: high. Benefit: browser CPU, transport, memory, and model context.**

[`snapshot_impl`](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/mcp/v2.rs#L1000) passes a 100,000-byte collection ceiling to the legacy snapshot route and applies the smaller public budget afterwards. Targeted responses default to 12 KiB; many task/action receipts default to 2 KiB. Those compact outputs do not by themselves bound the upstream work to the same size.

The [page collector](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/shared_page/snapshot.js) walks up to 65,536 nodes, computes presentation, text, dates, and fingerprints, and can read bounding boxes more than once for a selected element. Scoped queries and early stopping already exist.

Separate internal action-reference retention from returned evidence. Propagate a task-specific field and byte budget into the collector; avoid dates, card text, and geometry when the next decision does not need them. Cache geometry and computed style within a single collection pass. Prefer narrow role/label/scope queries before broad scans. Consider incremental observations only after defining invalidation for navigation, shadow roots, frames, focus, selection, and viewport changes.

Do not equate truncation with uniqueness or exhaustiveness. A short candidate list must still carry omitted counts and incomplete coverage. Avoid a persistent semantic cache that silently ignores DOM changes outside the observed subtree.

**Acceptance:** lower scanned-node count, collection time, and wire bytes at unchanged task success and ambiguity detection on large pages, dynamic lists, open shadow roots, and nested frames. Use browser performance traces to confirm forced layout rather than assuming every geometry read causes it.

### 4. Reduce session-wide lock hold times and replace hot polling where it matters

**Priority: P1/P2. Confidence: code-level contention opportunities; impact needs profiling.**

The runtime frequently holds a shared-session mutex across awaited browser operations, including [form execution](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/mcp/v3_full.rs#L5201). The DAG can launch independent read nodes concurrently, but shared locks may limit the benefit. The browser scheduler already has per-target gates and adaptive limits; simply increasing concurrency is unlikely to solve contention.

Separate immutable transport/lease handles from per-tab mutable snapshot state. Keep critical sections short and retain serialization for actions on the same target/document. Measure lock queue time before changing the boundaries; release/reconnect must still fence in-flight operations correctly.

The native host scans pairing requests every 75 ms and refreshes inventory every five seconds ([native_host.rs](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/native_host.rs#L328)). Contended target leases retry every 25 ms ([v3_full.rs](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/mcp/v3_full.rs#L11755)). Pairing polling is primarily an idle-efficiency and cold-open issue, not the main warm-action bottleneck.

Use in-process notifications for lease release and consider a cross-platform notification channel or filesystem watcher for pairing. Keep a slow reconciliation fallback for lost notifications and service-worker restarts. Do not remove the inventory heartbeat without preserving freshness guarantees.

**Acceptance:** reduced idle wakeups/filesystem activity and cold-pair latency; independent-tab reads overlap; same-target mutations remain serialized; cancellation cannot orphan a process lock.

### 5. Move blocking persistence off async workers, with different rules for telemetry and correctness

**Priority: P1/P2. Confidence: high on synchronous paths; latency impact unmeasured.**

[`task_checkpoints.rs`](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/task_checkpoints.rs#L63) uses synchronous file locking and filesystem operations. Several V3 call sites invoke checkpoint methods while holding an async mutex. [`workflow_dag.rs`](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/workflow_dag.rs#L56) periodically serializes and writes cost histories from synchronous methods called by async execution paths.

Use a bounded storage worker or `spawn_blocking` with owned inputs and controlled ordering. Coalesce noncritical telemetry/history updates and avoid repeatedly rewriting entire stores. Execution-cost recording and skill-trace persistence already use background work in relevant paths; preserve that rather than proposing it as missing everywhere.

Correctness-critical checkpoints are different: the write-ahead dispatch state must be acknowledged before the side effect. Do not defer that write past mutation dispatch just to lower response latency. Any durability claim should specify atomicity versus power-loss durability and test the chosen guarantee.

**Acceptance:** executor responsiveness under slow disk and cross-process lock contention, bounded storage queues, no lost checkpoint updates, and no duplicate mutation after crash/restart.

### 6. Calibrate the existing route and concurrency policies with real cohorts

**Priority: P2. Confidence: high.**

[`execution_cost_router.rs`](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/execution_cost_router.rs) already records latency distributions, failures, expiry, and sixteen clean observations before learned routing. The DAG cost history uses a 300 ms initial parallel-setup estimate and observed node histories. These are useful conservative mechanisms, but their estimates are not calibrated end-to-end client economics.

After recommendation 1, compare routes on verified completion latency, reliability, caller turns, and response size. Keep provider/site/task-size/warmth distinctions; account for build or browser changes that invalidate historical cost estimates. Use bounded exploration only for safe read-only alternatives, not speculative mutations. Reject unmeasured universal claims such as "semantic is always faster" or "more tabs is always faster."

**Acceptance:** held-out tasks improve at p95 without worse task success or false-success rate; report cohort sizes and fallback behavior, not just average latency.

### 7. Improve capability through qualification of complete workflows

**Priority: P1 for the workflows users need most; P2 for broad expansion.**

The current [blockers document](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/docs/blockers.md) distinguishes substantial local implementation from outstanding installed-Chrome, application, and client qualification. More primitives alone will not close these gaps.

Prioritize three task families: (1) cross-frame form completion with rich-editor readback, (2) download-to-upload with independently confirmed bytes and application acceptance, and (3) long tasks interrupted by navigation, reconnect, or process restart. Extend existing recipes, checkpoint contracts, and site adapters instead of creating overlapping APIs. For visual applications, prove the selected object or slide matches the captured pixels and persisted result; a successful click or screenshot call is insufficient.

Maintain separate capability levels: implemented, fixture-tested, installed-Chrome-tested, and application-qualified. Surface unsupported combinations directly to the caller so it can choose another route without repeated failed exploration. Keep scope limited to Chrome/web apps, consistent with AGENTS.md.

**Acceptance:** representative live tasks pass on the exact packaged host/extension/client build, including negative cases and restart recovery. Do not claim universal handling of closed shadow roots, opaque frames, frozen tabs, MFA, or CAPTCHA.

### 8. Treat WebMCP as an optional fast path and qualify the real API

**Priority: P2. Confidence: high on opportunity, site-dependent availability.**

[`v3_runtime.rs`](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/crates/controlla-runtime/src/v3_runtime.rs#L30) already implements `document.modelContext` discovery, schema checks, tool-change invalidation, and bounded invocation. The gap is qualification and coverage, not initial integration.

Chrome's current documentation describes WebMCP as a proposed standard with an origin trial beginning in Chrome 149 and a local testing flag. It has imperative and declarative APIs; cross-origin use depends on permissions policy. Pin a tested Chrome/API matrix and run against real registered tools, not just mocked JavaScript objects. Treat tool descriptions and read-only claims as untrusted; preserve runtime-owned operation classification and independent postcondition verification. Fall back to semantic operations when unavailable or changed.

**Acceptance:** successful real-page discovery/execution plus safe handling of missing APIs, tool changes, schema drift, frame policy restrictions, timeout, and navigation. Do not make this the only route for ordinary websites.

### 9. Split the largest dispatchers to reduce regression cost

**Priority: P2. Confidence: high on maintenance cost; no direct runtime speed claim.**

`mcp/v3_full.rs` is approximately 29,560 lines and 1.4 MB, including tests; legacy `mcp.rs` is approximately 10,521 lines. The extension background script is about 286 KB with substantial embedded page scripts. Multiple generations of routing and repeated guard logic make performance changes difficult to reason about.

Extract browser lifecycle, observation, actions, files, workflow execution, tasks, and evidence into modules without changing the public contract. Keep one owner for each identity/guard/receipt contract. Move embedded page programs into separately testable files where practical. Preserve provenance and Apache attribution when extracted code changes. Use characterization tests before changing behavior; modularization is valuable but should not delay the measured hot-path work above.

**Acceptance:** identical schemas and receipts, the same negative-control behavior, clearer call ownership, and reduced duplicated guard/serialization code. Measure compile time separately if build speed is a goal.

### 10. Make CI and releases cheaper and more reproducible

**Priority: P2. Confidence: high on workflow configuration.**

The [CI workflow](https://github.com/Praket7/chrome-controlla/blob/34924239529bdc6bf2cfa389d933a0bb75d828a4/.github/workflows/ci.yml) has a useful OS matrix, but no explicit Rust build cache. Its concurrency group includes the commit SHA and does not cancel obsolete runs. Several platform-independent checks repeat across the matrix. The headless job downloads the moving stable Chrome package.

Add a Rust cache keyed by OS/toolchain/lockfile, keep canonical JS/docs checks in one job where platform independence is established, and retain platform-specific install/launch/native-host checks. Cancel superseded PR runs while preserving release runs and required evidence. Use a pinned Chrome-for-Testing build for reproducible performance comparisons and a separate current-stable compatibility lane. A Chrome-for-Testing native host may require different registration paths on newer versions, so test packaging explicitly.

**Acceptance:** lower CI minutes and queue time with unchanged OS/install coverage; each benchmark report binds browser, binary, extension, client, and source versions.

## Recommended implementation order

| Stage | Concrete deliverable | Why this order |
|---|---|---|
| First | Independent outcome oracle and three reproducible baseline scenarios | Prevents optimizing misleading measurements. |
| Next | Guarded click/readback fusion and upstream snapshot budgets | Targets the strongest existing evidence of repeated work. |
| Then | Storage-worker boundaries, lock profiling, independent-tab concurrency | Addresses tail latency and resource efficiency without guessing. |
| Then | Installed-Chrome qualification for frame/forms, files, and resume | Turns implemented features into dependable capability. |
| Ongoing | Route calibration, module extraction, CI caching | Makes improvements sustainable and cheaper to validate. |

For every change, compare the same task and starting state with the same verification predicate. Keep wrong-target and false-success failures as hard release blockers. Do not trade correctness for an attractive median.

## External research used

Six Exa search workstreams requested 30 results, followed by direct reads of four primary documentation pages. This is a discovery count, not 30 independently validated sources. Recommendations rely on the primary sources below; promotional percentage claims were excluded.

1. [Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging): persistent `connectNative` versus process-per-message behavior; JSON framing; 1 MB host-to-Chrome and 64 MiB extension-to-host limits. Supports keeping the existing persistent bridge and reducing bounded exchanges rather than changing transport blindly.
2. [Extension service-worker lifecycle](https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle): native connections and disconnect/reconnect behavior. Supports event-driven work with reconciliation, not unguarded removal of lifecycle recovery.
3. [Chrome debugger frame/session model](https://developer.chrome.com/docs/extensions/reference/api/debugger): flat child sessions and non-recursive auto-attach; same-process frames need execution-context handling. Supports precise frame identity and OOPIF qualification.
4. [CDP Runtime](https://chromedevtools.github.io/devtools-protocol/tot/Runtime/): evaluation, context lifecycle, remote object management. Treat experimental/version-specific fields as capability-detected.
5. [Playwright actionability](https://playwright.dev/docs/actionability) and [locators](https://playwright.dev/docs/locators): bounded waiting, visibility/stability/hit-target checks, fresh target resolution, open-shadow support and closed-shadow limits. Useful behavior to test against, not evidence that replacing Controlla with Playwright is faster.
6. [Playwright MCP snapshots](https://playwright.dev/mcp/snapshots): subtree/depth scoping and optional snapshots. Supports task-specific observation budgets; does not justify comparing tools on different evidence requirements.
7. [Chrome forced reflow guidance](https://developer.chrome.com/docs/performance/insights/forced-reflow) and [web.dev layout guidance](https://web.dev/articles/avoid-large-complex-layouts-and-layout-thrashing): profile style/layout work, batch reads, avoid unnecessary read/write alternation.
8. [Chrome WebMCP](https://developer.chrome.com/docs/ai/webmcp): proposed API, origin-trial/flag availability, declarative/imperative tools and iframe permissions policy. Supports progressive enhancement with fallback.

## Verification performed and limits

- Verified all 405 retrieved text files against the Git blob hashes at the pinned revision. Coverage is recorded in `chrome-controlla-review-coverage.json`.
- Ran the four benchmark-analysis test files and V3 release-gate tests: **31 passed, 0 failed**. The log is `chrome-controlla-analysis-tests.log`.
- The broader attempted Node run was not clean: 5 of 10 file-level suites failed. One adapter hit sandbox `listen EPERM`; client child-process tests also failed. A standalone child-process stdout probe behaved unexpectedly in this environment, so those failures are retained as unresolved execution-environment/test results rather than asserted product defects.
- No Rust compiler was installed, so Cargo tests, Clippy, builds, and packaged-binary tests were not run. No live user Chrome or account-backed benchmark was performed. The GitHub workflow lookup returned no runs for this commit; this does not establish that hosted CI passed or failed.
- No source changes, commits, issues, or pull requests were published to GitHub. This report and the local source snapshot are analysis artifacts.

The report identifies concrete optimization targets and validation work. It does not claim a measured speedup, a passing full test suite, or superiority over another browser agent.
