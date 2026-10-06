# Verification matrix

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
| CC-07 | partial | 3 | bounded wait and durable state tested with local async fixture; no Promise/CDP/bridge worker route or reconnecting MCP client |
| CC-08 | partial | 5 | synthetic Chrome 42-record extraction and policy completeness fixtures pass; representative application lists and terminal semantics remain unqualified |
| CC-09 | partial | 2/4 | target/frame/browser revisions and stale-handle fixtures pass; Phase 4 guard compares caller-supplied identity/dependency snapshots; no authoritative app identity observer |
| CC-10 | partial | 4 | fill checks the live value in the same page evaluation that writes; insert and every sequential key event refresh value/focus before each CDP send; navigation race fixture withholds the fill. CDP/page handlers/server effects remain non-atomic |
| CC-11 | partial | 4 | strict-background mouse input returns `NeedsForeground`; read-only macOS observer confirms unchanged frontmost app, cursor, and pasteboard change count for one isolated Chrome text fixture; other OSes/modes and repeated interference remain unqualified |
| CC-12 | partial | 4 | installed Chrome verifies Unicode text/caret, guarded IME composition, fail-closed marked controls, overlay refusal, and DOM drag result; OS IME UI, app-specific semantics, and canvas movement remain unqualified |
| CC-13 | fixture_verified | 2 | 1/4/8 target scheduling, blocked-target fairness, and shared-document mutation serialization fixtures |
| CC-14 | unimplemented | 6 | none |
| CC-15 | partial | 5 | MCP observe/extract plus bounded selected-node AX, PNG crop, resumable extraction, and synthetic Chrome hidden-section fixtures pass; broad app qualification and external client acceptance remain open |
| CC-16 | unimplemented | 7 | none |
| CC-17 | unimplemented | 9 | none |
| CC-18 | unimplemented | 9 | none |
| CC-19 | unimplemented | 8 | none |
| CC-20 | unimplemented | 10 | none |
| CC-21 | unimplemented | 7 | none |
| CC-22 | unimplemented | 11 | none |
| CC-23 | unimplemented | 6 | none |
| CC-24 | unimplemented | 12 | none |
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
| B24 | partial | 4 | read-only macOS observer confirms unchanged pasteboard change count in one isolated strict-background text fixture; other platforms, repeated races, and artifact insertion remain open |
| B25 | partial | 4 | strict-background text uses CDP and one macOS snapshot confirms unchanged frontmost app/cursor; mouse returns `NeedsForeground`; other OSes/modes and repeated disruption remain open |
| B26 | fixture_verified | 2 | crash reconciliation reports owned leftovers and preserves adopted/user tabs |
| B27 | unimplemented | 6 | none |
| B28 | unimplemented | 7 | none |
| B29 | unimplemented | 7 | none |
| B30 | unimplemented | 7 | none |
| B31 | unimplemented | 6 | none |
| B32 | unimplemented | 6 | none |
| B33 | partial | 5 | fixture policy classifies wrong-account 404 unknown; mocked CDP preflight returns unknown and sends no scroll command; real 404/account UI remains unqualified |
| B34 | unimplemented | 8 | none |
| B35 | unimplemented | 9 | none |
| B36 | partial | 4 | real Chrome strict-background text succeeds with no native requirement; strict-background mouse/native routes return `NeedsForeground` in fixtures; no native-dialog fixture or platform observer |

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

The local Phase 4/5 gates are complete. File insertion proves Chrome selected the requested file, not app acceptance or persistence. Remaining boundaries are listed in `blockers.md`: OS-level IME, canvas app-specific verification, repeated/native interference outside the single macOS fixture, representative app behavior, other OS/mode cells, and external MCP client acceptance.
