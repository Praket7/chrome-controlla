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
| CC-04 | fixture_verified | 1 | packed darwin/arm64 npm archive clean-prefix install, spaces and empty PATH; other OS package jobs pending |
| CC-05 | unimplemented | 3 | none |
| CC-06 | unimplemented | 3 | none |
| CC-07 | unimplemented | 3 | none |
| CC-08 | unimplemented | 5 | none |
| CC-09 | unimplemented | 2 | none |
| CC-10 | unimplemented | 4 | none |
| CC-11 | unimplemented | 4 | none |
| CC-12 | unimplemented | 4 | none |
| CC-13 | unimplemented | 2 | none |
| CC-14 | unimplemented | 6 | none |
| CC-15 | unimplemented | 5 | none |
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
| B05 | unimplemented | 3 | none |
| B06 | unimplemented | 3 | none |
| B07 | unimplemented | 3 | none |
| B08 | unimplemented | 2 | none |
| B09 | unimplemented | 4 | none |
| B10 | unimplemented | 4 | none |
| B11 | unimplemented | 4 | none |
| B12 | unimplemented | 4 | none |
| B13 | unimplemented | 2 | none |
| B14 | unimplemented | 5 | none |
| B15 | unimplemented | 5 | none |
| B16 | unimplemented | 5 | none |
| B17 | unimplemented | 5 | none |
| B18 | unimplemented | 3 | none |
| B19 | unimplemented | 3 | none |
| B20 | unimplemented | 2 | none |
| B21 | unimplemented | 2 | none |
| B22 | unimplemented | 4 | none |
| B23 | unimplemented | 4 | none |
| B24 | unimplemented | 4 | none |
| B25 | unimplemented | 4 | none |
| B26 | unimplemented | 2 | none |
| B27 | unimplemented | 6 | none |
| B28 | unimplemented | 7 | none |
| B29 | unimplemented | 7 | none |
| B30 | unimplemented | 7 | none |
| B31 | unimplemented | 6 | none |
| B32 | unimplemented | 6 | none |
| B33 | unimplemented | 5 | none |
| B34 | unimplemented | 8 | none |
| B35 | unimplemented | 9 | none |
| B36 | unimplemented | 4 | none |

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

The npm artifact built here is specifically for darwin/arm64. The generated package metadata constrains OS and CPU so npm rejects installation on unsupported hosts; current hosted results include successful macOS and Ubuntu package jobs. Doctor transport/authentication uses loopback fixture services; no live daemon, browser, client, or production dispatch handler exists. The first Sol Phase 1 review was not approved; findings were fixed and Sol approved the local/fixture gate on 2026-10-06. Phase 1 commit `23b7827` was pushed; hosted run 37460467645 failed on Windows Clippy and the Ubuntu oversized-PID test, while macOS passed. Fix commit `ed2beb2` passed macOS/Ubuntu but exposed a Windows-only doctor expectation in run 37461256280; `b159a3a` fixed it. Hosted run 37461823256 passed macOS/Ubuntu and failed Windows provenance hashes because text files checked out with CRLF; fix commit `63b74d5` pins LF line endings. Hosted run 37462361648 passed macOS/Ubuntu and failed Windows package-check after Cargo verification; fix commit `ec5982c` added diagnostics and uses `npm exec` for platform-correct shim execution. Commit `c2fce64` normalizes Windows tar-list CRLF and adds per-entry diagnostics; hosted confirmation is pending.

## Phase 1 hosted CI run 37460467645 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37460467645](https://github.com/Praket7/chrome-controlla/actions/runs/37460467645); package/runtime jobs passed. |
| Windows | fail | Clippy reported `unused_imports` for Unix-only `Command` and `Stdio` imports in `doctor.rs`; imports are now gated with `#[cfg(unix)]`. |
| Ubuntu | fail | `doctor_reports_fresh_heartbeat_but_dead_process_and_unreachable_loopback` treated PID `4294967295` as alive because it overflowed signed `pid_t`; process probing now rejects values above `i32::MAX` before invoking `/bin/kill`, with a unit test proving the probe is not called. |

The run remains recorded as failed historical evidence. The fixes are locally verified below; hosted confirmation is pending.

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

This failed run remains historical evidence. The Unix branch passes locally; the Windows `Unknown` expectation is source-updated but awaits hosted confirmation.

## Phase 1 hosted CI run 37462361648 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37462361648](https://github.com/Praket7/chrome-controlla/actions/runs/37462361648); full CI passed. |
| Ubuntu | pass | Hosted workflow run [37462361648](https://github.com/Praket7/chrome-controlla/actions/runs/37462361648); full CI passed. |
| Windows | fail | `./scripts/package-check.sh` exited 1 after Cargo package verification. The hosted log did not expose the silent failing subcommand. Package check now emits stage labels and checks the installed command through `npm exec`, which selects the platform-appropriate shim under Git Bash. Fix commit `ec5982c` passes local package check; Windows rerun is pending. |

After the failure, package-check stage labels localized the post-Cargo path, and the installed consumer command check was changed to `npm exec`. `./scripts/package-check.sh` passes locally on macOS, including archive contents, target metadata, attribution, clean-prefix installation, direct package-local launcher, and npm command execution.

## Phase 1 hosted CI run 37463463416 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37463463416](https://github.com/Praket7/chrome-controlla/actions/runs/37463463416). |
| Ubuntu | pass | Hosted workflow run [37463463416](https://github.com/Praket7/chrome-controlla/actions/runs/37463463416). |
| Windows | fail | `package-check.sh` reached `package-check: inspecting npm archive chrome-controlla-0.1.0.tgz` and exited before clean-prefix install. Commit `c2fce64` strips carriage returns from the archive listing and logs each required entry and metadata validation stage. Windows rerun is pending. |

After the failure, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `./scripts/package-build.sh`, `./scripts/package-check.sh`, and `git diff --check` all passed locally on macOS 26 / Darwin 25.6 arm64. All 36 workspace tests passed. The package check logged each archive entry, validated darwin/arm64 metadata and attribution, installed into a clean prefix, and launched both the package-local binary and npm command shim.

## Phase 1 hosted CI run 37461823256 — failed

| Runner | Result | Evidence |
|---|---|---|
| macOS | pass | Hosted workflow run [37461823256](https://github.com/Praket7/chrome-controlla/actions/runs/37461823256); all Rust, package and provenance checks passed. |
| Ubuntu | pass | Hosted workflow run [37461823256](https://github.com/Praket7/chrome-controlla/actions/runs/37461823256); all Rust, package and provenance checks passed. |
| Windows | fail | `check-provenance.mjs` hashed CRLF working-tree files against LF source digests. Added `.gitattributes` with `* text=auto eol=lf`; this correction still requires hosted confirmation. |
