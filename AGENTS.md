# Repository instructions

- Keep this project limited to Chrome and web apps running in Chrome.
- Treat `docs/design/` as proposals; claim support only after the matching evidence gate passes.
- Preserve Comptrol Apache-2.0 attribution for extracted files and update `provenance/extraction.json` when they change or new source is imported.
- Keep Rust at the version in `rust-toolchain.toml`; keep direct dependency versions exact and commit `Cargo.lock` changes.
- Do not add monolithic Comptrol, native desktop automation, credentials, browser profiles, or unrelated assets to the workspace/package.
- Update `docs/progress.md`, `docs/verification-matrix.md`, and `docs/blockers.md` at phase gates. Retain failed evidence.

## Efficient Controlla browser tasks

- Check the Controlla tool surface once. If it is not registered in the client, use one persistent stdio MCP process for the whole browser task; do not launch a new process for each step.
- Keep one shared session: discover once, pair only the explicitly selected/in-scope tab IDs from that fresh snapshot, accept with bounded polling, perform the task, verify once, and release in cleanup.
- For a new foreground tab, use `shared_tab` `open` with the task URL when the accepted session is ready; it returns the new tab ID and document identity. Reuse that tab/session for the remaining work.
- Prefer one narrow `shared_observe` read. If the page hits its scan limit, switch once to a targeted `shared_accessibility` query instead of trying a sequence of broader selectors.
- `shared_input` click requires a unique target, exact current `innerText`, and a unique postcondition selector. If preflight reports `stale_value` or `*_ambiguous`, refresh evidence once and correct only the mismatched input; never guess repeatedly. If dispatch may have happened, inspect the current tab/document before retrying.
- After a click, verify the new document in the same session. Report an observed navigation as such; do not claim the click caused it unless the action result establishes that. If the requested destination is what matters and click preflight still fails, navigate only to a URL read from the page and state that navigation replaced the click.
