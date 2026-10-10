# Repository instructions

- Keep this project limited to Chrome and web apps running in Chrome.
- Treat `docs/design/` as proposals; claim support only after the matching evidence gate passes.
- Preserve Comptrol Apache-2.0 attribution for extracted files and update `provenance/extraction.json` when they change or new source is imported.
- Keep Rust at the version in `rust-toolchain.toml`; keep direct dependency versions exact and commit `Cargo.lock` changes.
- Do not add monolithic Comptrol, native desktop automation, credentials, browser profiles, or unrelated assets to the workspace/package.
- Update `docs/progress.md`, `docs/verification-matrix.md`, and `docs/blockers.md` at phase gates. Retain failed evidence.

## Efficient Controlla browser tasks

Follow the [current V3 browser-control skill](packages/chrome-controlla/plugin/skills/browser-control/SKILL.md). The older [master guide](docs/MASTER_GUIDE.md) documents the legacy router and is not the six-tool V3 contract. This is the default flow:

1. Find the directly registered `chrome-controlla` V3 MCP once and cache the schemas of the six exposed tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. Never guess argument names or ID types. A client restart means reload schemas.
2. Reuse one accepted session. Otherwise call `browser` action `discover_shared_tabs`, then `pair_shared` with the returned `host_id` and only the user's selected decimal tab IDs, then poll `accept_shared`. To open a requested URL in a new active tab, call `browser` action `open` with `mode:"foreground"`, the accepted `session_id`, and the HTTP(S) `url`.
3. Use the `tab_id` returned by `browser` action `open` as `chrome_tab_id` in `snapshot` and `act`. Click a returned `reference` with `act` action `click` and a meaningful `outcome`. Use the returned fresh state for the next action. No separate verification read when the result already answers the question.
4. `not_dispatched`: correct the stated cause; take one fresh snapshot if stale. `unknown`: inspect the returned state; do not replay the click. If still unclear, stop and report exactly what is missing.
5. Navigate only to user-provided or observed URLs. Never invent app routes, account indexes, headings, selectors, or expected text. Verify account identity from page evidence.
6. Loading, a 404, hidden text, empty output, or truncated coverage cannot establish an empty to-do list. Report only observed content and missing coverage.
7. Release the shared session in cleanup; preserve the user's tabs.

If `browser` action `describe` reports `interaction_policy:"local_unrestricted"`, `pair_shared` accepts numeric `target_ids` without a discovery `host_id`, and click and type dispatch do not wait for a verified outcome. Report `dispatched` as unverified. The direct local MCP can enable this temporary mode with `CONTROLLA_UNRESTRICTED=1`.

V3 `act` supports click, fill, type, press, select, and page tools. V3 `browser` task `research` navigates an existing tab rather than opening one. In guarded mode, `upload` and artifact registration require a dedicated private headless session. Local unrestricted mode permits staging a file and selecting it through an existing shared Chrome file input; confirm any subsequent app post separately.

Only when registered/Hotload tools are unavailable, use the packaged `controlla-client` for one persistent process. It initializes and caches schemas once; request one full schema by tool name. Do not improvise another connection wrapper or change state directories to evade ownership errors. See [client setup](docs/clients.md#maintained-standalone-task-client).
