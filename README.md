# Chrome Controlla

Chrome Controlla is a separate Chrome-only project derived selectively from Comptrol. This repository is at **Phase 0: source extraction and build baseline**. It is not yet a usable browser MCP, and no client, app, mode, or performance claim has been qualified.

The initial workspace contains only the upstream browser crate, renamed `controlla-browser`, plus the four controlling design documents. Source provenance and exact imported paths are recorded in [`provenance/extraction.json`](provenance/extraction.json). See [`docs/progress.md`](docs/progress.md) for phase status.

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

These checks cover the extracted crate only; they do not establish live Chrome behavior.
