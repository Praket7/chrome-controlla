# Verification matrix

## V3 PR #2 local verification — 2026-10-09

| Check | Result | Evidence / limit |
|---|---|---|
| Rust formatting, warning-denied Clippy, workspace tests | pass | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked`. Three existing installed-Chrome tests remain ignored by default. |
| Real-Chrome headless stress | pass, macOS | `cargo test -p controlla-browser --test headless_stress --locked -- --ignored --nocapture`; eight isolated headless profiles opened, observed, and identity-bound-cleaned. This does not qualify headed-background or app persistence. |
| Extension batch and background harnesses | pass | `node extensions/chrome-controlla/test-background.cjs`; `node extensions/chrome-controlla/test-v3-batch.cjs`; JS syntax check. Batch tests cover stale guard early-stop and no later dispatch. |
| Node/client/app/benchmark/release checks | pass | Node 24.19.0 / npm 11.17.0 after `npm ci`; `check:docs`, `check:provenance`, `check:clients`, `check:apps`, `check:bench`, `check:release`, and `check:v3`. Generated benchmark rows remain ineligible as real evidence. |
| Dependency and package checks | pass, macOS arm64 | `scripts/check-dependencies.sh`, `scripts/package-build.sh`, and `scripts/package-check.sh`; packaged plugin discovery confirms the six v3 tools. No published install is claimed. |
| Hosted Linux/macOS/Windows CI and headless/background stress | pass | GitHub runs [37994625965](https://github.com/Praket7/chrome-controlla/actions/runs/37994625965) and [37994621542](https://github.com/Praket7/chrome-controlla/actions/runs/37994621542), exact head `f724637cf5c9121c6ebaa3bde420c1c133a9e417`; both passed all Ubuntu, macOS, Windows, and stress jobs. |
| Real competitor/client qualification | not run / unverified | External competitor runtimes/credentials and live client workflows were not available in this local verification. The claim gate remains blocked. |

## Source-audit repair follow-up — 2026-10-08

| Check | Result | Evidence / limit |
|---|---|---|
| Workspace tests | pass | `cargo test --workspace --locked --offline`; installed-Chrome tests are ignored by default. |
| Installed-Chrome integration | pass, 4 tests | `cargo test -p controlla-browser --locked --offline -- --ignored --nocapture`; three provider tests and one Phase 5 extraction fixture passed in isolated headless profiles. This does not exercise the shared-extension route or the user's Chrome profile. |
| Rust format and warning-denied Clippy | pass | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`. |
| Extension harness and syntax | pass | `node extensions/chrome-controlla/test-background.cjs`; `node --check extensions/chrome-controlla/background.js`; includes native/popup replacement races, delayed document lookup with authorization revocation, synchronous per-tab pair reservations across both transports, and retry after a blocked navigation detach. |
| Final Sol review | pass, scoped | Re-review closed the four reported findings (optional A1 URL, missing A6 token, drag release after move failure, malformed handshake abort) and found no new finding in those areas. Reviewer did not independently rerun commands. |
| Final Sol review of document binding | approved, source/fixture scope | Reviewer found no remaining pairing, detach, or post-`getFrame` authorization race after the reservation fixes. Reviewer did not rerun tests; the subsequent live fixture result is recorded below, and hosted CI remains open. |
| Docs, provenance, client/app/benchmark, dependencies | pass | `npm run check:docs`, `check:provenance`, `check:clients`, `check:apps`, `check:bench`, and `./scripts/check-dependencies.sh`. These are local checks. |
| Package build and consumer lifecycle | pass | `./scripts/package-build.sh` and `./scripts/package-check.sh`; macOS arm64 archive, clean-prefix install, lifecycle/rollback and user-data preservation. Not published. |
| Live shared-extension/MCP route in this follow-up | pass, disposable local fixture | On 2026-10-09 the enabled local MCP entry and loaded extension paired the exact agent-created fixture tab, completed guarded fill/type/click and mock in-memory post actions, rejected a file-input attempt, invalidated on navigation, re-paired, observed the new document, and released/closed the fixture. See the detailed live battery below; no real app write or persistence is claimed. |
| Durable document identity | implemented, fixture pass; live fixture pass | Chrome 106+ `webNavigation.getFrame` document IDs flow from discovery through native pairing. The extension compares IDs before/after attach and before every allowed command, then rechecks live authorization and stored identity after the async frame lookup. Per-tab reservations prevent simultaneous native/popup attaches; attachment waits for an in-flight detach. Pairing fails closed if discovery lacks an ID. Live fixture navigation detached the stale pair and fresh pairing observed the new document. The final synchronous check cannot make Chrome command processing atomic with navigation. |
| Hosted CI | not run for this dirty diff | The CI workflow now includes the lightweight local checks, but no hosted run has qualified these uncommitted changes. |

## Native shared-tab bridge — local gate, 2026-10-07

| Check | Result | Evidence / boundary |
|---|---|---|
| Native messaging extension protocol | fixture pass | `node extensions/chrome-controlla/test-background.cjs` covers startup/reconnect through Chrome alarms, read-only tab discovery, exact selected-tab attach, command allowlist, independent native/popup ownership, overlap refusal, and release. It does not prove installed Chrome connection. |
| Native host protocol | focused pass | `cargo test -p controlla-runtime --lib native_host::tests --locked --offline` covers bounded framing, loopback/expiry/private-file guards, and inventory before pairing. Host atomically claims each pairing file, bounds the provider upgrade, and returns to inventory after failed or closed sessions. |
| Child script worker | focused pass | `cargo test -p controlla-runtime --test script_worker --locked --offline` covers brokered reads, deadline kill, and recovery. QuickJS heap/stack limits do not impose OS RSS/CPU limits. |
| Installed extension and real Chrome | native transport connected; attach pending | A screenshot showed Chrome's native-host-forbidden error. OCR of the installed extension ID found `bhgfjpbajecihfbgaikampminappgodh`; the first host manifest had `i` after `bhgf` and was corrected to `j`. Chrome then launched the release native host with that exact extension origin; its private mode-0600 tab inventory refreshed, and the live Hotload MCP `session discover` returned `companion_extension.available=true` from a fresh inventory. The old manual WebSocket port was expired. No tab was selected, attached, or changed in this check. One native shared session can select multiple tabs; an additional simultaneous session uses the manual popup fallback. |

## Current direct-Chrome Promise and shared-route gate — 2026-10-07

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| 12-second Promise resolve/reject and external navigation | pass, direct CDP only | macOS 26 arm64; installed Chrome 154.0.8037.98 | Ignored installed-Chrome test measured both resolve and reject within 11.8–15 seconds. A second CDP connection observed a synchronous marker from the pending evaluation before navigating; the pending call settled without its delayed value, the new URL/title appeared, and the old target reference became stale. Sol approved after the dispatch barrier. This does not qualify the shared extension, live MCP job route, or process-kill recovery. |
| 12-second response and exception through native extension route | pass, bounded live route | macOS / Chrome / temporary local fixture tab | A local fixture page blocked its `innerText` getter for 12 seconds. `shared_observe` returned the expected value after 12,017 ms with frame/loader identity stable. A second getter threw after 12,014 ms; the bridge surfaced the CDP `exceptionDetails` as `CALL_FAILED` without hanging. The tab was released and closed and the local server stopped. This qualifies a delayed native-extension command response and error propagation, not an async page Promise, navigation race, durable MCP job, or process-kill boundary. |
| Shared selected-node AX | fixture pass; live pending | Rust 1.99.0; local duplex extension fixture | `shared_accessibility` scopes one CSS node to a selected tab/principal, checks frame/loader/URL before and after, enforces a whole-call deadline and output byte budget; the shared WebSocket rejects messages over 1 MiB before JSON parsing. Oversized-hello and stalled-request tests pass. No real extension AX call is claimed yet. |
| Shared click changed-state verifier | fixture pass; live pending | Rust 1.99.0; local duplex extension fixture | Click requires a separate unique DOM node whose bounded observed value changes to the exact requested postcondition. An unchanged state now fails. This proves DOM readback only, not app save/persistence. |
| Workspace checks | pass | macOS 26 arm64; Rust 1.99.0 | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked` passed after the route and transport edits: 191 tests passed, 4 installed-Chrome tests ignored by default. The new installed-Chrome Promise test was run separately and passed. |

## Phase 7 review-fix verification — 2026-10-06

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| Red-first review reproduction | failed as expected before fix | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | Forged `IndependentState` returned `passed`; target drift left the original cache entry usable. These are now regression tests. |
| Public evidence verifier | fail-closed | Rust runtime | Evidence includes operation/provenance and principal/session/target/app/account/revision/time/predicate bindings, but caller-set observer/scope is untrusted and always yields `inconclusive`; no production observer exists. |
| Observer receipt predicate fixtures | pass (2 unit tests) | Rust runtime unit tests | Opaque receipts gate field/object/state evaluation; stale, wrong target/revision/account, tampered receipts, changed field, and caller-supplied evidence are covered. Fixture-only: MCP/CDP observations are page DOM reads and caller-supplied account/document revisions, not independent app-state readback. |
| Offline independent-state regression fixtures | pass (runtime unit + `tests/phase7.rs`) | Rust runtime | Separately held fixture state rejects a persuasive fake save, stale/old revision evidence and changed publish-control semantics; known artifact validation rejects a truncated otherwise-correct byte sequence. Test-only fixture observer is not a production trust boundary; visual predicates remain inconclusive. |
| Artifact validator | local fixture pass | Rust unit library | Known-good SHA-256 passes; truncated and same-length altered bytes fail. Browser download capture is not exercised. |
| Qualification/cache mechanics | internal unit pass | Rust runtime unit tests | Canonical signature varies with site/app/schema/content/permissions/identity/footprint/verifier/versions/authority/failure policy. Only opaque successful training+validation receipts mint a token; drift, expiry and failed/inconclusive verification quarantine; restore requires a new token. Test-only receipts do not qualify a live workflow. |
| Workspace tests and Clippy | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | `cargo test --workspace`: 135 passed, 3 ignored (installed-Chrome-only); `cargo clippy --workspace --all-targets -- -D warnings`: pass. |
| Provenance and docs links | pass | Node.js 24.19.0 | `node scripts/check-provenance.mjs`: 13 destination digests and 22 retained tests; `node scripts/check-doc-links.mjs`: 13 Markdown files. |
| Live app, visual and persistence qualification | not run | none | CC-16 is not passed. No safe production observer/suite runner can issue qualification tokens; no MCP workflow integration, live persisted-state evidence, or B28–B30 release suite is claimed. |

Evidence labels: `source_observed`, `fixture_verified`, `live_verified`, `benchmark_verified`. A result is limited to the stated environment. Phase 0 does not qualify live browser behavior.

## Phase 0 commands

| Command | Result | Environment | Evidence |
|---|---|---|---|
| `/Users/pcg/.cargo/bin/cargo fmt --all -- --check` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | completed 2026-10-06 11:11 UTC; no output |
| `/Users/pcg/.cargo/bin/cargo clippy --workspace --all-targets --locked -- -D warnings` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | completed 2026-10-06 11:12 UTC; clean build; no warnings |
| `/Users/pcg/.cargo/bin/cargo test --workspace --locked` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | completed 2026-10-06 11:12 UTC; 22 unit tests passed, 0 failed; 0 doc tests |
| `./scripts/check-dependencies.sh` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | completed 2026-10-06 11:12 UTC; dependency tree excludes banned Comptrol platform/core crates |
| `npm run check:docs` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Node 24.19.0, npm 11.17.0 | completed 2026-10-06 11:22 UTC; checked local Markdown links in 11 files |
| `npm ls --package-lock-only --depth=0` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Node 24.19.0, npm 11.17.0 | completed 2026-10-06 11:22 UTC; Playwright 1.63.0 matches manifest and lockfile |
| `npm ci --ignore-scripts` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Node 24.19.0, npm 11.17.0 | completed 2026-10-06 11:22 UTC; added 2 packages; audit found 0 vulnerabilities |
| `./scripts/package-check.sh` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0; pre-commit dirty tree | completed 2026-10-06 11:22 UTC; package list includes LICENSE/NOTICE and excludes banned paths |
| `./scripts/check-dependencies.sh` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | completed 2026-10-06 11:22 UTC; no banned Comptrol dependencies |
| `git check-ignore node_modules/playwright/package.json` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64 | completed 2026-10-06 11:22 UTC; dependency path ignored |
| `git diff --check` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64 | completed 2026-10-06 11:22 UTC; no whitespace errors |

## Requirement status

Phase 0 establishes the repository only. CC/B statuses below reflect evidence from their owning phases. Corrective docs/package/dependency checks pass on pinned Node 24.19.0/npm 11.17.0; Sol approved the corrective review and final CI npm pin. Hosted CI run [37456380443](https://github.com/Praket7/chrome-controlla/actions/runs/37456380443) passed on macOS, Linux, and Windows for commit `10f1aa7`.

| ID | Status | Owning phase | Evidence |
|---|---|---:|---|
| CC-01 | fixture_verified | 1 | `controlla-runtime` evaluator parity tests; no live routes |
| CC-02 | fixture_verified | 1 | open-stdin help/no-state and invalid-flag CLI tests |
| CC-03 | partial | 1 | doctor schema, local PID and loopback fixture probes; no live daemon or service |
| CC-04 | fixture_verified | 1 | packed darwin/arm64 npm archive clean-prefix install, spaces and empty PATH; Ubuntu/macOS/Windows hosted package checks pass |
| CC-05 | fixture_verified | 3 | SQLite journal enforces principal/session/key uniqueness, canonical request conflict, concurrent admission and one-time dispatch claim; independent fixture endpoint count stays one across replay/lost reply; unknown stays terminal |
| CC-06 | unimplemented | 3 | none |
| CC-07 | partial | 3/6 | Durable workflow jobs and `workflow_status` use mocked CDP; a never-settling QuickJS Promise now returns on its deadline. No live extension job dispatch, reconnecting external MCP client, or cancellation tool |
| CC-08 | partial | 5 | synthetic Chrome 42-record extraction and policy completeness fixtures pass; representative application lists and terminal semantics remain unqualified |
| CC-09 | partial | 2/4 | target/frame/browser revisions and stale-handle fixtures pass; Phase 4 guard compares caller-supplied identity/dependency snapshots; no authoritative app identity observer |
| CC-10 | partial | 4 | fill checks the live value in the same page evaluation that writes; insert and every sequential key event refresh value/focus before each CDP send; navigation race fixture withholds the fill. CDP/page handlers/server effects remain non-atomic |
| CC-11 | partial | 4 | strict-background mouse input returns `NeedsForeground`; read-only macOS observer confirms unchanged frontmost app, cursor, and pasteboard change count for one isolated Chrome text fixture; other OSes/modes and repeated interference remain unqualified |
| CC-12 | partial | 4/8 | installed Chrome verifies Unicode text/caret, guarded IME composition, fail-closed marked controls, overlay refusal, and DOM drag result; shared-input fixtures verify guarded fill/click plus nonempty ASCII key dispatch bound to one retained DOM object, replacement refusal, bounded key-up retry, and key-event failure semantics. Live shared typing, OS IME UI, app-specific semantics, and canvas movement remain unqualified |
| CC-13 | fixture_verified | 2 | 1/4/8 target scheduling, blocked-target fairness, and shared-document mutation serialization fixtures |
| CC-14 | partial | 6 | Bounded workflow graph, QuickJS source/heap/stack/interrupt/output limits, per-call read authorization, durable checkpoints and restart recovery are fixture-verified; OS isolation, arbitrary mutations, cancellation API, file-backed/download artifacts and live/browser/client qualification remain open |
| CC-15 | partial | 5 | MCP observe/extract plus bounded selected-node AX, PNG crop, resumable extraction, and synthetic Chrome hidden-section fixtures pass; broad app qualification and external client acceptance remain open |
| CC-16 | partial | 7 | Evidence is bound to operation/target/account/revision/time/predicate; caller-controlled evidence always returns inconclusive because no runtime-controlled independent observer exists |
| CC-17 | partial | 9 | `docs/clients.md` records dated local config shapes; no external client or live workflow acceptance |
| CC-18 | partial | 9 | Versioned guide resources, runtime-derived bootstrap names, every pinned rmcp initialize revision over stdio, and latest discovery lifecycle are covered. Fresh-agent B35 guide/schema evaluation passed 20/20 tasks. Other transports, generated descriptive facts, and real-client acceptance remain open |
| CC-19 | unimplemented | 8 | No app-specific edit/execute/verify routes or live app evidence. Per-app gates are recorded below; current generic browser tools do not qualify Slides, Canva, or CapCut workflows. |
| CC-20 | partial | 10 | Validated 30-task pilot and disjoint 100-task held-out manifests, baseline lock, and analysis fixtures exist; no pilot task or held-out comparison run has executed |
| CC-21 | partial | 7 | Local fail-closed verifier/cache mechanics and cache-signature/quarantine tests exist; no production observer, qualification suite or execution integration |
| CC-22 | unimplemented | 11 | Optional learning deferred: no controlled benchmark or consented trace result justifies added runtime cost; deterministic rules remain the active policy |
| CC-23 | partial | 6 | `workflow` MCP admits durable observe/wait/checkpoint/script jobs with idempotency, target/revision-bound receipts, persistent checkpoints, operation counts, `workflow_status`, and 12 KiB max inline artifacts for trusted-local scripts; arbitrary mutations, file-backed/download artifacts, session rehydration, and cross-client live recovery remain open |
| CC-24 | partial | 12 | macOS arm64 distinct-binary package upgrade/rollback passes with installed executable SHA-256 checks and user-data preservation. No live client/server compatibility, signed artifacts, Windows/Linux consumer installs, or GitHub install is verified |
| B01 | fixture_verified | 1 | direct-only/bridge-only evaluator fixture, catalog and dispatch decisions match |
| B02 | fixture_verified | 1 | updated policy revision with revoked grant is denied on reevaluation |
| B03 | fixture_verified | 1 | help exits with stdin held open; state path remains absent |
| B04 | partial | 1 | stale/fresh heartbeat is separated from local PID and authenticated loopback mock-service probes; no production service is available |
| B05 | fixture_verified | 3 | same body after reopen returns original operation ID; replay dispatch gate refuses another send and independent fixture endpoint count stays one |
| B06 | fixture_verified | 3 | changed request under same scoped key conflicts; principal scope is isolated |
| B07 | fixture_verified | 3 | independent fixture endpoint effect remains one after lost reply; replay traverses dispatch gate and is refused; recovery preserves acknowledged delivery as sent, while claim-before-send crash remains unknown with zero effects |
| B08 | fixture_verified | 2 | closed/reused target references rejected; fresh target identity required |
| B09 | partial | 4 | caller account revision mismatch yields before fixture dispatch; no live account switch observer |
| B10 | partial | 4 | account/dependency snapshot mismatch yields before CDP call; race fixture withholds next write after navigation event; no authoritative in-page user-edit observer |
| B11 | partial | 4 | unchanged dependency snapshot permits continuation; no semantic DOM churn observer or sound unrelated-change exclusion evidence |
| B12 | partial | 4 | installed Chrome overlay covers the requested click point; fresh hit testing returns stale and the covered button click handler remains untouched |
| B13 | fixture_verified | 2 | frame navigation/revision and OOPIF replacement fixtures |
| B14 | partial | 5 | installed Chrome synthetic virtualized fixture extracts 42 IDs across recycled batches; policy/dedup test passes; one run only, not broad app qualification |
| B15 | fixture_verified | 5 | isolated Chrome synthetic page verifies declared hidden expansion, blocked/disabled refusal, and partial coverage; representative app-specific sections remain unqualified |
| B16 | partial | 5 | stale-count fixture requires partial; count is metadata and never sufficient for complete |
| B17 | partial | 5 | infinite-feed fixture without explicit terminal marker requires partial; bounded traversal implemented, not live-qualified |
| B18 | partial | 3 | local async job finishes after short caller wait; exact 12-second Promise/CDP/bridge path is absent |
| B19 | partial | 3 | persisted deadline survives reopen; queued expiry is failed/deadline_error/not_sent; dispatched expiry is unknown while preserving acknowledged sent or unacknowledged unknown delivery; real Promise and worker cancellation route remain absent |
| B20 | fixture_verified | 2 | a blocked target does not prevent independent target scheduling |
| B21 | fixture_verified | 2 | concurrent mutations for the same shared document serialize |
| B22 | partial | 4 | Chrome verifies Unicode text/caret and simulated IME composition; marked masked/trusted-event-dependent controls and contenteditable fail closed; OS IME and unmarked app semantics remain unqualified |
| B23 | partial | 4 | installed Chrome DOM drag verifies final bounds; canvas route refuses without app-specific verifier, so canvas outcome remains unqualified |
| B24 | partial | 4 | read-only macOS observer confirms unchanged pasteboard change count in one isolated strict-background text fixture; MCP artifact registration/file selection has fixture coverage, while other platforms, repeated races, and app acceptance remain open |
| B25 | partial | 4 | strict-background text uses CDP and one macOS snapshot confirms unchanged frontmost app/cursor; mouse returns `NeedsForeground`; other OSes/modes and repeated disruption remain open |
| B26 | fixture_verified | 2 | crash reconciliation reports owned leftovers and preserves adopted/user tabs |
| B27 | partial | 6 | Trusted-local scripts are opt-in; worker exposes only the read-only observe broker and no filesystem/network/process globals. Page-prompt injection behavior remains unqualified. |
| B28 | partial | 7 | Persuasive page claims and stale/unbound evidence remain inconclusive; runtime-controlled observer and app-save proof are absent |
| B29 | partial | 7 | Artifact length/hash helper rejects truncated or corrupt bytes; no browser download/export capture route is integrated |
| B30 | partial | 7 | Cache signature drift and non-passing verification quarantine fixture pass; no live control-semantics observer or workflow-execution integration |
| B31 | fixture_verified | 6 | MCP workflow rejects target references whose session differs, and compiler unit checks reject cross-principal/session bindings before execution |
| B32 | partial | 6 | Workflow step count, per-step/aggregate waits, observe limits and aggregate declared bytes are rejected above bounds; infinite-loop, unresolved-Promise deadline, heap, stack and output limits have worker tests. In-process worker isolation remains unqualified. |
| B33 | partial | 5 | fixture policy classifies wrong-account 404 unknown; mocked CDP preflight returns unknown and sends no scroll command; real 404/account UI remains unqualified |
| B34 | unimplemented | 8 | none |
| B35 | fixture_verified | 9 | Four blind fresh-agent groups × 5 tasks = 20/20 pass against the rebuilt package and guide v3; per-group scenarios/evidence: [routes](review/phase9-b35-routes.md), [apps](review/phase9-b35-apps.md), [long tasks](review/phase9-b35-long-tasks.md), [extraction/file selection](review/phase9-b35-extract-file.md). No browser, app, or external client connection |
| B36 | partial | 4 | real Chrome strict-background text succeeds with no native requirement; strict-background mouse/native routes return `NeedsForeground` in fixtures; no native-dialog fixture or platform observer |

### Phase 8 app qualification matrix — 2026-10-06

| Application | Browser-only route | API/SDK route | Current usable MCP surface | Status |
|---|---|---|---|---|
| Google Slides | Generic guarded shared fill/type/click only; no live app route | Offline 10-slide candidate uses caller-asserted inventory, explicitly non-authoritative, and emits no deletes; no token injection/API dispatch | Generic browser tools plus `app_capabilities`, `slides_plan_text_edit`, and `slides_deck_plan` | `unqualified` |
| Canva | Generic guarded shared fill/type/click only; no live app route | Offline five-page plan uses unverified caller assertions; sync candidate is advisory; Apps SDK connection absent | Generic browser tools plus `app_capabilities`, `canva_sync_preflight`, and `canva_design_plan` | `unqualified` |
| CapCut Web | No qualified controls or verifier | Offline asset/timing recipe validation only; no official general timeline API route established | Generic browser tools plus `app_capabilities`, unsupported `capcut_web_plan`, and `capcut_recipe_plan` | `unqualified` |

These rows describe available code paths, not app validation. Planning/preflight tools do not dispatch. `shared_input` is fixture-verified only. No live documents, accounts, or media were used. A file-selection result only establishes browser selection; it does not show app acceptance, saved persistence, or export quality.

The offline Slides, Canva, and CapCut planning tools each passed focused unit tests and the local stdio roundtrip. CapCut now rejects end cards shorter than one second or with text contrast below 4.5:1; Slides/Canva planner principal labels are server-owned and forged request fields are covered by the stdio regression. See [Phase 8 offline planner evidence](review/phase8-offline-planners.md). This adds deterministic preparation only; it does not change any app's `unqualified` status or close the Phase 8 execution, persistence, export, and quality gates.

## Phase 0 review 1 corrective rerun — 2026-10-06 11:25–11:26 UTC

| Command | Result | Environment | Evidence |
|---|---|---|---|
| `/Users/pcg/.cargo/bin/cargo fmt --all -- --check && /Users/pcg/.cargo/bin/cargo clippy --workspace --all-targets --locked -- -D warnings && /Users/pcg/.cargo/bin/cargo test --workspace --locked` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 22 unit tests passed; 0 failed; 0 doc tests; `docs/review/phase0-review-1.md` |
| `PATH=/Users/pcg/.nvm/versions/node/v24.19.0/bin:$PATH node --version && ... npm --version && ... npm ci --ignore-scripts && ... npm run check:docs && ... npm ls --package-lock-only --depth=0` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Node 24.19.0, npm 11.17.0 | versions matched pins; 2 packages installed; 0 vulnerabilities; 11 Markdown files checked; Playwright 1.63.0 locked; `docs/review/phase0-review-1.md` |
| `npm run check:provenance` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Node 24.19.0 | 13 destination digests and 22 retained tests verified; optional upstream source hashes checked because source checkout is present; `docs/review/phase0-review-1.md` |
| `./scripts/check-dependencies.sh && ./scripts/package-check.sh && git diff --check && git diff --cached --check` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | package list has LICENSE/NOTICE and four browser source files; banned dependency paths absent; no whitespace errors; `docs/review/phase0-review-1.md` |

The first package-list attempt before adding `--allow-dirty` exited 101 because Cargo refuses packaging an uncommitted tree. The corrected check passes without requiring a commit and keeps that earlier failure visible. CI runs both scripts on all configured runners, including Windows via Bash. Hosted CI run [37456380443](https://github.com/Praket7/chrome-controlla/actions/runs/37456380443) passed on all three configured operating systems for Phase 0 commit `10f1aa7`.

## Phase 1 local acceptance

Evidence is fixture/local only. Capability route values are test inputs, not observed browser connections. No daemon, MCP transport, authenticated principal, browser effect, or live client is implemented in this phase.

| Command | Result | Environment | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 35 tests passed (22 browser, 2 runtime unit, 8 CLI/doctor, 3 registry); no Clippy warnings; formatting clean |
| `./scripts/check-dependencies.sh && ./scripts/package-build.sh && ./scripts/package-check.sh` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0, Node 26.7.0/npm 11.19.0 | no prohibited dependencies; Cargo runtime package contains 14 files including LICENSE/NOTICE; npm archive is labeled darwin/arm64, includes LICENSE/NOTICE, installs in a clean prefix with spaces, and runs with empty PATH through its package-local binary |
| `npm run check:docs && npm run check:provenance && git diff --check` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Node 26.7.0/npm 11.19.0 | 11 Markdown files checked; 13 provenance digests and 22 retained tests verified; whitespace clean |
| `cargo test -p controlla-runtime --locked --offline` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | doctor distinguishes invalid config, live local PID, dead PID and loopback fixture health; open-stdin help is bounded at 1s; Windows unsupported PID probe expects `Unknown` |
| Red-first test evidence | not recorded | Phase 1 implementation history | No pre-implementation failing run is claimed; the current tests verify the resulting behaviors only |

The npm artifact built locally is specifically for darwin/arm64. The generated package metadata constrains OS and CPU so npm rejects installation on unsupported hosts. Doctor transport/authentication uses loopback fixture services; no live daemon, browser, client, or production dispatch handler exists. Sol approved the Phase 1 local/fixture gate on 2026-10-06. After the historical failures recorded below, hosted CI run [37465119376](https://github.com/Praket7/chrome-controlla/actions/runs/37465119376) passed the complete configured matrix on Ubuntu, macOS, and Windows for commit `8dbf51d`.

## Phase 1 hosted CI run 37465119376 — passed

| Runner | Result | Evidence |
|---|---|---|
| Ubuntu | pass | Rust formatting, Clippy, tests, package build, docs, provenance, dependency, npm archive and clean-prefix install checks passed. |
| macOS | pass | Rust formatting, Clippy, tests, package build, docs, provenance, dependency, npm archive and clean-prefix install checks passed. |
| Windows | pass | Rust formatting, Clippy, tests, package build, docs, provenance, dependency, `.exe` archive selection, npm archive and clean-prefix install checks passed. |

Workflow: [37465119376](https://github.com/Praket7/chrome-controlla/actions/runs/37465119376), commit `8dbf51dd5755932361d5bbbb3e56ca99862e76f3`.

## Phase 1 hosted CI run 37460467645 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37460467645](https://github.com/Praket7/chrome-controlla/actions/runs/37460467645); package/runtime jobs passed. |
| Windows | fail | Clippy reported `unused_imports` for Unix-only `Command` and `Stdio` imports in `doctor.rs`; imports are now gated with `#[cfg(unix)]`. |
| Ubuntu | fail | `doctor_reports_fresh_heartbeat_but_dead_process_and_unreachable_loopback` treated PID `4294967295` as alive because it overflowed signed `pid_t`; process probing now rejects values above `i32::MAX` before invoking `/bin/kill`, with a unit test proving the probe is not called. |

The run remains recorded as failed historical evidence. Its fixes were subsequently verified by the passing three-platform run 37465119376.

## Phase 1 review-fix local rerun — 2026-10-06

| Command | Result | Environment | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64 | formatting and Clippy clean; 36 tests passed (22 browser, 3 runtime unit, 8 CLI/doctor, 3 registry); no doc tests. Includes oversized PID rejection before the OS probe. |
| `./scripts/package-build.sh && ./scripts/package-check.sh && git diff --check` | pass (exit 0) | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0, Node 26.7.0/npm 11.19.0 | Cargo archive packaged 14 files; npm archive verified as darwin/arm64 with attribution and clean-prefix install; whitespace clean. |

## Phase 1 hosted CI run 37461256280 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37461256280](https://github.com/Praket7/chrome-controlla/actions/runs/37461256280). |
| Ubuntu | pass | Hosted workflow run [37461256280](https://github.com/Praket7/chrome-controlla/actions/runs/37461256280); includes the oversized-PID regression fix. |
| Windows | fail | `doctor_reports_stale_heartbeat_separately_from_live_process_and_authenticated_health` expected a Healthy process probe, although the implementation correctly reports `Unknown` on unsupported OSes. The assertion now expects Healthy on Unix and Unknown elsewhere. |

This failed run remains historical evidence. The Unix branch passes locally; the Windows `Unknown` expectation was subsequently verified by the passing three-platform run 37465119376.

## Phase 1 hosted CI run 37462361648 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37462361648](https://github.com/Praket7/chrome-controlla/actions/runs/37462361648); full CI passed. |
| Ubuntu | pass | Hosted workflow run [37462361648](https://github.com/Praket7/chrome-controlla/actions/runs/37462361648); full CI passed. |
| Windows | fail | `./scripts/package-check.sh` exited 1 after Cargo package verification. The hosted log did not expose the silent failing subcommand. Package check now emits stage labels and checks the installed command through `npm exec`, which selects the platform-appropriate shim under Git Bash. The Windows package path was subsequently verified by run 37465119376. |

After the failure, package-check stage labels localized the post-Cargo path, and the installed consumer command check was changed to `npm exec`. `./scripts/package-check.sh` passes locally on macOS, including archive contents, target metadata, attribution, clean-prefix installation, direct package-local launcher, and npm command execution.

## Phase 1 hosted CI run 37463463416 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37463463416](https://github.com/Praket7/chrome-controlla/actions/runs/37463463416). |
| Ubuntu | pass | Hosted workflow run [37463463416](https://github.com/Praket7/chrome-controlla/actions/runs/37463463416). |
| Windows | fail | `package-check.sh` expected the Unix name `package/bin/controlla-core`, while the archive contains `package/bin/controlla-core.exe`. Commit `8dbf51d` selects the expected entry from `process.platform`; the corrected check passed in run 37465119376. |

After the failure, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `./scripts/package-build.sh`, `./scripts/package-check.sh`, and `git diff --check` all passed locally on macOS 26 / Darwin 25.6 arm64. All 36 workspace tests passed. The package check logged each archive entry, validated darwin/arm64 metadata and attribution, installed into a clean prefix, and launched both the package-local binary and npm command shim. Run 37464310787 passed macOS/Ubuntu but failed on Windows extension selection; the process-platform branch correction passed on all three runners in run 37465119376.

## Phase 2 local acceptance — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | clean formatting |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | no warnings |
| `cargo test --workspace --locked --offline --quiet` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 85 passed, 1 ignored, 0 failed |
| `cargo test -p controlla-browser real_chrome_headless_provider_launch_and_runtime_smoke --locked --offline -- --ignored --nocapture` | pass | macOS 26 / Google Chrome 154.0.8037.98, isolated temporary profile | CDP page count 1; `Runtime.evaluate` returned visible; the fixture observer authorized test-owned target close; `Browser.close` completed, child exited, and profile was absent after teardown |
| `npm run check:provenance` | pass | Node 24.19.0, npm 11.17.0 | 13 destination digests and 22 retained tests verified |
| `npm run check:docs` | pass | Node 24.19.0, npm 11.17.0 | 12 Markdown files checked |
| `./scripts/package-check.sh` | pass | macOS 26 / Darwin 25.6 arm64 | host-bound npm archive, attribution, and clean-prefix install verified |
| `node --check extensions/chrome-controlla/background.js && node --check extensions/chrome-controlla/popup.js && node extensions/chrome-controlla/test-background.cjs && git diff --check` | pass | Node 24.19.0 | extension syntax, pairing-generation cleanup fixture, and whitespace checks passed |

Sol independently approved the Phase 2 code diff after reviewing launch cancellation, browser-binding rollback, retryable shared pairing, target cleanup, headless target inventory, headed-window preservation, and cleanup retry state. Hosted CI is recorded with the Phase 2 commit below. This is code/fixture evidence: the smoke test's observer is test-only. Live MV3 attachment/debugger consent, headed desktop behavior, app-level identity observation, and native focus/cursor/clipboard observation remain unqualified. The headless empty-target check is a snapshot; it cannot rule out a page opened immediately before `Browser.close`.

## Phase 1 hosted CI run 37461823256 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37461823256](https://github.com/Praket7/chrome-controlla/actions/runs/37461823256); all Rust, package and provenance checks passed. |
| Ubuntu | pass | Hosted workflow run [37461823256](https://github.com/Praket7/chrome-controlla/actions/runs/37461823256); all Rust, package and provenance checks passed. |
| Windows | fail | `check-provenance.mjs` hashed CRLF working-tree files against LF source digests. Added `.gitattributes` with `* text=auto eol=lf`; the correction passed Windows provenance checking in run 37465119376. |

## Phase 2 hosted CI — initial failures and final pass

| Run | Runner | Result | Evidence |
|---|---|---|---|
| [37510070079](https://github.com/Praket7/chrome-controlla/actions/runs/37510070079) | macOS | fail | Provenance check caught a stale digest after the Phase 2 build-plan status edit. Updated the recorded destination hash. |
| [37510070079](https://github.com/Praket7/chrome-controlla/actions/runs/37510070079) | Ubuntu | fail | Scheduler concurrency fixture observed peak 3 instead of 4 because response timing varied. The fixture now holds responses until a full expected request wave arrives. |
| [37510715198](https://github.com/Praket7/chrome-controlla/actions/runs/37510715198) | Windows | fail | Clippy found an unused `path` parameter in the non-Unix no-op permission helper; renamed it `_path` while retaining Unix use. Ubuntu and macOS passed this run. |
| [37511048769](https://github.com/Praket7/chrome-controlla/actions/runs/37511048769) | Ubuntu, macOS, Windows | pass | All configured Rust, packaging, documentation, dependency, and provenance checks passed for final commit `928c3a8`. |

## Phase 3 local journal gate — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test --workspace --locked --offline --quiet` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 95 passed, 1 ignored, 0 failed; includes ten journal tests |
| `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | pass | macOS 26 / Darwin 25.6 arm64 | formatting and warning-free Clippy |
| `node scripts/check-provenance.mjs`, `node scripts/check-doc-links.mjs`, `git diff --check` | pass | macOS 26 | 13 source digests and 22 retained tests; local Markdown links pass; no whitespace errors |

Sol approved the local journal gate after reviewing two correction rounds. Tests cover owner-scoped durable replay, concurrent admission and one-time dispatch claims, fixture send counting, crash-before-send uncertainty, acknowledged-delivery preservation, pre-send failure, bounded wait, and persisted deadlines. This is not an MCP jobs tool or live browser execution: B18/B19 remain partial for real Promise, worker cancellation, and CDP/extension routes.

## Phase 3 restart-before-dispatch correction — 2026-10-07

The new red-first `startup_recovery_keeps_running_job_not_sent_when_dispatch_was_never_claimed` regression failed against the prior behavior (`running/unknown`), then passed after recovery was keyed to persisted delivery state. The focused `jobs` integration suite passes 18/18: real subprocess kills before dispatch recover as `failed/not_sent`, and after a dispatch claim recover as `unknown` and reject replay. Additional coverage preserves acknowledged `sent` delivery, leaves completed operations untouched, and expires running/not-sent work as failed. This verifies journal boundaries only; no live Chrome dispatch process was stopped.

## Phase 3 expanded subprocess crash matrix — 2026-10-07

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test -p controlla-runtime --test jobs --locked --offline` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 17 passed, 0 failed. Child processes are killed and the journal reopened at seven durable fixture cut points: admitted, running/not-sent, claim-before-effect, effect-before-ack, acknowledged, checkpointed, and completed. Each recovery state is asserted and replay remains refused. |
| Sol focused review of `jobs.rs` | pass | 2026-10-07 | Ready marker uses sync plus same-directory atomic rename; claim-before-effect pauses before writing the fixture effect. Review confirms fixture journal coverage only, not live browser dispatch or every runtime boundary. |

## Phase 4 local guard primitives — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test -p controlla-browser --locked --offline input::tests` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 4 typed snapshot/action-callback tests pass, including caller-revision mismatch, strict-background fail-closed behavior, Unicode fixture value echo, and invalidation; no browser DOM dispatch |
| `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test --workspace --locked --offline --quiet` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 68 passed, 1 ignored in browser unit tests; 7 phase2 integration, 3 runtime unit, 8 CLI, 10 jobs, 3 registry integration tests passed; formatting and Clippy clean |
| `npm run check:provenance && node scripts/check-doc-links.mjs && git diff --check` | pass | macOS 26 | 13 extraction digests and 22 retained tests verified; 14 Markdown files checked; no whitespace errors |
| Red-first evidence | pass | installed Chrome fixture | Initial smoke failed with `input fixture navigation did not create a frame` and an empty frame graph because targets created after connection bootstrap had not enabled Page/Runtime. After provider launch bootstrapped the attached target session, the same real Chrome fixture passed with DOM value/caret readback. Earlier callback-only tests still have no pre-implementation RED run. |

This entry records the initial guard-only commit and is superseded by the Phase 4 continuation entry below. Native focus/cursor/clipboard isolation remains unverified.


## Phase 4 CDP input continuation — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test -p controlla-browser --locked --offline guarded_text_actions_use_revision_bound_cdp_and_race_yields -- --nocapture` | pass | macOS 26 / Darwin 25.6 arm64 | websocket fixture exercised fill with same-evaluation value guard, insert and sequential event checks, click/drag sequences and predicates, and navigation race withholding. This follow-up run measured 716 us in the mock; this is not a Chrome processing bound. Separate earlier installed Chrome 154.0.8037.98 fixture read back `héllo 👋λa`, caret 10. The focused mock does not prove page handlers or server effects atomic. |
| `cargo test -p controlla-browser --locked --offline input::tests` | pass | macOS 26 / Darwin 25.6 arm64 | 5 focused guard tests: navigation/account/document/dependency invalidation, strict-background native requirement, zero/multiple locator failure, external invalidation, and UTF-16 selection replacement behavior |
| `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test --workspace --locked --offline --quiet` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | formatting and warning-free Clippy; full workspace test counts recorded in continuation report |

Text dispatch supports ordinary input and textarea controls only; sequential typing is printable ASCII only. IME and masked/contenteditable/event-dependent inputs remain unqualified. Click/drag require explicit predicates; drag verifies final DOM bounding-box coordinates only. Canvas object movement is unqualified. Check and remote effect are still non-atomic: navigation is observed in the fixture before the mutation request, but a page-side event or remote state change can happen after the last local reference check and before/while Chrome processes the request. Native OS focus/cursor/clipboard isolation is not claimed.

| `cargo test -p controlla-browser --locked --offline real_chrome_headless_provider_launch_and_runtime_smoke -- --ignored --nocapture` | pass | macOS 26 / Darwin 25.6 arm64, installed Google Chrome 154.0.8037.98 | Isolated headless Chrome opened a local `data:` input fixture; guarded fill, insert, and sequential ASCII key dispatch all reported verified readback. Final DOM value `héllo 👋λa`; `selectionStart=10` UTF-16 code units. The ignored test disables preserve-on-drop for failure cleanup and shuts down the isolated profile explicitly on success. This covers ordinary `<input type=text>` only. |

## Phase 4 real Chrome controls follow-up — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test -p controlla-browser --locked --offline real_chrome_headless_provider_launch_and_runtime_smoke -- --ignored --nocapture` | pass | macOS 26 / Darwin 25.6 arm64, installed Google Chrome 154.0.8037.98 | Same test-owned isolated profile verifies ordinary Unicode fill/insert/sequential keys; stale expected value returns before mutation; explicitly marked masked tel and trusted-event-dependent inputs return unsupported without mutation; contenteditable returns unsupported; password returns unsupported without exposing its value; actual overlay at the click point causes stale refusal with no click-handler effect; a DOM drag moves from (20,100) to (140,160) and passes its bounds predicate. Strict-background text uses the page/CDP route with no native requirement. Profile/process are removed by explicit shutdown on success and `Drop` cleanup on assertion failure. |

The masked/event-dependent check uses app-provided `data-masked` and `data-requires-trusted` markers. Undeclared app semantics are not inferred from DOM shape. IME, canvas drag, app-specific identity/document semantics, and independent OS focus/cursor/clipboard observation remain unqualified.

## Phase 4 repeated Chrome interference fixture — 2026-10-07

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test -p controlla-browser --locked --offline real_chrome_headless_provider_launch_and_runtime_smoke -- --ignored --nocapture` | pass | macOS 26 / Darwin 25.6 arm64, installed Google Chrome | 1 passed. Three deterministic cycles reset and blur the input, trigger its real Chrome focus handler to write an external value, then assert guarded fill yields and the external value remains. This is an isolated fixture, not a live-user race or check-to-dispatch timing bound. |

## Phase 5 local library gate — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test -p controlla-browser --lib observe::tests --locked --offline` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | Two tests pass: policy fixture table for virtualized 42/recycled 8 nodes, separate bucket, blocked expansion, duplicate labels, stale count, infinite feed and wrong-account 404; overlapping recycled-ID batches deduplicate to 42. These are model/policy fixtures, not browser DOM runs. |
| `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test --workspace --locked --offline --quiet` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | Format and warning-free Clippy; 73 browser unit tests passed, 1 ignored; integration groups 7, 3, 8, 10, and 3 passed. |
| `npm run check:provenance && node scripts/check-doc-links.mjs && npm run check:docs && ./scripts/check-dependencies.sh` | pass after digest update | macOS 26 / Node 24.19.0 / npm 11.17.0 | Phase 5 edits and `lib.rs` export are recorded in provenance; Markdown links checked. |

At the time of the earlier Phase 5 review, no extraction call had run against Chrome and no MCP observe tool existed. The subsequent MCP and synthetic-fixture evidence is recorded below; AX/screenshot routes and resumable cursors remain unverified. Matrix rows CC-08/15 and B14–17 remain partial.

## Phase 5 review follow-up — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test -p controlla-browser --lib observe::tests --locked --offline -- --nocapture` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | Focused checks include wrong-account mock receives only the read-only preflight and no scroll command; invalid selector Runtime.evaluate exception is rejected; malformed evaluation value is rejected; valid empty items remain accepted; no authoritative expected count remains partial; absent fields block completion; page script contains record/text/byte caps; final serialized objects obey the full byte budget or return an error. |
| `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test --workspace --locked --offline --quiet` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 81 browser tests passed, 1 ignored; integration groups 7, 3, 8, 10, and 3 passed; warning-free Clippy and formatting. |
| `npm run check:provenance && npm run check:docs && ./scripts/check-dependencies.sh && git diff --check` | pass | macOS 26 / Node 24.19.0 / npm 11.17.0 | 13 destination digests and 22 retained tests; 15 Markdown files; dependency and whitespace checks clean. |

The page-script limit test inspects the generated script contract; the wrong-account/no-scroll test uses a mock CDP server. Neither is a live Chrome extraction result. Limits bound selected records, text, page response bytes, and final serialized response bytes; they do not bound native selector/text evaluation time.

## Phase 5 MCP stdio slice — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| rmcp initialize, `tools/list`, discovery `tools/call` | pass | macOS 26 / Rust 1.99.0 | Required fields and object/type schema properties verified for session, observe, and extract. |
| Session connect/list-targets, observe, and two-section extract through MCP against mocked CDP | pass | macOS 26 / Rust 1.99.0 | Real MCP request/response roundtrip; explicit target selection, reference capture/validation, and bounded browser library calls. |
| MCP shared pairing, bounded shared observation, and file artifact tools | pass | macOS 26 / Rust 1.99.0 | Loopback bridge fixture checks one-session authentication, selected tab IDs, root-frame freshness before/after shared observe, stale-navigation refusal, artifact registration, and guarded file-input selection refusal before dispatch when the caller's marker mismatches. The unpacked extension is loaded, but live pairing/dispatch and external MCP client acceptance remain unverified. |
| Endpoint/grant checks | pass | Rust fixtures | Loopback-only parsing, credentials/DNS/non-loopback rejection, separate Direct CDP grant, shared-extension denial. |
| Synthetic virtual-list extraction | pass | Installed Chrome 154.0.8037.98; commits `44041af` and current review follow-up | Latest single synthetic run: 42 extracted rows, 438.17 ms / 1,860 B versus 1.39 ms / 2,760 B full DOM; wrong-account preflight did not scroll. Diagnostic only; no performance claim. Blocked expansion is represented by the policy fixture, not the live run. |
| Sol review follow-up | pass | Phase 4/5 current continuation | Independent Sol reviewer rechecked and approved four fixes: page-only target selection, release the global session-map lock before CDP awaits, validate sequential input before focus, and report every section skipped after timeout. MCP regression covers non-page rejection and timeout coverage. |
| Final workspace gate | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | 82 browser unit tests passed, 2 ignored; integrations 7, 8, 10, and 3 passed; 6 runtime unit tests passed. Formatting, Clippy, provenance (13 digests/22 retained tests), docs (15 files), dependency audit, and `git diff --check` passed. |

MCP transport and extraction assertions use mocked CDP, not an external MCP client or real application account/list. The single installed-Chrome synthetic run is not a general virtualized/hidden-section qualification.

## Phase 4/5 completion gate — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test --workspace --locked --offline --quiet` | pass | macOS 26 / Rust 1.99.0 | 90 browser unit tests passed, 2 ignored; all workspace integration/runtime groups passed. |
| Phase 4 installed-Chrome fixture | pass | macOS 26 / Chrome 154.0.8037.98 | Guarded text and IME composition, native state snapshots, per-session text paste, hidden file-input selection, text-input refusal, replacement-race refusal, and filename/size readback. |
| Phase 5 installed-Chrome fixture | pass | macOS 26 / Chrome 154.0.8037.98 | Selected-node AX, byte-budgeted PNG crop, resumable cursor/stale rejection, and successful/blocked hidden-section expansion. |
| Formatting, Clippy, docs, provenance, dependency, packaging, and whitespace gates | pass | macOS 26 / Rust 1.99.0 / Node 24.19.0 | `cargo fmt`, workspace Clippy, 13 provenance digests/22 retained tests, 15 Markdown files, dependency audit, Cargo workspace pack, host-bound npm archive and clean-prefix install, and `git diff --check`. |
| Sol low independent review | approved | commits `197b8a6`, `8784f1b`, `1fd49f9`, `d211c50`, `e69674b`, `a50d088` | Final review approved scoped local Phase 4/5 gates; earlier artifact selection and cleanup findings were fixed and re-reviewed. |
| Shared click guard false-refusal regression | pass | macOS 26 / Rust 1.99.0 | The production guard allows implicit-submit buttons without a form and still blocks form-associated submit/reset and sensitive input types. The focused shared-fill regression also passed. Independent GPT-6 Sol review approved the current click-guard and Phase 6 process-kill diffs. |

The local Phase 4/5 gates are complete. File insertion proves Chrome selected the requested file, not app acceptance or persistence. The MCP shared route is fixture-verified, not qualified against the installed extension or external client. Remaining boundaries are listed in `blockers.md`: OS-level IME, canvas app-specific verification, repeated/native interference outside the single macOS fixture, representative app behavior, other OS/mode cells, and external MCP client acceptance.

## Phase 6 deterministic workflow gate — 2026-10-06

| Check | Result | Environment | Evidence |
|---|---|---|---|
| rquickjs 0.14.0 worker red/green tests | pass | macOS 26 / Rust 1.99.0 | Tests verify async broker await, interrupting an infinite loop, heap/output caps, aggregate declared observe-byte reservation before broker dispatch, unavailable `node:fs` import, absent process/network/file globals, and rejection of cross-session fields. Aggregate test initially failed because the second 600 KB observation was allowed; after the fix only the first reaches the broker. |
| Durable job journal tests | pass | macOS 26 / Rust 1.99.0 | Checkpoint survives journal reopen, each dispatch increments the claim count, timeout unknown state persists; all `tests/jobs.rs` cases pass. |
| Startup journal recovery | pass | macOS 26 / Rust 1.99.0 | Reopen test invokes the shared production recovery function: accepted jobs become failed/not-sent; running jobs become unknown, preserve checkpoint/dispatch count, and reject redispatch. `mcp::run` invokes this recovery immediately after opening its durable journal. |
| State-directory lifetime lock | pass | macOS 26 / Rust 1.99.0 | Standard-library `File::try_lock` test rejects a second owner with an actionable `CONTROLLA_STATE_DIR` message, then permits acquisition after the first owner drops. `mcp::run` holds the lock before journal open/recovery. |
| MCP workflow call against mocked CDP | pass | macOS 26 / Rust 1.99.0 | Stdio-style duplex call admits async operation, polls `workflow_status` to completion, verifies target identity, browser operation count, idempotency replay, and rejects mismatched session TargetRef. |
| Inline workflow artifact contract/status/replay | pass | macOS 26 / Rust 1.99.0 | Trusted-local script returns a 2-byte JSON artifact through mocked CDP; status and idempotent replay return identical bytes with operation/principal/session binding and SHA-256. Parser tests reject path names, unsupported types, byte overflow, oversized output, and unknown fields; a journal reopen test confirms durable retrieval remains principal/session scoped. |
| Forced child-process termination and journal recovery | pass, scoped | macOS / Rust 1.99.0 | `cargo test -p controlla-runtime --test jobs subprocess_kill_recovers_running_job_without_replaying_effects --locked --offline -- --nocapture` passed. A child was force-killed after two fixture dispatch claims; reopened journal preserved the checkpoint and unknown delivery, and rejected replay without repeating fixture effects. This does not kill the MCP service during a live Chrome dispatch. |
| `cargo fmt --all && cargo test --workspace --locked --offline` | pass | macOS 26 / Darwin 25.6 arm64, Rust 1.99.0 | Browser 91 passed/2 ignored; runtime unit 16 passed; CLI 8, job 12, registry 3 passed; phase2 7 passed; phase5 advanced 1 ignored; doc tests pass. |

At this dated Phase 6 gate, the selected JavaScript candidate was pinned `rquickjs 0.14.0`, verified with a runnable probe and worker tests; the worker still ran in-process. A later Phase 6 change moved the worker into a bounded child process with a cleared environment. Wasmtime is a WebAssembly runtime, not a JavaScript engine. The current child process remains opt-in and trusted-local-only; QuickJS limits and process separation are not an OS security boundary, and no hard OS CPU/RSS controls are enforced. Untrusted/production scripts, hard OS CPU/RSS controls, arbitrary mutations, file-backed/download artifact output, measured browser overhead, and benchmark comparison remain open. Dependency invalidation and effect-aware splitting exist as deterministic planning rules only. No `node:vm` boundary is used.

## Phase 9 local client setup preview — 2026-10-06

Focused verification below ran 2026-10-06 20:37 EDT on implementation commit `20f3ac23dc5a8e3adddff639adf3daf58619f3a5`.

| Check | Result | Environment | Evidence |
|---|---|---|---|
| Red-first MCP guide contract | pass | macOS 26 / Rust 1.99.0 | Before implementation, the stdio handler test failed because `tools/list` did not include `guide`; after implementation, the same test confirms listing, version-matched content, and rejection of a stale server version. |
| `cargo fmt --all -- --check && cargo clippy -p controlla-runtime --all-targets --locked --offline -- -D warnings` | pass | macOS 26 / Rust 1.99.0 | Formatting and runtime-target Clippy clean. |
| `cargo test -p controlla-runtime --locked --offline --quiet` | pass | macOS 26 / Rust 1.99.0 | 30 runtime/unit/integration tests passed across 4 test groups; no failures. MCP guide call used the in-process rmcp duplex harness, not an external client. |
| `npm run check:docs` | pass | Node.js local | Checked local Markdown links in 15 files. |
| `node tests/guide/check-master-example.mjs` | pass | Node.js local | Parsed the master-guide JSON sample and checked its documented workflow/request fields; this does not call MCP or qualify B35. |
| Master guide B35 usability | pass (20/20) | Four fresh-agent groups / rebuilt packaged local stdio MCP schema; 2026-10-06 | Separate 5-task groups assessed target/route boundaries, professional app limits, long jobs/unknown outcomes, extraction/resume and file selection. Each group queried the rebuilt server's tool catalog and its per-scenario record is checked in under `docs/review/phase9-b35-*.md`. No browser, app, or external client was connected. |
| Shared `shared_input` route | pass | macOS 26 / Rust 1.99.0 | Duplex extension-protocol fixture covers event-handler-changed fill readback, stale/ambiguous refusal, click-coordinate refresh, inherited-disabled/offscreen/overlay script guards, plus sequential ASCII key events tied to one retained CDP DOM object. It verifies selector replacement refusal, empty-input no-dispatch, non-ASCII rejection, and injected key-down/character/key-up failures with one bounded key-up retry; a transient release error remains unverified even when retry is acknowledged. This is fixture evidence, not live Chrome extension/app acceptance. |
| Same-tab navigation continuity (`node extensions/chrome-controlla/test-background.cjs`; `cargo test -p controlla-runtime shared_click_reports_verified_navigation_in_the_same_tab`) | pass | Local extension and MCP fixtures | A committed HTTP(S) navigation refreshes the document binding while retaining the explicitly selected tab; an extension-reported debugger detach during navigation is restored only for that same tab, and failed or unreported navigation still fails closed. The MCP click fixture confirms a new loader and URL are returned as a verified navigation. Fixture evidence only; live Chrome retry is separate. |
| Fixed-paced shared typing (`cargo test -p controlla-runtime --lib mcp_shared_input_fills_with_readback_and_refuses_stale_or_ambiguous_targets`) | pass | macOS 26 / Rust 1.99.0 | Runtime fixture observed two actual keyDown dispatches at least 60 ms apart; the pace is fixed (non-random), and a 100-character request at the 6 second minimum timeout was rejected before any browser command. Minimum duration is `(character count - 1) × 60 ms`; the action budget reserves 5 seconds of the requested overall timeout for framing/cleanup. CDP events are not OS keyboard input/native IME and are not for bypassing app security or bot checks. This is local fixture evidence only. |
| `cargo test -p controlla-runtime --lib` | pass, 63/63 | macOS 26 / Rust 1.99.0 | Full runtime library suite passed after the pacing change. |
| Parse fenced JSON in `docs/clients.md` and `git diff --check` | pass | Python 3 / working tree | Three client JSON config examples parse; whitespace check clean. |
| Client acceptance | open | Not run | No Freebuff, OpenCode, Claude Code, or ChatGPT client was launched. Documentation/source schema review is not external client acceptance. |
| Authenticated loopback HTTP MCP route | pass (in-process) | macOS / Rust 1.99.0 / rmcp 3.5.1 | Focused tests reject missing/duplicate bearer, wrong principal, Host, and Origin, then initialize MCP, list tools, and call the read-only guide. No listener process or external client was launched. |
| Live HTTP service process | pass, bounded | macOS arm64; local `serve-http` process | A temporary loopback service completed initialize, `tools/list`, and a master guide v3 tool call; a request without bearer was rejected with HTTP 401. Random bearer and disposable state were removed after shutdown. This was a protocol harness, not a supported external MCP client. |
| External HTTP/client acceptance | open | Not run | No Freebuff/OpenCode/Claude/ChatGPT client connected over HTTP. The stdio OpenCode probe verifies connection/discovery only; no client tool action was issued. |
| Current discovery lifecycle over packaged stdio | pass | macOS / Rust / rmcp 3.5.1 | A child process negotiated the SDK's latest protocol without `initialize`, then listed tools and versioned guide resources. This covers only the current lifecycle over local stdio, not other revisions/transports or real clients. |
| Every rmcp-advertised initialize revision over packaged stdio | pass | macOS / Rust / rmcp 3.5.1 | `cargo test -p controlla-runtime --test stdio_conformance --locked --offline -- --nocapture` passed. Child processes negotiated 2024-11-05, 2025-03-26, 2025-06-18, and 2025-11-25 and listed tools. This verifies the pinned SDK/server's initialize revisions over local stdio only; no alternate transport or external client acceptance is claimed. |

The client guide schema references were checked on 2026-10-06. Client binary versions were not recorded; examples are version/schema-labeled where formats differ and are not an advertised live support matrix.

## Phase 9 registry-derived bootstrap — 2026-10-06

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| `cargo test -p controlla-runtime --lib bootstrap_instructions_include_the_runtime_tool_registry --locked --offline --quiet` | pass | macOS / Rust 1.99.0 | Initialization instructions enumerate names from `App::tool_router().list_all()` and remain at most 600 characters. The bootstrap directs clients to `tools/list` for schemas; guide operation descriptions remain human-authored. |
| `cargo test -p controlla-runtime --locked --offline --quiet` | pass | macOS / Rust 1.99.0 | 31 runtime unit tests and all runtime integration groups passed, including the shared input and Windows artifact refusal regression. |
| `cargo clippy -p controlla-runtime --all-targets --locked --offline -- -D warnings`; `cargo fmt --all -- --check`; `node tests/guide/check-master-example.mjs`; `npm run check:docs`; `git diff --check` | pass | macOS / Rust 1.99.0 / Node.js local | Runtime Clippy, workspace format/whitespace, guide sample shape, and all 23 Markdown files passed. |

This is a short registry-derived bootstrap, not generated documentation or B35 usability evidence.

## Phase 9 authenticated loopback transport — 2026-10-07

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| `cargo test -p controlla-runtime http_server::tests --locked --offline -- --nocapture` | pass (4/4) | macOS / Rust 1.99.0 / rmcp 3.5.1 | Exercises endpoint-audience binding, app/credential principal match, duplicate/missing bearer rejection, strict Host/Origin rejection, authenticated initialize, `tools/list`, and read-only `guide` over an in-process Axum/RMCP service. The HTTP listener was not bound or launched. |
| HTTP security review | pass | Independent GPT-6 Sol review, pre-live | No actionable findings in auth/routing/principal/body/protocol/state-lock/CLI code. Static review only; no listener, browser, or external MCP client was used. |
| Full integrated verification | pass | macOS / Rust 1.99.0 / Node.js 24.19.0 | `cargo test --workspace --locked --offline --quiet`: 183 passed, 0 failed, 3 ignored (installed-Chrome-only); workspace Clippy and format passed. Client, app-brief, benchmark, and docs checks passed. Provenance was rerun after refreshing the build-plan digest. |

## Pre-live integration checks — 2026-10-06

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| Shared extension/server version handshake | pass | Rust 1.99.0 unit fixtures and Node 24 extension harness | Extension reports its manifest version; the local server rejects missing or stale versions with a reload instruction; the extension refuses a server version mismatch and releases attached tabs. No live extension pairing was performed. |
| Full workspace tests | pass | macOS 26 / Rust 1.99.0 | 157 passed, 3 ignored (installed-Chrome-only); all configured unit and integration tests pass. |
| Workspace Clippy and formatting | pass | macOS 26 / Rust 1.99.0 | `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` and `cargo fmt --all -- --check`. |
| Offline Phase 10 fixture contract | pass | Node.js 24.19.0 | Twelve tests cover clone resets, bounded state changes, all 30 pilot task fixture shapes, independent predicates, malformed conditions, invalid result rejection, and frozen split binding. Fixture validation only; not browser/model benchmark evidence. |

## Phase 12 local release candidate — 2026-10-06

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| Clean-tree release-candidate assembly | pass | macOS arm64 / source commit `5e2914f` | `npm run prepare:release-candidate`; generated npm archive, extension archive, host-filtered Cargo SBOM, release manifest, and SHA256SUMS under `dist/release-candidate/`. Candidate is unpublished. |
| Candidate file digests | pass | macOS arm64 | All entries in `SHA256SUMS` verified. This checks local artifact integrity, not consumer download or signature verification. |
| Cross-platform build/package CI | pass | GitHub Actions / source commit `2f4fb06` | Run `37564319202` passed Rust format, Clippy, workspace tests, host package build, and package checks on Windows, macOS, and Linux. No published consumer install was performed. |

## Phase 12 distinct-binary lifecycle — 2026-10-07

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| Distinct-binary package lifecycle | pass | macOS arm64; old source `2f4fb06`, candidate source `b0a7c7c` | Separate binaries were packed as package `0.1.0` and `0.1.1`; install, upgrade, rollback, uninstall and user-data preservation passed with installed executable digest assertions. Full hashes and command: [lifecycle evidence](review/phase12-binary-lifecycle.md). Not an MCP client/server compatibility test. |

## Phase 12 final pushed candidate — 2026-10-07

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| Rust/package CI | pass | Windows, macOS, Ubuntu; source `13c64e3` | [GitHub Actions run 37618717861](https://github.com/Praket7/chrome-controlla/actions/runs/37618717861) passed all build, test, dependency, docs, provenance, and package jobs. |
| Local unpublished candidate | pass | macOS arm64; source `13c64e3` | `npm run prepare:release-candidate` completed; checksums for the npm package, extension archive, and SBOM verified. Candidate status is `local-candidate-not-published`; no live client/app/browser acceptance. |

## Phase 12 current source candidate — 2026-10-07

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| Release candidate build and package lifecycle | pass | macOS arm64 | `npm run prepare:release-candidate` completed for commit `7d5b95b63201350b806068686ef871d31bd92841`; clean-prefix install and upgrade/rollback/uninstall lifecycle passed; staged SBOM, npm archive, extension archive, and `SHA256SUMS` verified. |
| Hosted CI | pass | GitHub Actions | Run [37624887673](https://github.com/Praket7/chrome-controlla/actions/runs/37624887673) passed Ubuntu, macOS, and Windows. |
| Host-matched clean-prefix consumer install | pass | Windows, macOS, Linux; GitHub Actions | Run [37628108314](https://github.com/Praket7/chrome-controlla/actions/runs/37628108314) passed `package-check.sh` on all three hosts; each job packed, installed to a clean prefix, and ran the installed command shim. This was not a published GitHub install. |
| Live use of this server binary | pass, bounded fixture route (2026-10-09) | Locked local Mac | The current local entry served the live fixture route recorded below. This supersedes the prior pending status; real app workflows remain unqualified. |

## Live shared-extension pairing and read-only observation — 2026-10-07

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| Updated stdio MCP process | pass | Codex desktop, local macOS process | The configured `target/release/controlla mcp` process is running from the Phase 4/5 worktree. Session discovery works. |
| Test Classroom selection | pass | Existing Chrome extension browser; user-authorized test account | Opened the supplied test Classroom and selected only that tab in the extension popup. No class content was modified. |
| Earlier shared-extension handshake | failed, superseded | Chrome extension popup + local Controlla 0.1.0 | An earlier attempt returned `extension version unknown does not match server version 0.1.0`. A later fresh session below accepted the loaded 0.1.0 extension. |
| Shared-extension handshake | pass | Chrome Controlla Shared Tab Bridge 0.1.0 + local Controlla 0.1.0 | Popup paired only Chrome tab `1649771390` (`testing - Classroom`); server accepted session `session-3-1`. The endpoint/token are intentionally omitted. |
| Shared target enumeration | pass | Same live session | `list_shared_targets` returned exact URL `https://classroom.google.com/c/ODI2NTQ5Mjc1ODU3` with root frame. |
| Shared read-only observation | pass, bounded | Same live session | `shared_observe` on `body` returned a focused `h1` title (`Classroom`, `testing`) with no truncation; root frame, loader, and URL matched before and after. A broader body-text read was truncated, and the first `main` query returned empty/truncated; this verifies one bounded target-bound observation, not full-page extraction. No writes were sent. |
| Shared-session cleanup | pass | Same live session | `release_shared` confirmed the provider debugger attachment was released after observation; the Chrome tab was left open. |
| Extension source contract | pass | Local `extensions/chrome-controlla/` | Manifest version is `0.1.0`, permissions are `tabs` and `debugger`, and `node extensions/chrome-controlla/test-background.cjs` passes its version-handshake, cleanup, and command-allowlist checks. The live accepted handshake independently confirms a matching 0.1.0 extension was loaded. |
| Guarded shared input attempt | blocked, historical pre-fix | Test Classroom home; session `session-6-1` | A read-only button inventory succeeded; the guarded click on `button[aria-label='Main Menu']` was refused as blocked. No click was dispatched and no page state changed. The predicate was fixed and locally reviewed afterward; no post-fix live click has been sent yet. |
| Save or app persistence | not run | Same live session | No content write or save command was sent. |
| External MCP client action | pass, narrow | macOS arm64; OpenCode 1.18.5; local Qwen 2.5 7B; rebuilt package binary SHA-256 `e9080d9bcb89a6e514b184b0b75ca10c0a93a70cccf31d557d69bfc2a38bc9a7` from commit `53a278a` | With an isolated generated OpenCode v1 config and only the read-only guide tool enabled, an OpenCode run emitted a completed `tool_use` for `chrome-controlla_guide` with `{server_version:"0.1.0",topic:"clients"}`. It returned `clients-2026-10-06-v1`. No saved client config or browser state was changed. The generated v2 shape was rejected by this installed v1 client. The `artifact_verify` tool schema and workflow were not enabled or tested. This verifies one read-only client tool action, not workflow acceptance. |
| Other external-client acceptance | open | Freebuff, Claude Code, ChatGPT and HTTP clients | No tool-call, workflow, artifact or cleanup behavior was tested through those clients. |

## Phase 3–5 focused follow-up — 2026-10-07

| Check | Result | Environment | Evidence |
|---|---|---|---|
| `cargo test -p controlla-runtime --test jobs --locked --offline` | pass | macOS 26 / Rust 1.99.0 | 17 passed. Subprocess restart/replay coverage spans seven durable fixture cut points; Sol approved the atomic readiness marker and claim-before-effect boundary. |
| `cargo test -p controlla-browser --locked --offline real_chrome_headless_provider_launch_and_runtime_smoke -- --ignored --nocapture` | pass | macOS 26 / installed Chrome | 1 passed. Repeats the focus-handler interference path three times; all guarded writes yield and preserve the external value. Sol approved the test and its narrow claims. |
| `cargo test -p controlla-browser --test phase5_advanced --locked --offline -- --ignored --nocapture` | pass | macOS 26 / installed Chrome | 1 passed. Rechecked selected-node AX, bounded crop, resumable cursor/stale rejection, and successful/blocked expansion. Sol approved the Phase 5 local evidence and corrected extraction-count semantics. |
| `cargo fmt --all -- --check`, workspace Clippy, `cargo test --workspace --locked --offline --quiet` | pass | macOS 26 / Rust 1.99.0 | 92 browser tests passed, 2 ignored; integration/runtime groups 7, 55, 8, 17, 4, 3, and 3 passed; Clippy and formatting clean. |
| Provenance, Markdown links, `git diff --check` | pass | Node.js / local workspace | 13 destination digests and 22 retained tests verified; local links checked in 29 files. |
| Current MCP direct route | blocked | Codex desktop | Direct `guide` and `session` now return `Transport closed`. Process inspection found a child from the Phase 9 checkout, while current configuration targets the Phase 4/5 checkout. Restart Codex Desktop to reconnect; no extension restart is indicated. |
| OpenCode stdio connection to current candidate | pass, connection only | OpenCode 1.18.5; local candidate SHA-256 `c88a29cb183524b32a66181e0612938314b73e3a3f1c7813d1d3c71962c60878` | An isolated generated v1 config connected to the release binary; auto-connect was omitted to avoid opening remote debugging. No tool action was called. Existing OpenCode config was not changed. |

The current guide distinguishes Direct CDP AX/crop/extraction from the paired extension's `shared_observe` and `shared_input` routes. No extension source changed in this follow-up.

## Current Codex MCP connection diagnostic — 2026-10-07

The rows below retain the initial direct-entry failure. The subsequent Hotload route and live check supersede that connection status.

| Check | Result | Environment | Evidence |
|---|---|---|---|
| Configured command/state path | pass | Codex Desktop config | `~/.codex/config.toml` points to this checkout's `target/release/controlla mcp` and `/Users/pcg/.chrome-controlla/codex-chat-phase6-12`. |
| Host-loaded Controlla tools | blocked | Current Codex task | Hotload reports `connection closed: initialize response`; no direct Controlla namespace is exposed. |
| Standalone stdio initialize + tool discovery | pass | Release candidate, exact configured state path | After stopping the Codex-launched child, a standalone stdio client received an initialize response and `tools/list` with 20 tools. This excludes a general binary/protocol failure but does not prove Codex-hosted connection health. |
| Hotload refresh | queued | Codex Desktop | `hotload_reload_server({})` returned `reloaded=true`, `verified=true`, `refresh=queued_for_next_active_turn`. No browser action was run. |

## Codex Hotload and sequential shared-tab pairing — 2026-10-07

| Check | Result | Evidence |
|---|---|---|
| MCP tool loading | pass | Local Hotload loaded 20 current Controlla tools; `guide` and `session discover` returned through this task. The duplicate direct server entry is disabled. |
| Red-first pairing regression | pass | Before the fix, `shared_pairing_accepts_extension_before_followup_tool_call` timed out because `pair_shared` did not begin accepting. After the fix, the extension can complete its hello before `accept_shared`, and an early acceptance check returns promptly. |
| Local validation | pass | `cargo test --workspace --locked --offline`: 190 passed, 0 failed, 3 Chrome-required tests ignored by default. `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, the extension background fixture, provenance validation, documentation link check, and `git diff --check` passed. Release binary rebuilt successfully. |
| Independent review | pass | GPT-6 Sol reviewed the pairing task lifecycle, ownership, cancellation, and security checks; no blocker found. |
| Sequential live pairing | pass | Loaded Chrome Controlla Shared Tab Bridge 0.1.0 paired only tab `1649771543` using the popup; the corrected release server returned `accepted: true` after the popup had completed. No concurrent waiting tool call was needed. |
| Live target and observation | pass | `list_shared_targets` returned only tab `1649771543` and `https://classroom.google.com/c/ODI2NTQ5Mjc1ODU3`; `shared_observe` returned `Classroom / testing` with the same root frame, loader, and URL before and after. No write was sent. |

## Native bridge discovery verification — 2026-10-08

| Check | Result | Environment | Evidence / boundary |
|---|---|---|---|
| Corrected extension origin | pass | Installed Chrome + local native host | Chrome showed extension ID `bhgfjpbajecihfbgaikampminappgodh`; the native-host manifest was corrected to that exact origin after the prior launch rejection. |
| Native host and private inventory | pass | Installed Chrome + local native host | After the rebuilt release binary and unpacked extension were reloaded, Chrome launched the registered native host. `discover_shared_tabs` returned a fresh inventory with a host ID, 51 tabs, and `truncated:false`. |
| MCP companion discovery | pass | Current Hotload MCP revision 4, `session discover` | Fresh inventory produced `companion_extension.available=true`. This establishes discovery and host reachability only. |
| Native route attachment or command | not run | Same installation | No selected tab was attached and no browser command was sent over the native route. |
| Alarm reconnect and popup native status | fixture only | Extension background/popup fixtures | Local fixtures cover alarm-based reconnection after worker suspension and status rendering. These behaviors have not been verified in the installed extension. |

The native path is the documented default: one-time `controlla install-bridge <exact-extension-id>`, reload the unpacked extension, `discover_shared_tabs`, pass `snapshot.host_id` and exact selected tab IDs to `pair_shared`, then `accept_shared`. Discovery is capped at 100 HTTP(S) tabs and reports `snapshot.truncated`; snapshots expire after 15 seconds. Single-host process fixtures exercise host-specific pairing and fresh inventory; extension fixtures exercise serialized release and cleanup, and a provider fixture rejects an authenticated pair error. Cross-profile interleaving, MCP host-change rejection, and host-error delivery through `accept_shared` have code review only and remain untested end to end. The native attachment and command boundary in the table above was superseded by the later check below. Popup endpoint/token pairing is a fallback only when the native host is unavailable or busy.

## Native shared route live check — 2026-10-08

| Check | Result | Evidence / limit |
|---|---|---|
| Selected-tab attach and release | pass | The current Hotload MCP native route paired only the authorized testing Classroom tab `1649771654` with the exact discovered `host_id`; `accept_shared` returned `accepted:true`, `list_shared_targets` returned that tab and its Classroom root frame, and `release_shared` returned `released:true`. No class data was changed. |
| Shared observation and accessibility | pass | On the same Classroom tab, `shared_observe` returned `Classroom / testing` with root frame, loader, and URL stable before/after. The broad one-item read reported truncation. `shared_accessibility` returned the selected `h1` heading without truncation. This is a selected-node check, not full-page extraction. |
| Guarded fill and click through native route | pass, local page | A temporary HTTP fixture at `127.0.0.1` in Chrome was paired as tab `1649771657`. `shared_input` filled `#field` only when its value was `before`, read back `phase4-native-ok`, then clicked `#apply` only when its text was `Apply` and independently confirmed `#result` changed from `idle` to `phase4-native-ok`. A separate Chrome accessibility read saw the new field and output values. The session was released, fixture tab closed, and server stopped. This proves the scoped native browser path, not Google Classroom writes or app persistence. |
| Earlier hosted three-platform checks | superseded | GitHub run [37763062495](https://github.com/Praket7/chrome-controlla/actions/runs/37763062495) failed before steps due to account payment/spending-limit restrictions. After the repository became public, the final fix run below passed on all three operating systems. |

## Phase 6 local completion gate — 2026-10-08

| Check | Result | Evidence / limit |
|---|---|---|
| No-dispatch completion versus uncertain dispatch | pass | `cargo test -p controlla-runtime --test jobs` passed 18 tests on macOS. A new regression first failed before the fix, then passed: a started job with no browser dispatch completes with `delivery=not_sent`; a claimed but unacknowledged dispatch remains ineligible for completion. Sol independently approved this journal-level transition. This does not prove a full MCP script-only workflow or exactly-once browser effect. |
| Extraction provenance | pass | `node scripts/check-provenance.mjs` verified 13 destination digests and 22 retained tests. |

## Phase 9 Freebuff client probe — 2026-10-08

| Check | Result | Evidence / limit |
|---|---|---|
| Freebuff Desktop connection and Controlla discovery | blocked | Desktop 0.0.134 reported `connected_writable` with write authorization. In one temporary project with isolated Controlla state, its agent said no `chrome-controlla` MCP tools were loaded and made 0/2 requested read-only `guide`/`session` actions; it only listed/read project files. One model turn ran, with 165 Freebucks visible beforehand; exact debit was not checked. No browser operation occurred. CLI 0.2.19 stopped at login in an earlier isolated attempt; its bundled runtime reported 0.2.22, and CLI authentication remains unverified. |
| Generated client defaults | pass, local | Removed `COMPTROL_CHROME_AUTO_CONNECT=1` from Freebuff, OpenCode, and Claude generated configs so the documented native messaging path remains the default. `node tests/clients/check-configs.mjs` passed the generated examples and safe merge behavior. This does not establish Freebuff MCP loading. |
| Hosted Windows import fix | failed, superseded | GitHub run [37765065408](https://github.com/Praket7/chrome-controlla/actions/runs/37765065408) revealed an unused test-only Unix WebSocket import on Windows. |
| Current hosted three-platform verification | pass | GitHub run [37822533307](https://github.com/Praket7/chrome-controlla/actions/runs/37822533307), commit `35a8c1f`, passed on Ubuntu, macOS, and Windows. Each job passed formatting, Clippy, workspace tests, package build, docs/provenance/dependency checks, and package checks. |

## Hotload shared observation and schema boundary — 2026-10-08

| Check | Result | Evidence / limit |
|---|---|---|
| Live native extension route | pass, read-only | Hotload revision 1 and the rebuilt revision 2 each paired only tab `1649771654` (`https://classroom.google.com/c/ODI2NTQ5Mjc1ODU3`), returned `accepted:true`, and released it. Revision 2 `shared_observe` read `Classroom / testing` with stable root-frame and loader identity; two `h1` matches and possible omissions were reported with `truncated:true`. No page write was sent. |
| Observation input schema and compiler ranges | pass, local and live schema | Live `max_bytes:2048` was rejected at runtime although schema allowed zero. A red-first stdio schema test failed at `minimum:0`; direct/shared schema was corrected to the browser contract `4096..=1,000,000`. Source audits found workflow compiler item/text/cursor limits exceeded runtime's 500/10,000/256 limits, and a workflow must reserve 1,024 bytes for its result envelope. Workflow schema/compiler now enforce `max_items=1..=500`, `max_text_chars=1..=10,000`, `max_bytes=4096..=998,976` per observe step plus the aggregate cap, and `cursor<=256`. Focused regression tests cover rejection and exact accepted upper boundaries. Hotload revision 2's live `tools/search` schema read confirms all direct/shared/workflow bounds. |
| Local validation of schema correction | pass | Focused workflow-boundary and stdio schema tests pass. Format, warning-denied Clippy, full workspace tests, Markdown links, provenance, dependency checks, package lifecycle, and `git diff --check` pass. |

## Shared sequential typing and local qualification — 2026-10-08

| Check | Result | Evidence / limit |
|---|---|---|
| `cargo test --workspace --locked --offline` | pass | 202 passed, 0 failed; 4 Chrome-dependent tests are ignored by default. |
| Installed-Chrome fixtures with `--ignored --test-threads=1` | pass | All 3 browser-provider Chrome fixtures and the Phase 5 accessibility/crop/resumable-extraction fixture passed with isolated headless profiles. These are fixtures, not Slides, Canva, CapCut, or Classroom acceptance. |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`; `cargo fmt --check`; `git diff --check` | pass | Warning-denied static analysis, formatting, and whitespace checks are clean after the typing guard and cleanup-error fix. |
| Extension fixture; docs, clients, app briefs, provenance, benchmark harness, package lifecycle and package archive checks | pass | The extension fixture covers the key-dispatch allowlist and pairing cleanup; provenance verifies 13 destination digests and 22 retained tests; the benchmark harness remains offline and measures no browser/model performance. Package checks are local darwin/arm64 only. |
| Independent review of shared sequential typing | approved | The high reviewer approved original DOM identity, selector replacement, keyDown/char/keyUp uncertainty, bounded release recovery, cleanup-error reporting, fixed 60 ms pacing between dispatched characters, deadline preflight math, and the fixture-only evidence boundary. |
| Live shared typing in Chrome | pass, local fixture | On 2026-10-08, after rebuilding/reloading Hotload revision 3, Controlla typed `Hello, world!` into the exact disposable localhost tab `1649771820`; final value, focus, and caret were verified. Chrome recorded 13 each of `keydown`, `keypress`, `beforeinput`, `input`, and `keyup` (65 events total); the 12 keyDown gaps were 66.7–81 ms, median 71 ms. The native session was released, tab closed, and local server stopped. The unpacked extension was already loaded and had no source changes; no further extension or Codex restart is needed. This does not qualify real app acceptance/persistence, IME, rich editors, canvas, or other OS/browser targets. |

These checks do not close the app-specific acceptance, performance, external-client, benchmark-run, or public-release gates recorded above.

## Paired-extension diagnostics and test-tab isolation — 2026-10-08

| Check | Result | Evidence / limit |
|---|---|---|
| Native discovery, exact test-tab pairing, acceptance, and release | pass | Fresh native bridge discovery paired only the selected Slides tab `1649771881`, then the authorized test Classroom tab `1649771875`; both sessions were released. |
| First read-only debugger command on old Slides and Classroom tabs | failed, target-specific | The first commands timed out on the selected tabs; no page write was issued. For original Classroom tab `1649771875`, the later service-worker trace shows `Page.getFrameTree` errored after 22,004 ms and the reply was suppressed after the native deadline. |
| Fresh test Classroom tab | pass, live read-only | A newly opened tab at the same authorized testing Classroom URL returned its frame tree and a bounded `h1` observation (“Classroom / testing”). The paired session was released and the temporary tab closed. This supports a stale/unresponsive old-tab diagnosis; it does not verify Slides, Canva, CapCut, or persistence. |
| Diagnostic source change | pass, fixture and independent review | Extension logs bounded `dispatch`, debugger `settled`, and native `reply` state, including numeric command IDs. Logs exclude page data, URLs, command parameters, tokens, and error bodies. Fixtures cover pending settlement and stale-generation suppression; focused tests, both `node --check` invocations, and `git diff --check` pass. |
| Full local workspace and installed-Chrome validation | pass | `cargo test --workspace --locked --offline`, warning-denied Clippy, formatting, dependency/docs/provenance checks pass. Three installed-Chrome provider tests pass with isolated headless profiles; this does not verify the shared extension route or app persistence. |
| Chrome CDP composition protocol | pass, isolated headless fixture | The installed-Chrome provider smoke fixture exercised `Input.imeSetComposition`, observed compositionstart/update/end and composing input, and verified the resulting Unicode field value. This is Chrome DevTools Protocol composition, not native macOS IME UI or other OS/browser input. |
| Client/app/benchmark offline suites | pass, fixture-only | `npm run check:clients`, `npm run check:apps`, and `npm run check:bench` pass. The benchmark validator verifies preregistration/manifests and local synthetic counts; it explicitly measures no browser latency or model usage and does not count as pilot or held-out execution. |
| Slides, Canva, CapCut save/persistence and export | not verified | Current routes are planners/recipe validation only. No app document or media was changed. |

## Source-audit repair queue A1–A8 — 2026-10-08

Fixture-level repairs of the independent source-audit findings in `handoff.md` §6. These are code-level fixes with focused regression fixtures; none of them was reproduced live in Chrome, and none changes the app-save/persistence, external-client, benchmark, or release gates.

| Finding | Fix and test | Result |
|---|---|---|
| A1 pairing/page binding | Native discovery captures a stable main-frame document ID alongside the URL. Both are required for selected tabs and compared before/after attach; command dispatch rechecks the current ID, while main-frame navigation/commit events invalidate the attachment. An explicit handshake capability rejects older extension code. Fixtures cover missing identity, same-URL document replacement, event invalidation, and command-time mismatch. | pass, fixture |
| A2 per-peer handshake deadline | WS handshake and hello for one inbound peer are bounded to 10 s (`PER_PEER_HANDSHAKE_TIMEOUT`) inside the overall pairing window; timeout, malformed upgrade, and invalid hello are skipped so the accept loop continues. The fixture sends a malformed upgrade and oversized hello before proving the legitimate extension still pairs. | pass, fixture |
| A3 pending-reply leak on cancellation | A `PendingEntryGuard` drop guard now owns each pending command entry: normal reply, timeout, transport error, and early future cancellation all remove it; the guard is disarmed only on a settled reply. New fixture: three timed-out commands against a never-replying extension leave zero pending entries, subsequent commands still work, wire command IDs are never reused, and late replies are not replayed. | pass, fixture |
| A4 duplicate dispatch claim | `record_dispatch` rejects re-claiming a correlation that is already acknowledged as `sent`; distinct new-step correlations remain allowed. New journal regression covers same-correlation refusal after ack, new-step claim, and non-acquiring same-correlation retry while unacknowledged. | pass, fixture |
| A5 readonly direct fill | The locator probe reports `readOnly`, admission refuses readonly targets, and the fill mutation rechecks `e.readOnly||e.disabled` inside the same evaluation that writes, refusing to report Applied on a readonly field. | pass, fixture |
| A6 same-selector replacement | Each resolution stamps the element with a unique `__controllaNodeToken`; guarded input now rejects probes that lack the token, and every dispatch-stage script requires the probed token. | pass, fixture |
| A7 pointer release recovery | Click/drag release is routed through a scoped helper with one bounded retry only when node identity still passes. A failed drag move after press now attempts guarded release before returning the movement error; an unconfirmed release reports pointer state as uncertain. | pass, fixture |
| A8 CI coverage | `.github/workflows/ci.yml` now runs `check:clients`, `check:apps`, `check:bench`, the extension fixture, and `node --check` on the extension source. All five pass locally; hosted verification still requires the next CI run. | pass locally, CI pending |

## Earlier live 4-tab window pairing and observation — 2026-10-08

An earlier live session reported that a release binary at this checkout's path served MCP stdio in an isolated state directory while Chrome's pre-existing native host relayed pairing. It observed and released four tabs read-only. The record identifies the binary path and extension ID, but has no binary digest or captured source revision; it therefore remains historical evidence and does not establish that the exact current dirty diff was live-qualified. No page content was modified.

| Check | Result | Evidence / limit |
|---|---|---|
| Native bridge identity | reported pass | Manifest `chrome_controlla_bridge.json` pointed at this checkout's `target/release/controlla`; extension `bhgfjpbajecihfbgaikampminappgodh`; native host process live from Chrome. No binary digest was retained. |
| Discovery → pair → accept (native route) | pass | Fresh inventory (58 tabs, untruncated) listed all four window tabs; `pair_shared` with `host_id` + the four IDs returned `native_bridge:true`; `accept_shared` returned accepted with all four selected targets. |
| Frame identity of all four paired tabs | pass | `list_shared_targets` returned root frame trees for Canva (`canva.com/`), Slides (`docs.google.com/presentation/u/0/`), CapCut (`capcut.com/my-edit`), Classroom (`classroom.google.com/c/ODI2NTQ5Mjc1ODU3`). |
| Bounded observation per tab | pass, sequential sessions | Slides: title "Google Slides", header text read. Canva: title "Home - Canva", skip-navigation text read. CapCut: title "My projects…", banner text read. Classroom: title "testing - Classroom", `h1` = "Classroom\ntesting", main-menu text read. |
| Shared accessibility per tab | pass | `shared_accessibility` on `body` returned bounded AX nodes for all four tabs. |
| Clean release | pass | Every session released; debugger attachments detached. |
| Multi-tab single-session pairing under rapid pair/detach churn | flaky, observed | Repeated multi-tab pairing immediately after prior releases intermittently returned `TargetChanged` on all commands until the extension settled; single-tab sessions and the first post-idle multi-tab session worked. This is attach-churn behavior of the running pre-repair extension build, not a regression from today's changes; it needs the extension reload to re-evaluate. |
| Field-selector semantics | documented | Observation field values are CSS selectors resolved against each matched element (`*` reads the element itself). Passing a DOM property name such as `innerText` yields a missing-field null, not text. |

These live checks are point-in-time read-only observations on this machine; they do not qualify app writes, save/persistence, external clients, or the remaining phase gates.

## Live local Controlla fixture battery — 2026-10-09

The enabled local `[mcp_servers.chrome-controlla]` entry was served through the current Codex Hotload child (`chrome-controlla`, 21 tools; separate state directory). Tests used only the agent-created fixture tab `1649772132` at `http://127.0.0.1:8977/` and `/nav.html`. The bridge paired this exact Chrome tab through `discover_shared_tabs` → `pair_shared` → `accept_shared`; no other tab was selected.

| Check | Result | Evidence and limit |
|---|---|---|
| Shared form fill and type | pass, live fixture | Filled `#post-title` and `#post-body`, then typed into the focused title. Readback was `Phase4/5 live test typed` and `Disposable local fixture body.` |
| Guarded navigation within fixture | pass, live fixture | Clicked `#next-slide` twice; observed `Slide 2 of 3` then `Slide 3 of 3`. Navigating the fixture to `/nav.html` detached the old document-bound session. |
| In-memory post action | pass, live fixture only | Clicked `#post-submit`; fixture status became `Posted: Phase4/5 live test typed`. This is a mock local page state, not a remote post or persisted app change. |
| File input | blocked, fail-closed | `shared_input` fill on `#post-file` returned `CALL_FAILED: shared input refused: blocked`. Follow-up observation found the file input empty; no upload occurred. |
| Re-pair after document navigation | pass, live fixture | Discovered and paired the same tab again after navigation. `shared_observe` returned heading `Navigation reached` and marker `nav-fixture`; root frame, loader, and URL were unchanged across the read, with no truncation. |
| Session and tab cleanup | pass | `release_shared` confirmed debugger attachments were released. Closed only the agent-created fixture tab. |

This verifies live shared-route fill, typing, guarded clicks, stale-document rejection, re-pairing, and bounded observation on the local fixture. It does not verify file upload, real Google/Canva/CapCut workflows, app save/persistence, nor execution of any remote action. The earlier pending status for this server binary is superseded by this fixture-only live run; it must not be read as an app qualification.

## Shared-tab open and navigation — 2026-10-09

| Check | Result | Evidence and limit |
|---|---|---|
| Extension open/navigation protocol | pass, fixture | New active tab was created, attached to the current pairing, and returned its tab ID/URL/document ID; navigating that exact tab reattached and returned its updated URL/document ID. `javascript:` was rejected. |
| MCP tool registration | pass, local | `shared_tab` is present in `tools/list` with action and URL arguments. Runtime and browser-provider compile/tests pass. |
| Live extension/session behavior | not run | The changed unpacked extension has not been reloaded into Chrome; no live open/navigation effect is claimed. |

## Efficiency repair evidence (2026-10-09)

| Repair | Evidence | Boundary |
|---|---|---|
| Schema cache and maintained task client | 7 Node tests: queued startup, type/required validation, stderr separation, timeout without replay, tool error preservation, schema invalidation, child/startup failure | Not a full general JSON Schema engine; server validation remains authoritative |
| Result count independent from scan depth | Real Chrome exact target beyond 200 DOM nodes with max_items=1 | 65,536-node scan ceiling remains explicit |
| Snapshot references and shared click | Real Chrome masked-text, accessible-name, delayed menu, stale node, timeout counter and fragment-navigation regression | Light DOM only; UI outcome is not persistence or causality evidence |
| Distribution and instructions | Workspace tests, clippy, docs/provenance/client checks, extension fixture suite, package install/upgrade/rollback checks; independent static review | Current app/account task and installed-extension end-to-end test still pending |

## Live extension check after reload — 2026-10-09

Passed through the installed native extension and Hotload: accepted a selected-tab session, opened ESPN in a new foreground tab, read a shared_snapshot, clicked the accessible-label league menu and verified expanded=true, then navigated through an observed Gamecast link. A score-card click first opened a menu rather than navigating; shared_click reported unknown for its unsatisfied navigation outcome, returned the new Gamecast control, and did not replay the action. Clicking that new control verified the destination URL. A subsequent snapshot on the same session reported ready_state=complete, the expected game title and 34 controls. Session release succeeded; the foreground tab was preserved. No re-pair or reconnect was needed after navigation. This qualifies that ESPN flow on this Mac; Classroom account switching and the to-do task remain unverified.
