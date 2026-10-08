# Chrome Controlla

Chrome Controlla is a Chrome-only browser MCP. Its native-messaging bridge can discover Chrome tabs and pair one shared session with multiple explicitly selected tabs. Pass `discover_shared_tabs.snapshot.host_id` with the chosen `target_ids` to `pair_shared`; omitting `host_id` selects the manual popup fallback. Discovery truncates at 100 eligible tabs and marks this as `snapshot.truncated: true`. Pairing trusts the MCP client that supplies the IDs; the extension's debugger indicator is visible, but separate browser-side approval per tab is not established. Native-route attachment and commands remain unqualified; a separate popup route completed one bounded live read-only observation.

The initial extraction retained the upstream browser crate, renamed `controlla-browser`, plus the four controlling design documents. Source provenance and exact imported paths are recorded in [`provenance/extraction.json`](provenance/extraction.json). See [`docs/progress.md`](docs/progress.md) for phase status and [`docs/clients.md`](docs/clients.md) for local setup, including one-time Chrome bridge registration.

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
