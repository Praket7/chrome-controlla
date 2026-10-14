# Controlla live mode and action speed audit

Date: 2026-10-13
Checkout: `/Users/pcg/Documents/Codex/2026-10-12/controlla 4`
Branch / base revision: `codex/controlla-2-stage3` / `1856b6dd470d1c6fffa3fc4ee134404d802e7e8a`
Live MCP: `chrome-controlla`, V3 six-tool surface (`browser`, `snapshot`, `act`, `workflow`, `extract`, `verify`), schema fingerprint `5d8576cf0ec3a49dc8f4e862aa56c0dad7f7b1188d2b2805fdf1587989a70cb4`.

## Measured browser actions

Measurements used a local disposable page served from `127.0.0.1:18765`, not an authenticated user site. MCP elapsed times include tool transport; they are not prompt-send-to-final-answer measurements.

| Mode / action | Result | Wall time | Browser / CDP / host work |
|---|---|---:|---|
| Background open local page | Opened without activating the window | 270 ms | 2 / 2 / 2 |
| Background scoped snapshot | Passed | 40 ms | bounded read |
| Background click link with navigation postcondition | before fix: `unknown`; after fix: immediate `not_dispatched` capability result | prior: 3,043 ms; fixed path avoids deadline wait | inactive-tab pointer input is not supported without activation |
| Background click button with text postcondition | before fix: `unknown`; after fix: immediate `not_dispatched` capability result | prior: 5,047 ms; fixed path avoids deadline wait | pointer input is not dispatched |
| Background fill text field | Passed, independent value readback | 28 ms | — |
| Background rich-editor type | `not_dispatched`, `stale_value`; editor unchanged | 28 ms | — |
| Foreground open local page | Opened and activated | 387 ms | 2 / 2 |
| Foreground scoped snapshot | Passed | 23 ms | — |
| Foreground click button with text postcondition | Passed | 42 ms | — |
| Foreground fill text field | Passed | 27 ms | — |
| Foreground rich-editor type, 17 chars | Passed; independent readback | 161 ms | 51 key events, 109 CDP commands, no intentional delay |
| Four-step workflow: snapshot, locate, fill, verify | Passed as one external call | 58 ms outer / 48 ms runtime | 15 CDP commands, 13 host round trips |
| Release background debugger attachment | Passed; did not close its tab | 6 ms | — |

The fastest demonstrated user-action path was a single bounded workflow call with local verification. Separate snapshot/action calls add model orchestration and force fresh references. The snapshot reference contract is single-use, so the agent should group dependent actions into one workflow when safe and use a new snapshot after independent actions or any uncertain outcome.

## Headless and foreground/background checks

- The installed-Chrome browser package suite passed serially, including isolated real-Chrome fixtures. The dedicated headless stress suite passed 8 isolated launch/evaluate/cleanup cycles in 4.07 seconds and did not show a Keychain prompt.
- The live ESPN headless smoke passed in a prior run in 2.13 seconds using the test helper's explicit Chrome executable.
- The reloaded MCP's direct headless launch previously failed because `COMPTROL_CHROME_EXECUTABLE` was missing. The launcher now exports the installed Chrome executable when available; its shell syntax check passes. The running MCP still needs a reload before the new environment is active.
- Foreground live MCP actions on the local fixture worked. Background open correctly kept `tab_active=false`; background clicks previously consumed their full timeout and now fail quickly and explicitly. Background fill worked; rich-editor typing remains safely rejected with `stale_value` until focus/selection/value can be rebound inside the guarded action.
- The configured browser tool has no close-tab action. `release` detaches Controlla's debugger and leaves the tab open. No unrelated tab was closed. The local HTTP fixture server was stopped after the run.
- No real Google Slides edit or live slide movement was attempted; that would modify account content. Slides plan logic is covered by local tests. “Slides” are not a browser primitive exposed by the current Controlla action surface.

## Additional failures found by ignored real-Chrome suites

These are reproducible failures and block a clean all-mode qualification:

1. FIXED: `shared_snapshot_and_click_real_chrome_regressions` exposed a guard JavaScript binding/order error and a rejection path that could scroll before refusing. The guard now checks trusted/masked boundaries before scrolling and validates the post-scroll viewport at dispatch. Isolated real-Chrome regression passes.
2. FIXED: private headless workflows failed because read-only task owners were recorded as lease holders despite never acquiring a lease. The task path now tracks only real mutation leases. Both isolated real-Chrome private-headless cases pass.
3. An earlier parallel run of the browser ignored suite timed out a virtualized extraction fixture (8/42 records); the same test passed alone, and the full package passed serially. Real Chrome suites should run serially to avoid resource contention.
4. `npm run check:client-hosts` exits 2 because Claude Code (`claude`) is not installed. Its report correctly leaves other live client qualification unrun or manual; this is an environment dependency, not a source test failure.

## Broad gate results

- `cargo test --workspace --locked`: PASS (the regular suite; intentionally ignored live/private-headless tests are covered separately above).
- `cargo fmt --all -- --check`, `git diff --check`: PASS.
- `npm run check:docs`, `check:provenance`, `check:clients`, `check:apps`, `check:bench`, `check:agent-ux`, `check:release`, and `check:v3`: PASS. Release gate continues to disable a “best Chrome” claim absent measured paired results.
- `npm run check:client-hosts`: FAIL / environment missing Claude Code executable.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: PASS.

## Fixes applied and remaining gates

1. FIXED: mutation-lease ownership is limited to acquired mutation leases; private headless real-Chrome tests pass.
2. FIXED: denied guarded clicks check policy before scrolling; the viewport fingerprint is bound to accepted dispatch coordinates; isolated real-Chrome regression passes.
3. APPLIED, RELOAD GATE: launcher now supplies `COMPTROL_CHROME_EXECUTABLE` from the installed Chrome path. Verify direct MCP headless launch after reloading the MCP.
4. FIXED: inactive-tab pointer actions return immediate `not_dispatched` rather than waiting or claiming success. Background fill remains supported only with independent readback; rich-editor typing still refuses with `stale_value` until safe in-action reacquisition is implemented and qualified.
5. DOCUMENTED: `release` detaches Controlla and leaves tabs open; there is no close-tab action in the current tool surface.
6. EXISTING: prompt-send trace instrumentation is present in client-host adapters and records client-boundary spans. Instrumentation does not qualify real clients by itself.
7. Remaining external gates: Claude Code is not installed; six-client matched live traces and paired competitor prompt-to-outcome measurements have not been run. No superiority claim is supported.

## Working tree note

Production fixes from this audit are in the current worktree. The shared-page regression assertion counts exact duplicate labels while permitting bounded intent-ranked tail items; this matches current ranking semantics. This report does not claim all-mode success or comparative speed superiority.

## Post-rebuild verification — 2026-10-13

- Built the configured release runtime with `cargo build --locked --release -p controlla-runtime --bins`; Clippy and formatting checks passed.
- Spawned the configured `/Users/pcg/.chrome-controlla/codex-mcp-launcher` directly, completed MCP initialize and `tools/list`, and confirmed exactly the six expected tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`.
- Using that same launcher, issued a private headless `browser(action="launch", mode="headless", url="about:blank")`; it reported `provider=dedicated_headless` and released the session successfully. This verifies the launcher exports the Chrome executable and the rebuilt server can launch headless Chrome. Codex's already-running MCP process still requires reload to adopt the rebuilt process.
- Re-ran the regular workspace tests and the ignored real-Chrome private-headless workflow/download, shared-click, headless stress, and serial browser-package suites. All completed successfully. Chrome emitted non-fatal macOS display/GPU diagnostics during isolated headless runs; no Keychain dialog appeared.
- All `npm run check:*` gates passed except `check:client-hosts`, which exits 2 because Claude Code is not installed. Codex and OpenCode host adapters are available but unqualified; ChatGPT/Freebuff require interactive host adapters. The release gate remains fail-closed without paired prompt-to-outcome competitor evidence.
- Fresh review also identified a time-of-check gap between target validation and input dispatch. The click path now returns its initial guard receipt, checks document/viewport identity, runs the exact-target/hit-test guard as its last CDP read, enforces the same viewport, and dispatches those final coordinates immediately afterward. A unit test checks final guard receipt validation; isolated real-Chrome click regression passes.
- Final post-fix gates: workspace tests PASS; Clippy PASS; formatting and diff checks PASS; docs/client-config/Agent UX/release/V3 gates PASS; private-headless workflow/download tests PASS; final-revision live ESPN headless newest-story smoke PASS in 2.17 s; configured launcher MCP initialize/tools/list and private headless launch/release PASS.
- Fresh Luna review confirmed the two earlier fixes and found no additional actionable defect in the final click ordering. A minimal browser change can still happen between the final browser-side guard response and Chrome's separate input command; there is no intervening read or work in that interval, and the guard's coordinates are dispatched directly.
