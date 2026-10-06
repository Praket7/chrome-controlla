# Build progress

Status: Phase 0 and Phase 1 are pushed. Phase 0 hosted CI passed. Phase 1 commit `23b7827` passed Sol's local/fixture review. Hosted run `37460467645` found Windows Clippy and Ubuntu PID issues; `ed2beb2` fixed those, and run `37461256280` found one Windows-only test expectation. Commit `b159a3a` fixed that; hosted run `37461823256` passed macOS/Ubuntu and failed Windows provenance digests due to CRLF checkout. Fix commit `63b74d5` pins LF for text files. Hosted run `37462361648` passed macOS and Ubuntu but failed Windows `package-check.sh` after Cargo package verification; commit `ec5982c` added diagnostics and `npm exec` shim handling. Commit `c9bffcf` passed macOS/Ubuntu but failed Windows during npm archive inspection; `c2fce64` normalized tar-list CRLF and added per-entry diagnostics. Hosted run `37464310787` showed the Windows archive contains `.exe` while the shell file test selected the Unix name. Fix commit `9c8adaf` selects the archive entry by Node's platform value; local checks pass and its push/hosted rerun are pending.

## Phase 0 — source extraction and baseline

- [x] Confirm source repository, remote, clean status, and pinned upstream SHA.
- [x] Copy controlling design documents into the new repository.
- [x] Extract only the self-contained browser crate and its inline tests.
- [x] Preserve Apache-2.0 license and upstream notice; record every copied source path and digest.
- [x] Pin Rust 1.99.0, Node.js 24.19.0, npm 11.17.0, Playwright 1.63.0, `rmcp` 3.5.1, MCP protocol 2026-07-28, and Chrome for Testing 154.0.8037.92; generate Rust and npm lockfiles.
- [x] Add a cross-platform Rust build, dependency, package, and documentation-link CI baseline.
- [x] Rerun focused checks after review findings are fixed; retain exact commands/results in `verification-matrix.md`.
- [x] Pass scoped independent review; commit with `chore: establish Chrome-only source extraction and provenance`.

## Review record

- Review 1: not approved; corrections and pinned-toolchain checks are recorded in [Phase 0 review 1](review/phase0-review-1.md). Sol re-review approved Phase 0 on 2026-10-06 after independently checking package, docs, dependency, provenance, and Rust gates. The final npm pin in CI was re-reviewed and approved. Commit `10f1aa7` is pushed to the private repository; hosted CI run `37456380443` passed on macOS, Linux, and Windows.

## Remaining phases

### Phase 1 — capability evaluator, CLI and package launcher

- [x] Add a single evaluator consumed by capability listing and dispatch authorization; include policy and capability revisions.
- [x] Add side-effect-free Rust CLI parsing/help/schema, invalid-flag exit code 2, and conservative JSON doctor report.
- [x] Add a package-relative launcher and host/architecture-bound npm archive with Apache license and NOTICE; clean-prefix install with spaces and restricted PATH verified on macOS arm64.
- [x] Add direct-only, bridge-only, unconfigured, denied, revoked, bounded dispatch/revocation, global/subcommand open-stdin help, doctor health-separation and launcher tests.
- [ ] Live transport/process/authentication round trip and client/browser dispatch remain unavailable: no service or browser runtime exists yet.

The first Phase 1 Sol review was not approved; all reported findings were fixed and Sol approved the local/fixture gate on 2026-10-06. After the hosted CI fixes, local checks pass with 36 workspace tests. Red-first execution evidence was not recorded, so it is not claimed. Phase 1 evidence and remaining platform/live boundaries are in `verification-matrix.md` and `blockers.md`. Phase 2–12 remain unstarted; no live browser capability is implied.
