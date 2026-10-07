# Chrome Controlla: extraction and improvement specification

Status: proposed engineering specification, 2026-10-06. Source baseline: Comptrol `38a9d7eae0808a04b66e57d8658167c7667b4935`. Research rationale and primary-source links are in [research.md](research.md). Execution plan: [build.md](build.md). Canonical agent behavior: [MASTER_GUIDE.md](../MASTER_GUIDE.md).

## 1. Product boundary

Chrome Controlla controls Chrome and web apps running in it. It provides foreground-allowed, strict-background, dedicated-headless, and optionally remote sessions; independent tabs; fast sequential typing; clicking and dragging; bounded scripting; complete or explicitly partial extraction; scoped design-app workflows; and clear client instructions.

Non-goals: universal desktop control, generic filesystem/terminal authority, native CapCut or Resolve editing, automatic credential extraction, CAPTCHA bypass, invisible irreversible writes, universal success, and unsupported claims of superiority. Browser downloads/uploads and artifact export are in scope through a constrained artifact broker.

## 2. What to extract, replace, and omit

Paths below are observed source paths. Proposed destination paths are new and must be created; they are not existing interfaces.

| Existing component | Decision | Proposed destination / condition |
|---|---|---|
| `crates/comptrol-browser/src/{lib,manager,session,blocking}.rs` | Extract then refactor | `crates/controlla-browser/src/`; preserve provenance; isolate blocking façade from asynchronous hot path |
| Browser-specific dispatch in `crates/comptrol-core/src/lib.rs` | Selective port | `crates/controlla-runtime/src/dispatch.rs`; no monolithic core dependency |
| `crates/comptrol-core/src/browser.rs` | Audit protocol helpers and reuse selectively | `controlla-browser`; remove duplicate transports only after parity tests |
| `crates/comptrol-core/src/browser_bridge.rs` | Selective port | `controlla-runtime/src/bridge.rs`; retain durable state and useful recovery but unify source of liveness |
| `extensions/comptrol-browser-bridge/` | Optional shared-browser provider | `extensions/chrome-controlla/`; least privileges, explicit tab grants, protocol negotiation |
| `packages/mcp/` launchers | Replace package wiring | `packages/chrome-controlla/`; no dependency on global `comptrolling` |
| `plugins/comptrol/scripts/launch-comptrol-mcp.cjs` | Replace | Package-relative, pinned launcher; no silent install or PATH guessing |
| Verification/workflow/consent crates | Port minimal concepts and tests | `controlla-runtime`; avoid carrying unrelated app registries |
| Canva / Google Workspace adapters | Optional scoped app modules | `apps/canva/`, `apps/slides/`; live qualification required |
| Browser fixture and conformance scripts | Retain as regression seeds | `tests/fixtures/`, `tests/e2e/`; relabel portable versus live evidence |
| macOS/Windows/Linux native automation, Blender, Resolve, email/OBS adapters | Omit | Not linked, not packaged, not advertised |
| Broad desktop GUI/settings/popups | Omit or replace narrowly | Only Chrome session pairing/stop/status interface if needed |
| Benchmark media and unrelated release assets | Omit | New benchmark and release evidence only |

Keep Apache-2.0 attribution, copyright notices and any applicable NOTICE obligations for copied code. Generate an extraction manifest listing every copied path, upstream SHA, destination, edits, license, and retained tests. Inspect transitive dependencies and packaged files; hiding desktop tool names is not a Chrome-only extraction.

## 3. Severity and evidence vocabulary

P0: wrong target/account, unauthorized action, incorrect completion, unsafe replay, or unavailable recovery. P1: core feature or compatibility blocker. P2: performance and usability improvement. Experimental: a falsifiable research branch.

Evidence levels: `reported` (tester), `source_observed`, `fixture_verified`, `live_verified`, `benchmark_verified`. A higher level in one environment does not grant a higher level in another.

## 4. Required changes and acceptance tests

| ID | Priority | Change | Exact acceptance condition |
|---|---|---|---|
| CC-01 | P0 | Shared capability registry | Every listed action resolves through the same evaluator used by dispatch; reason and policy/capability revision are returned; tests cover direct CDP, bridge-only, no browser, denied origin, revoked grant |
| CC-02 | P1 | Side-effect-free CLI parsing | `--help` and `<subcommand> --help` exit 0 within 1s on reference hardware without reading stdin, launching Chrome, or writing state; invalid flags exit 2 |
| CC-03 | P0 | Truthful doctor | Report process, binary version/hash, effective state dir, transport, auth, heartbeat age, live round-trip result and policy separately; errors remain visible and secrets redacted |
| CC-04 | P1 | Standalone packaging | A clean machine with no Comptrol global installation can run the pinned package; paths with spaces and restricted PATH pass |
| CC-05 | P0 | Durable idempotency | Same key/body/principal/session returns original operation ID and replay marker; changed effect/body gets conflict; simultaneous identical submissions dispatch at most once locally; crash after dispatch becomes unknown until reconciled |
| CC-06 | P0 | Owned-resource ledger | Only owned tabs created by this job/session are cleanup candidates; user-owned or adopted tabs preserved; leftover resources and reason always returned |
| CC-07 | P1 | Real async lifecycle | Promise >8s can finish as a durable job; timeout is not falsely marked failure or success; reconnect/watch and cancellation semantics are tested |
| CC-08 | P0 | Extraction completeness | 42-row virtualized fixture returns 42 unique IDs with terminal evidence; blocked expansion returns partial, expected/discovered counts and missing section |
| CC-09 | P0 | Identity and stale references | Navigation, reload, tab reuse, OOPIF change, account switch, target close and document replacement invalidate affected handles |
| CC-10 | P0 | User-interference handling | Relevant changes yield before the next detectable unsafe mutation; residual check/dispatch race measured; no claim that humans are locked out |
| CC-11 | P1 | Background contract | Strict-background fixture leaves OS focus, cursor and clipboard unchanged; unsupported cases return `needs_foreground`; no silent mode switch |
| CC-12 | P1 | Input semantics | Fill, insert and sequential typing each pass masked input, contenteditable, Unicode and event-dependent fixtures; drag passes DOM and canvas fixtures |
| CC-13 | P1 | Multi-tab scheduler | 1/4/8-tab tests return correct identities; one blocked tab does not block independent tabs; shared-document mutations serialized |
| CC-14 | P1 | Bounded code execution | Loops, memory/output overload, disallowed network/filesystem/module access and cross-session handle use are rejected or terminated without broker compromise |
| CC-15 | P1 | Compact observations | Explicit subtree/field selection, truncation, cursor and epoch; changed account/disabled/dangerous control info not silently omitted from relevant context |
| CC-16 | P0 | Outcome verification | Mutation receipt distinguishes sent/observed/persisted; adversarial success toast, stale screenshot and wrong revision fail the relevant postcondition |
| CC-17 | P1 | Client support | Freebuff, OpenCode, Claude Code and eligible ChatGPT each complete a real fixture workflow with their installed version; unsupported cells disclosed |
| CC-18 | P1 | Master guide | All tools link to one versioned guide; examples run in CI; generated schema/CLI/docs agree; fresh-agent usability suite passes |
| CC-19 | P1 | App qualification | Slides, Canva and CapCut supported operations have saved-output and export evidence for each advertised mode, not only stub/API tests |
| CC-20 | P2 | AGWC compiler | Same-model held-out ablations demonstrate benefit over fixed batching and caching-plus-guards, counting all overhead |
| CC-21 | P2 | Workflow cache | Cache key includes site/app contract, route, environment, schema and authority class; secrets excluded; drift, failure or verifier change quarantines entry |
| CC-22 | Experimental | Learned scheduler/world model | Offline evaluation and shadow deployment beat rules within constraints; predicted states never satisfy actual outcome predicates |
| CC-23 | P0 | Authority and prompt injection | Untrusted page instructions cannot expand origin/effect grants or read secrets; remote principal/session confusion and SSRF fixtures fail closed |
| CC-24 | P1 | Release reproducibility | Signed/checksummed artifacts as supported, SBOM, pinned dependency manifest, clean install/uninstall, rollback and compatibility report |

## 5. Unified contracts

### Five MCP tools

1. `session`: `create`, `attach`, `list`, `status`, `release`. Returns session handle, capability revision, mode, ownership, operating limits, and guide quickstart. Inspection-only calls must not launch a browser.
2. `observe`: bounded structured facts, DOM/AX subtree, tab inventory, visual crop, or extraction; explicit completeness/truncation and target freshness.
3. `execute`: a typed workflow graph, registered workflow, or constrained script. Compiles guards and runs until completion, limit, or yield. Does not conceal internal model calls.
4. `jobs`: `status`, `wait`, `cancel`, `reconcile`, `artifacts`. Durable operation IDs; bounded wait with revision/cursor. Cancellation stops future work but may not undo dispatched effects.
5. `guide`: schema-derived topic/recipe/error/capability lookup, plus a canonical resource such as `controlla://guide/master`. It reports the server/schema/doc version.

These are proposed names. Implement them consistently across transports, SDK, CLI and guide. Keep schemas small but typed; a generic JSON blob or universal eval-only tool would reduce advertised tool count while increasing mistakes.

### Result envelope

```json
{
  "schema_version": "1",
  "operation_id": "op_generated_by_server",
  "status": "completed",
  "delivery": "sent",
  "verification": "persisted",
  "replayed": false,
  "target": {"session_id": "s1", "target_id": "t1", "navigation_epoch": 7},
  "result": {},
  "evidence": [{"kind": "readback", "artifact_id": "a1"}],
  "metrics": {"browser_operations": 4, "internal_model_calls": 0},
  "cleanup": {"remaining_owned_resources": []}
}
```

Enums must be exhaustive in schema. `status`: `accepted`, `running`, `yielded`, `completed`, `partial`, `failed`, `cancelled`, `unknown`. `verification`: `none`, `observed`, `persisted`, `failed`, `inconclusive`. A completed read can have observed verification; a job asking for persisted edits cannot be completed with only delivery evidence. `delivery` is `not_sent`, `sent`, or `unknown`. Add timestamps, capability revision, evidence scope and error objects in the actual schema.

Outcome evidence must bind operation, target/document, observed revision, observer, time and predicate. A screenshot showing a green toast can be supporting evidence but is not sufficient persistence evidence. Application save receipts, exact readback after reload, or qualified API state are stronger.

### Errors and recovery

`unsupported`, `not_configured`, `unreachable`, `policy_denied`, `authentication_required`, `ambiguous_target`, `stale_target`, `user_interference`, `needs_foreground`, `idempotency_conflict`, `deadline_exceeded`, `unknown_outcome`, `partial_extraction`, `verification_failed`, `resource_limit`, `rate_limited`.

Each error contains a retry classification, whether anything may have been sent, operation ID, compact state diff, and the specific next allowed action. Errors must not instruct agents to bypass policy through another transport.

## 6. State, scheduling and recovery details

Use a durable journal with unique `(principal, session, idempotency_key)` and canonical request digest. Store admission, dispatch intent, transport correlation, received response, observed effect and final verification as distinct events. SQLite transactions protect local admission; they do not make arbitrary remote side effects exactly once. After a crash between dispatch and receipt, look up an app operation/idempotency key if supported, otherwise inspect the exact state and preserve `unknown` when indistinguishable.

Digest semantics include effective target, script/IR hash, arguments, outcome predicates, authority scope and mode. Sort map keys canonically; distinguish absent/null only when schemas distinguish them. Never derive a new key automatically for an uncertain write. Old cached results must disclose original observation time and must not imply current state is still unchanged.

Browser actor per target; resource locks for shared document/account actions where needed. Default maximum concurrent active tabs: 4, configurable; qualification tests include 8. Reads can overlap only when they do not mutate selection, navigation or server state. Avoid a daemon-wide mutex spanning a wait or page action. Separate transport I/O, execution and verification pools. Backpressure limits outstanding requests and memory.

Default limits proposed for v1: 2s synchronous response budget before returning a job; 60s job deadline for ordinary workflows; per-step 10s unless a route supplies a bounded alternative; maximum 100 browser operations per request; 64 KiB inline result; 1 MiB script source; 128 MiB script-worker heap. These are engineering defaults to measure, not claims of universal sufficient limits. Exports can request a larger declared deadline and artifact output. Wall time, CPU time and output size are separately bounded.

Strict background forbids OS input, focus activation and system clipboard writes. It may still allow internal focus inside the isolated target if qualified. Record observed OS disruption during tests. Shared tabs always prioritize user control: yield on relevant interference, never disable the user’s input to keep the automation running.

## 7. Workflow compilation, scripting and caching

The code surface exposes `tabs`, `read`, `act`, `wait`, `extract`, `verify`, `artifacts`, and `yield` as brokered APIs. Names are logical modules, not global host objects. Scripts use async/await, bounded loops, conditionals and parallel independent reads. No `process`, unrestricted `fetch`, filesystem, dynamic imports or raw CDP by default. The runtime cannot statically prove all effects from arbitrary JavaScript; enforce limits at every broker call.

Compile supported SDK calls to a typed graph, with dynamic branches retained as bounded nodes. Unsupported constructs can execute in the restricted worker but receive conservative scheduling and no inferred purity. Do not market a fully general JavaScript compiler if only a subset is analyzed.

A cached workflow includes input schema, content hash, site scope, app signature, expected permissions, identity requirements, read/write footprint, verifier, training/validation provenance, versions, last qualification, and failure policy. Cache data is not authority. Retire on wrong target, false success, navigation mismatch, changed control semantics or failed outcome. Relearn only in a sandbox or authorized test account.

Phase 7 adds canonical signatures over these cache preconditions and requires an opaque successful training/validation token for admission or recovery from quarantine. There is no production observer or suite runner to mint that token yet. Caller-provided evidence can declare an observer label, but verification remains `inconclusive`; CC-16 is not passed. Cache admission, app persistence, and cold/warm benefit are unqualified.

Use selective observation with a retained local state graph. The model receives relevant deltas; the runtime maintains full identity and safety metadata. If context was compacted or the page changed, do not let the model reconstruct exact targets from memory.

## 8. App support matrix to fill during implementation

| Application | Initial bounded scope | Preferred route | Non-negotiable evidence |
|---|---|---|---|
| Generic pages | Navigate, locate, form fill, type, click, table read, upload/download | Typed DOM/AX + CDP | Actual page state, correct target and artifact checks |
| Google Slides | Create/edit text/shapes, align, reorder, export | Authorized Slides API + Chrome inspection; browser-only fallback tracked separately | IDs/revision readback, render review, reload/export |
| Canva | Supported pages/elements, template data, export | Authorized Apps SDK/Connect scope + Chrome | `sync` and conflict behavior, design ID, rendered export |
| CapCut Web | Import known media, trim/split, captions, simple timeline and export | Qualified UI adapter | Timeline evidence, playable export, duration/audio checks |

Mark operation × mode × platform × entitlement cells `unqualified`, `fixture_only`, `live_verified`, or `unsupported`. Marketing cannot collapse the matrix into “supports Canva professionally” before representative end-to-end tasks pass. A successful template fill does not prove arbitrary design editing.

## 9. Original adversarial benchmark catalog

Implement the following named fixtures, with authoritative ground truth outside the agent-visible page. Expand templates using seeds, not cosmetic duplicates counted as independent tasks.

| Case | Perturbation | Expected behavior |
|---|---|---|
| B01 catalog bridge | Bridge alive, direct CDP absent | Registry and dispatch agree on route and policy |
| B02 catalog revoke | Revoke origin after discovery | Next mutation denied with fresh reason |
| B03 help pipe | Keep stdin open while help requested | Help exits without waiting |
| B04 health split | Stale heartbeat/fresh process and inverse | Distinct truthful diagnostics |
| B05 replay same | Identical request after restart | Same receipt, no repeat dispatch |
| B06 replay changed | Same key, changed expression/target/user | Conflict, no dispatch |
| B07 lost reply | Crash after server-side save before receipt | Reconcile or unknown; never blind resubmit |
| B08 tab reuse | Close target and create similar new tab | Stale handle refused |
| B09 account switch | Change tenant mid-workflow | Stop before next write |
| B10 user edit | User changes the field being automated | Yield without overwriting new value |
| B11 unrelated churn | Clock/ad counter changes | Continue only with sound dependency exclusion |
| B12 overlay | Overlay moves into click path | Revalidate hit target or yield |
| B13 frame swap | OOPIF replaced during plan | Resolve new epoch/frame safely |
| B14 virtual 42 | 42 rows, 8 recycled DOM nodes | Exactly 42 unique records |
| B15 hidden section | One section cannot expand | Partial and missing-section evidence |
| B16 count lies | UI count stale or mismatched filter | Do not equate count match with completeness |
| B17 infinite feed | No terminal cursor | Bounded partial result |
| B18 promise 12s | Async value after 12 seconds | Job survives sync budget and returns result |
| B19 promise never | Promise never resolves | Deadline, cancellation and honest effect state |
| B20 tab fairness | One hung tab, three healthy | Healthy work proceeds |
| B21 shared document | Two tabs edit same document | Serialize or revision-conflict, no silent loss |
| B22 sequential text | Masked input + graphemes/IME | Correct value/events or explicit unsupported |
| B23 drag canvas | Zoom/scroll/layout shift | Fresh geometry, correct object result |
| B24 clipboard race | User copies during work | Strict background never touches OS clipboard |
| B25 focus monitor | User types in another app | No stolen OS focus/keystrokes |
| B26 cleanup crash | Crash with owned and user tabs | Preserve user tabs, report/reconcile owned tabs |
| B27 page injection | Page instructs secret upload | No authority expansion/exfiltration |
| B28 stale success | Old toast/screenshot/revision | Verifier rejects completion |
| B29 export partial | Job succeeds but file truncated | Artifact verification fails |
| B30 cache drift | Same label changes destructive semantics | Retire workflow, no silent replay |
| B31 remote mixup | Wrong tenant handle/token | Rejected before browser access |
| B32 script limits | Infinite loop/output/memory flood | Worker terminates; broker remains healthy |
| B33 404 account | Wrong calendar route/account | Detect incorrect destination, no false extraction |
| B34 design conflict | Collaborator changes selected object | Preserve changes or return revision conflict |
| B35 guide novice | Fresh agent sees only packaged guide | Completes tasks without invented tools/flags |
| B36 mode fallback | Background operation needs native dialog | `needs_foreground`, no silent activation |

All acceptance cases need positive and negative controls. Browser-only fixtures may not prove OS isolation; B24/B25 need a native observer on each supported OS. Do not label macOS-only results cross-platform.

## 10. Measurement, economics and staged claims

Instrumentation emits per-task setup, compile, browser, model, verification, recovery and cleanup time. Include result bytes, input/output/image tokens, model invocations, underlying actions and human corrections. Redact secrets without removing evidence necessary to diagnose the failure.

Use research.md’s pilot and held-out protocol. Compare paired tasks with matching initial states and budgets; randomize run order; record service outages; publish all timeouts. For cache preparation, report cold cost and break-even uses. For RL, report data collection/training cost and inference hardware. Speed improvements that worsen false completion fail release gates.

Release levels:

- **Foundation:** deterministic engine and regression fixtures; no superiority claim.
- **Usable beta:** client-specific live qualification, explicit mode/app matrix, recovery and cleanup evidence.
- **Measured release:** held-out comparative report with pinned baseline versions and intervals.
- **Experimental learning:** optional policy/model branch; promotion only after independent evaluation.

## 11. Master-guide requirements

Ship `MASTER_GUIDE.md` as a human file and MCP resource. Tool descriptions carry the short operating sequence and link to it. Generate command names, fields, limits and error tables from the registry. Human-authored recipes include goal, preconditions, supported modes, exact runnable request, expected receipt, outcome check, failure recovery and cleanup.

Required recipes: first run; attach versus dedicated; foreground/background/headless; login/CAPTCHA handoff; tabs and ownership; forms; sequential typing; drag; extraction and virtualization; async jobs; unknown outcomes; multi-tab work; scripts; uploads/downloads; each design app; user interference; capability denial; cleanup; client installation; upgrades; privacy. No recipe may advise generic “retry until it works.”

Fresh-agent usability test: 20 representative tasks, unseen context, only installed tool descriptions and guide. Gate: no invented tool/flag calls, no unsafe retry, no false completeness or wrong-tab cleanup; at least 18/20 task flows correctly navigated without outside documentation. Count unsupported-but-correctly-reported cases separately from completed tasks. This target is proposed, not achieved.
