# Repository instructions

- Keep this project limited to Chrome and web apps running in Chrome.
- Treat `docs/design/` as proposals; claim support only after the matching evidence gate passes.
- Preserve Comptrol Apache-2.0 attribution for extracted files and update `provenance/extraction.json` when they change or new source is imported.
- Keep Rust at the version in `rust-toolchain.toml`; keep direct dependency versions exact and commit `Cargo.lock` changes.
- Do not add monolithic Comptrol, native desktop automation, credentials, browser profiles, or unrelated assets to the workspace/package.
- Update `docs/progress.md`, `docs/verification-matrix.md`, and `docs/blockers.md` at phase gates. Retain failed evidence.
