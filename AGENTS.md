# Repository instructions

- Keep this project limited to Chrome and web apps running in Chrome.
- Treat `docs/design/` as proposals; claim support only after the matching evidence gate passes.
- Preserve Comptrol Apache-2.0 attribution for extracted files and update `provenance/extraction.json` when they change or new source is imported.
- Keep Rust at the version in `rust-toolchain.toml`; keep direct dependency versions exact and commit `Cargo.lock` changes.
- Do not add monolithic Comptrol, native desktop automation, credentials, browser profiles, or unrelated assets to the workspace/package.
- Update `docs/progress.md`, `docs/verification-matrix.md`, and `docs/blockers.md` at phase gates. Retain failed evidence.

## Efficient Controlla browser tasks

Follow [the browser task recipe](docs/MASTER_GUIDE.md#efficient-shared-tab-recipe). This is the default flow:

1. Find Controlla once. Use registered tools; if they are hidden behind Hotload, search only server `chrome-controlla`. Cache the exact schemas for the tools you will call. Hotload calls must include the returned `schemaHash` as `expectedSchemaHash`. Never guess argument names or ID types. Reloaded server means reload schemas.
2. Reuse one accepted session. Otherwise discover once, pair only the user's selected tab IDs from that fresh inventory, and accept with bounded polling. Open a foreground tab with `shared_tab` when requested.
3. Read `shared_snapshot` once. Click its returned `reference` with `shared_click` and a meaningful outcome. Use the returned fresh snapshot for the next action. No separate verification read when the result already answers the question.
4. `not_dispatched`: correct the stated cause; take one fresh snapshot if stale. `unknown`: inspect the returned state; do not replay the click. If still unclear, stop and report exactly what is missing.
5. Navigate only to user-provided or observed URLs. Never invent app routes, account indexes, headings, selectors, or expected text. Verify account identity from page evidence.
6. Loading, a 404, hidden text, empty output, or truncated coverage cannot establish an empty to-do list. Report only observed content and missing coverage.
7. Release the shared session in cleanup; preserve the user's tabs.

Use `shared_observe` for known fields. `max_items` limits results, not scanning depth. `shared_accessibility` returns one selected AX node; it is not a page snapshot. Legacy `shared_input` remains for fill/type and compatibility clicks requiring raw text and a pre-existing changing postcondition. Prefer reference-based `shared_click` for navigation and menus.

Only when registered/Hotload tools are unavailable, use the packaged `controlla-client` for one persistent process. It initializes and caches schemas once; request one full schema by tool name. Do not improvise another connection wrapper or change state directories to evade ownership errors. See [client setup](docs/clients.md#maintained-standalone-task-client).
