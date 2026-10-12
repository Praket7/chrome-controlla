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
2. For a foreground task, call `browser` action `open` with `mode:"foreground"`, an HTTP(S) `url`, and optional bounded `steps`. An accepted `session_id` reuses its session. In `local_unrestricted` mode, omit `session_id` for one-call pairing and opening. In guarded mode, first discover tabs and pass that snapshot's `host_id` and selected decimal `target_ids` with the cold `open` call.
3. Use the returned `tab_id` as `chrome_tab_id` for follow-up tools. Prefer one `workflow` or `browser open` with `steps` over individual calls. A click receipt of `unknown` or `dispatched` needs a read-only `verify`; the workflow can perform it as the next step without replaying the click. Use an exact URL postcondition when the URL matters, or `url_starts_with` for an expected canonical redirect.
4. `not_dispatched`: correct the stated cause; take one fresh snapshot if stale. `unknown`: inspect fresh state or run the declared read-only verification; do not replay the click. If still unclear, report exactly what is missing.
5. Navigate only to user-provided or observed URLs. Never invent app routes, account indexes, headings, selectors, or expected text. Verify account identity from page evidence.
6. Loading, a 404, hidden text, empty output, or truncated coverage cannot establish an empty to-do list. Report only observed content and missing coverage.
7. Release the shared session in cleanup; preserve the user's tabs.

For a fast common task, use this compact route: one scoped snapshot → one bounded workflow → one independent verification only where the action receipt does not already prove the declared postcondition. For newest/latest selection, collect the complete candidate set and choose by parsed `datetime`; do not infer order from page position. A click outcome `{kind:"navigation"}` without `url` means any document change. `url` means exact absolute HTTP(S) equality; use a navigation-only click followed by `verify` with `url_starts_with` for a canonical redirect. Never give a relative path as an exact URL. For exact text, omit `typing_mode` so Controlla picks block insertion for ordinary text, fast key events where trusted keys are required, and IME for composition-sensitive text; use `human_keys` only for an explicit human-paced request. In `local_unrestricted` shared sessions, register one explicit artifact and use `workflow`'s visible-trigger upload with a bounded application-acceptance predicate. Guarded uploads use the dedicated headless task route. Continue to posting only after the receipt says `status:"verified"` and `application_accepted:true`. A selected file alone is not app acceptance; inspect once after `unknown`, never replay the upload or post, and release without closing the tab. Measure first-call-to-verified-result wall time and per-step `elapsed_ms`; keep read output scoped to the next decision.

If `browser` action `describe` reports `interaction_policy:"local_unrestricted"`, cold `open` can choose a stable web tab from the fresh native inventory, and click and type dispatch do not wait for a verified outcome. Report `dispatched` as unverified until a separate observation passes. The direct local MCP can enable this temporary mode with `CONTROLLA_UNRESTRICTED=1`.

V3 `act` supports click, fill, type, press, select, and page tools. V3 `browser` task `research` navigates an existing tab rather than opening one. In guarded mode, `upload` and artifact registration require a dedicated private headless session. Local unrestricted mode permits staging a file and selecting it through an existing shared Chrome file input; confirm any subsequent app post separately.

Only when registered/Hotload tools are unavailable, use the packaged `controlla-client` for one persistent process. It initializes and caches schemas once; request one full schema by tool name. Do not improvise another connection wrapper or change state directories to evade ownership errors. See [client setup](docs/clients.md#maintained-standalone-task-client).

Never pass passwords as Controlla tool values. Snapshots expose visible password controls only as redacted `credential_input` references with `raw_value:null`; have the user enter/autofill the password, wait for confirmation, then take a fresh snapshot before submitting. After MCP reload, verify `browser` exposes `idempotency_key` for `open` and the tagged `task` variants; if live schema and runtime disagree, stop and fix/reload the MCP pair rather than probing arguments.
