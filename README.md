# Chrome Controlla

Chrome Controlla is a Chrome-only browser MCP. The current V3 server exposes six tools—`browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`—and uses the Chrome native-messaging bridge to control explicitly selected tabs or open a foreground tab. Its bounded workflow runner groups known multi-step tasks into one MCP call and returns action, verification, and timing evidence. Read the [installation and agent guide](docs/quickstart.md) first; the full tool contract is in the [browser-control skill](packages/chrome-controlla/plugin/skills/browser-control/SKILL.md). The extension/native host pair must use the exact same extension ID. See [`docs/verification-matrix.md`](docs/verification-matrix.md) for test results and what has not been qualified.

The initial extraction retained the upstream browser crate, renamed `controlla-browser`, plus the four controlling design documents. Source provenance and exact imported paths are recorded in [`provenance/extraction.json`](provenance/extraction.json). For the shortest first-time install, start with [`docs/quickstart.md`](docs/quickstart.md); full client setup is in [`docs/clients.md`](docs/clients.md).

## Build

Install Rust 1.99.0 and Node.js 24.19.0. The reference runner is pinned to Playwright 1.63.0; the planned MCP SDK baseline is Rust `rmcp` 3.5.1 with protocol version `2026-07-28`. Chrome for Testing baseline: 154.0.8037.92. These choices pin tools for implementation; they do not qualify a client, browser mode, or app workflow. Then run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
./scripts/check-dependencies.sh
./scripts/package-check.sh
npm run check:docs
```

Build the local executable with `./scripts/package-build.sh`; `./scripts/package-check.sh` packs it, installs the archive into a clean temporary prefix whose path contains spaces, and runs the installed command. The npm launcher resolves only its package-sibling platform binary and does not invoke global Comptrol. The package includes the MCP server and shared-tab bridge; setup is documented in [`docs/clients.md`](docs/clients.md).

Each staged npm archive is restricted to its build host's OS and CPU architecture. Build and verify a separate archive on every release-matrix cell; there is no universal or cross-compiled archive. The current local package evidence covers macOS arm64 only.

Doctor reads the strict configuration shape in [`schemas/doctor-config.json`](schemas/doctor-config.json) from `<state-dir>/runtime.json` (default: `~/.chrome-controlla/runtime.json`; override with `--state-dir DIR` or `CONTROLLA_STATE_DIR`). It probes the configured local PID and loopback-only authenticated `/health` endpoint; heartbeat state, process liveness, transport, authentication and round-trip are separate. The bearer token is never printed, and the returned principal is shown only as a short SHA-256 correlation. Health fixture tests use a local mock service; doctor health checks do not qualify browser attachment or actions. On operating systems without the local PID probe, doctor reports process state as unknown.
