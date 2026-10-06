# Phase 0 review 1 — corrective report

Status: original review was not approved. Corrections below are implemented; focused verification and reviewer re-review are pending. This report is append-only across review cycles.

| Finding | Correction |
|---|---|
| Toolchain and reference stack did not pin Node, Playwright, MCP SDK/protocol, or Chrome for Testing | Recorded exact selections and rationale in `docs/decisions.md`; pinned Node/npm/Playwright in `.nvmrc`, `package.json`, and `package-lock.json`; recorded `rmcp` 3.5.1, MCP protocol 2026-07-28 (legacy 2025-11-25 compatibility), and Chrome for Testing 154.0.8037.92 as later-phase reference versions, not live qualification. Added official source links. Planned OS targets are macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64. |
| Provenance omitted adapted manifest, license files, copied specs, and retained tests | Rebuilt `provenance/extraction.json` with SHA-256 source/destination hashes, per-file edits and license status, upstream revision, all 22 retained inline test names, version pins, OS targets, and publication decision. |
| CI omitted package/docs checks; local dependency check skipped Windows; guide links were broken | Added `scripts/package-check.sh` and `scripts/check-doc-links.mjs`; added npm/Node lock install plus docs, package, and dependency checks to CI; dependency and package scripts run under Bash on Windows too; corrected links to the relocated master guide. |
| Progress claimed Phase 0 complete and evidence lacked commit/artifact pointer | Changed progress to in-progress, no-commit, re-review pending. `docs/verification-matrix.md` records commands, environment, timestamps/results, and this evidence file. No commit has been made. |
| GitHub owner/visibility decision missing | Recorded `Praket7/chrome-controlla`, private by default, authorized target selected, creation/publication pending. |

## Verification after corrections

The initial review failed as recorded above. Focused corrective verification is appended below; scoped independent re-review is still pending.

## Phase 0 corrective rerun — 2026-10-06

- Added `/node_modules/` to `.gitignore` and removed its 179 already-staged paths from the index; the local install remains available. `git check-ignore node_modules/playwright/package.json` passed.
- Confirmed `scripts/package-check.sh` already calls `cargo package --workspace --locked --allow-dirty --list`; it succeeds before the first commit and retains the required-file and banned-package checks. No script change was needed.
- Recomputed every source and destination SHA-256 in `provenance/extraction.json` against the current source and destination files. Corrected the `research.md` edit description: its relocated master-guide link changed. The adapted manifest destination hash was stale and is now corrected.
- The interactive login shell selected Node v26.7.0/npm 11.19.0; these were not used for this verification. Explicitly selected the installed pinned Node v24.19.0/npm 11.17.0 (`/Users/pcg/.nvm/versions/node/v24.19.0/bin`) for the checks below.
- Pass: `npm run check:docs` — checked local Markdown links in 11 files.
- Pass: `npm ls --package-lock-only --depth=0` — lockfile resolves the declared Playwright 1.63.0 dependency.
- Pass: `npm ci --ignore-scripts` — added 2 packages; audit reported 0 vulnerabilities.
- Pass: `./scripts/package-check.sh` — listed Cargo package contents; required LICENSE/NOTICE present and no banned package paths.
- Pass: `./scripts/check-dependencies.sh` — no banned Comptrol dependencies found.
- Pass: `git diff --check` — no whitespace errors.
- Node/npm checks ran on macOS 26 / Darwin 25.6 arm64 with Rust 1.99.0 available. These are local source/package checks only; no live Chrome qualification is implied.

### Corrective rerun details

Ran at 2026-10-06 11:25–11:26 UTC on macOS 26 / Darwin 25.6 arm64:

- `/Users/pcg/.cargo/bin/cargo fmt --all -- --check && /Users/pcg/.cargo/bin/cargo clippy --workspace --all-targets --locked -- -D warnings && /Users/pcg/.cargo/bin/cargo test --workspace --locked` — exit 0; 22 tests passed, no failures or doc tests.
- With Node 24.19.0 and npm 11.17.0 on PATH: `npm ci --ignore-scripts && npm run check:docs && npm ls --package-lock-only --depth=0` — exit 0; 2 packages installed, 0 audit vulnerabilities, 11 Markdown files checked, Playwright 1.63.0 resolved.
- With pinned Node 24.19.0: `npm run check:provenance` — exit 0; all 13 destination digests and all 22 retained test names matched the manifest; upstream digests were also verified because the sibling source checkout is available.
- `./scripts/check-dependencies.sh && ./scripts/package-check.sh && git diff --check && git diff --cached --check` — exit 0; no prohibited dependency, package contents include LICENSE and NOTICE plus only the browser crate, and no whitespace errors.

The initial docs-link run exited 1 because the review report referenced its own file before it existed; creating the report made the check pass. The initial package-list run exited 101 because the working tree was uncommitted; the package check now passes with `--allow-dirty` and still checks exact package contents. Both failures and their corrections are retained here.

No Rust CI job or Windows/macOS/Linux hosted CI run has been triggered. The local Rust/package tests are not live Chrome qualification. The build commit remains intentionally pending until scoped review passes.
