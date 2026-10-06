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

Phase 0 establishes the repository only. Every CC/B item remains unimplemented until its owning phase supplies evidence. Corrective docs/package/dependency checks pass on pinned Node 24.19.0/npm 11.17.0; Sol approved the corrective review and final CI npm pin. Node 26.7.0/npm 11.19.0 were observed in the login shell but were not used to claim pinned-runtime verification. Hosted three-OS CI remains pending the first push.

| ID | Status | Owning phase | Evidence |
|---|---|---:|---|
| CC-01 | unimplemented | 1 | none |
| CC-02 | unimplemented | 1 | none |
| CC-03 | unimplemented | 1 | none |
| CC-04 | unimplemented | 1 | none |
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
| B01 | unimplemented | 1 | none |
| B02 | unimplemented | 1 | none |
| B03 | unimplemented | 1 | none |
| B04 | unimplemented | 1 | none |
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
