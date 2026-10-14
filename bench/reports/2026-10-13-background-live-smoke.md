# Controlla Background Live Smoke — 2026-10-13

## Run identity

- Checkout: `/Users/pcg/Documents/Codex/2026-10-12/controlla 4`
- Branch and source commit: `codex/controlla-2-stage3` / `1856b6dd470d1c6fffa3fc4ee134404d802e7e8a`
- MCP: `chrome-controlla`, rebuilt V3 runtime; tools listed as `browser`, `snapshot`, `act`, `workflow`, `extract`, `verify`
- Browser schema fingerprint: `5d8576cf0ec3a49dc8f4e862aa56c0dad7f7b1188d2b2805fdf1587989a70cb4`
- Isolation: a Controlla-created **background** tab in the existing Chrome profile. The browser description reported `may_activate_window: false`; no foreground browser action was issued.

## Live site smoke

Target: `https://www.espn.com/nfl/team/_/name/buf/buffalo-bills`

| Check | Result | Evidence |
|---|---|---|
| Open direct Bills destination in background | **PASS** | Controlla returned `mode=background`, `created_by_controlla=true`, the requested URL, and a new tab ID. Elapsed 886 ms; 2 browser commands, 2 CDP commands, 2 host round trips. |
| Read page state | **PARTIAL** | Snapshot returned the Bills page title and URL in 39.9 ms after scanning 2,303 DOM nodes. It reported `complete=false`, `scope_complete=false`, `goal_satisfied=false`, and `cross_origin_frames_unread`; 3 cross-origin frames were unread. The requested link-ranked candidate list was empty (`items=[]`, `total_candidates=0`). |
| Select and open the newest story | **STOPPED SAFELY** | Visible page text contained relative ages, but no candidate with both an observed real `href` and structured publication timestamp was returned. The story click was not dispatched and no mutation was attempted. |
| Release session | **PASS** | Controlla returned `released=true` and released its debugger attachment. The test tab remains open in the background; no user tab was closed. |

The first snapshot request was rejected before browser dispatch because its requested text limit exceeded the tool's bound. One corrected bounded request ran successfully; this is not counted as a browser action.

## Full paired Phase 11 matrix

**BLOCKED BEFORE SITE DISPATCH.** The command attempted was:

```sh
npm run bench:phase11:live-sites -- --baseline=controlla-1
```

The runner stopped with `PHASE11_CONTROLLA_COMMAND_JSON is required and must be a JSON argv array`. A preflight confirmed these required settings are absent: `PHASE11_CONTROLLA_COMMAND_JSON`, `PHASE11_CONTROLLA_1_COMMAND_JSON`, `PHASE11_RESET_COMMAND_JSON`, `PHASE11_VERIFIER_COMMAND_JSON`, and `PHASE11_ENVIRONMENT_JSON`. Therefore no paired rows or Phase 11 result JSON were produced; no result was fabricated from the single-site smoke.

The client-host preflight also returned `qualification_status=not_run`: Codex and OpenCode executables are available but have no live qualification record; Claude Code is missing; Freebuff is interactive-only; ChatGPT Desktop requires a manual host; generic MCP is protocol-only. These checks did not launch client applications.

## Scope and limitations

- This records one live, read-only ESPN background smoke, not the 19-task Phase 11 site matrix or six-client qualification.
- No latest-story click, account action, form submission, download, or page write occurred.
- Prompt-send/model-turn telemetry and paired latency distributions were not available from this smoke; its open and snapshot timings are MCP/browser timings only.
- No competitor comparison or broad speed claim is supported by this run.

## Post-reload live ESPN background run — 2026-10-14

The configured Controlla MCP opened the Bills destination in a background tab in **747 ms** (2 browser commands, 2 CDP commands, 2 host round trips). The extension browser surface remained in the background; no foreground tab was selected or closed.

A single five-step workflow then completed in **1.431 s**: bounded story snapshot, link collection, date selection, background navigation, and independent URL verification. The snapshot returned three first-party Bills story links with real URLs and visible `3h`, `3h`, and `7h` labels. It reported `first_party_complete=true` and `scope_complete=true`; unread cross-origin embeds remained separately reported in global coverage.

Controlla enriched the story links from linked article metadata and selected **“Bills' Brady warns RB Cook of 'consequences' for costly fumbles”**, with `datePublished=2026-10-13T21:49:55Z`. Background navigation reached `https://www.espn.com/nfl/story/_/id/50177135/bills-brady-warns-rb-cook-consequences-costly-fumbles`; the workflow verified the observed URL. The session was released without closing the test tab.

The extension details screenshot supplied before this run showed the unpacked extension loaded from the older `work/chrome-controlla` checkout, while MCP/native host use Controlla 4. The task succeeded through that currently connected extension and the Controlla 4 runtime. This run proves the browser path works, but it does not prove that the extension-side source changes unique to Controlla 4 were loaded from the Controlla 4 folder.
