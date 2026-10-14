# Controlla Full Audit — ESPN Background Failure

Date: 2026-10-13
Checkout: `/Users/pcg/Documents/Codex/2026-10-12/controlla 4`
Branch / commit: `codex/controlla-2-stage3` / `1856b6dd470d1c6fffa3fc4ee134404d802e7e8a`
Scope: configured Codex MCP, native bridge/extension runtime, background ESPN task path, the candidate extraction and freshness-selection code, repository CI gates, headless browser regressions, and the release qualification boundary.

## Executive diagnosis (updated after fixes)

The background open worked. The story click did not happen because the live run had no candidate satisfying Controlla's evidence requirements. The current configured V3 MCP is not an old tool surface: it exposed the six V3 tools (`browser`, `snapshot`, `act`, `workflow`, `extract`, `verify`), and its browser description reported background mode with `may_activate_window:false`, `requires_visible_window:true`, and the real Chrome profile. Its schema fingerprint was `5d8576cf0ec3a49dc8f4e862aa56c0dad7f7b1188d2b2805fdf1587989a70cb4`.

There are two confirmed hot-path defects and one conservative coverage interaction:

1. `goal.query` is described as task intent for ranking, but the page-side implementation uses the entire normalized query as a literal substring and drops every nonmatching item before ranking. On this live page, `"Bills"` produced two Bills-related story links; `"story"` and `"Buffalo Bills story headlines with publication dates"` produced zero links. The natural-language query therefore removed the useful targets.
2. Omitting the query returns links in DOM order. ESPN puts the scoreboard ahead of the news cards, and the bounded payload was consumed by scoreboard entries (`total_candidates=38`, `coverage=partial: max_bytes`), so the news links were starved out.
3. Even the `"Bills"` query returned story links without inline `datetime`/`datetime_utc_ms`. The workflow can fetch same-origin linked article JSON-LD, but `choose_newest_with_bounds` requires complete candidate coverage. Unread cross-origin frames elsewhere on the ESPN page make snapshot coverage incomplete, so the selector fails closed and does not dispatch a click.

This is why the result looked like the previous Controlla: the background-tab capability and MCP wiring were active, but the live site fell through a brittle observation contract and conservative freshness gate. The fixes and follow-up live result are recorded below.

## Live evidence

The background-open receipt for the Bills team URL reported success, a new Controlla-created background tab, no activation, 886 ms, 2 browser commands, 2 CDP commands, and 2 host round trips. The page title and URL matched ESPN's Buffalo Bills page.

On a fresh read of that isolated background tab:

| Snapshot request | Result |
|---|---|
| Link role, exhaustive, no text query | 38 candidates; scoreboard/Gamecast/Box Score links dominate; `coverage=partial: max_bytes`; no article timestamps in the returned candidates |
| Query `story`, role `link` | 0 candidates |
| Query `Bills`, role `link` | 2 ESPN story links, both with `datetime=null` and no timestamp precision |
| Query `Buffalo Bills story headlines with publication dates`, role `link` | 0 candidates |

The two observed article URLs were:

- `https://www.espn.com/nfl/story/_/id/50134142/bills-qb-josh-allen-hollywood-star-endorsements`
- `https://www.espn.com/espn/betting/story/_/id/47809892/2026-super-bowl-lxi-odds-nfl-sports-betting-season-win-totals-playoffs`

The article candidates had real `href` values, but the listing had no structured publication timestamps. `complete=false` / `scope_complete=false` and unread frame coverage prevented proving which link was newest. No click was issued. The background session was released, leaving the isolated test tab open; no foreground tab was activated or closed.

## Findings, ranked by impact

### P0 — Natural-language query is a hard literal filter

In [`snapshot.js`](../../crates/controlla-runtime/src/shared_page/snapshot.js#L169), `queryScore` only gives a match when the normalized candidate label/value equals or contains the entire normalized `goal.query`; line 176 discards nonmatches. This contradicts the public schema's description of `query` as task intent used to rank returned page content and the runtime instruction to use a visible query. A task-intent query with extra words can exclude every target before ranking.

**Concrete case:** `Bills` returned two story candidates; `Buffalo Bills story headlines with publication dates` returned none.
**Consequence:** no target reference or `href` reaches workflow selection, so there is no click.
**Required fix:** tokenize and score relevant evidence terms, preserve a bounded low-relevance tail, and apply exact text only when the caller asks for an exact label. Keep candidate inclusion separate from a completeness/uniqueness claim. Add a live-DOM-like regression where a natural-language intent surfaces a link whose title shares only some query terms.

### P0 — Bounded unfiltered scan is order-biased and starves article links

The shared snapshot walker emits candidates in DOM order and stops when the serialized byte budget is reached. With no query, the page's scoreboard links fill the bounded result before the Bills news section is observed. The wrapper's later Rust relevance ordering cannot recover candidates that the JavaScript walker never returned.

**Concrete case:** role-only snapshot returned 38 candidates and `coverage=partial: max_bytes`; the returned link candidates were dominated by NFL scoreboard entries.
**Consequence:** the fastest bounded call is not useful for this page; it is incomplete before article freshness can be considered.
**Required fix:** make page-side collection relevance-aware and/or scope discovery to the relevant Bills news region before consuming the response budget. Return explicit omitted-candidate evidence; do not claim exhaustive newest selection from a truncated pool.

### P1 — Freshness recovery and completeness are coupled to unrelated frames

The workflow has a real same-origin linked-date enrichment path, capped at 12 URLs and 8 seconds. The newest chooser then refuses to return a choice unless `coverage_complete` is true. ESPN's cross-origin embeds make the page snapshot incomplete even when they are unrelated to the story list.

**Concrete case:** the no-role snapshot scanned 2,372 nodes in 19.6 ms, but reported `cross_origin_frames_unread=4`, `complete=false`, and `scope_complete=false`; the targeted query similarly reported incomplete frame coverage.
**Consequence:** candidate discovery and article-date enrichment may be correct locally, yet the global completeness flag prevents clicking because the page contains unrelated opaque frames.
**Required fix:** track completeness at the requested semantic scope (e.g. the bounded Bills news collection), and distinguish unread unrelated frames from unread candidate-bearing scopes. Keep fail-closed behavior when coverage inside the target story collection is uncertain.

### P1 — Current story regression does not qualify the live ESPN markup path

The local `espn_latest_story` tests use repository-owned fixture HTML with curated story `datetime` values or predictable same-origin article pages. All five pass, including verified navigation and linked JSON-LD enrichment, but the live ESPN listing observed here has different evidence: story candidates have no inline timestamps and the whole snapshot has unread frame coverage. The test proves the helper logic on fixtures; it does not prove that live ESPN can supply a complete story collection within the current snapshot budget.

**Required fix:** add an archived representative live-page DOM fixture with scoreboard-before-news ordering, missing listing timestamps, cross-origin ad/video frames, and same-origin article metadata; assert both bounded candidate retrieval and a correctly guarded selection. Then run the configured real-site qualification matrix with reset/readback adapters.

### P2 — No evidence supports a fastest/most reliable claim yet

The source includes latency/cost routing, compact tools, one-call task compilation, and release gates. However, the local Phase 11 live-site runner is blocked before site dispatch because `PHASE11_CONTROLLA_COMMAND_JSON`, `PHASE11_CONTROLLA_1_COMMAND_JSON`, `PHASE11_RESET_COMMAND_JSON`, `PHASE11_VERIFIER_COMMAND_JSON`, and `PHASE11_ENVIRONMENT_JSON` are not configured. Client qualification is `not_run` for Codex/OpenCode; other listed clients are unavailable or require manual setup. There is no matched prompt-send-to-verified-outcome data against the declared baselines.

**Consequence:** code and fixtures pass, but neither real-client qualification nor speed/success superiority has been established.

## Test and gate record

The initial audit ran at the listed base revision before the follow-up fixes. Follow-up source changes and their tests are recorded below and in the current worktree.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test --workspace --locked` | PASS, exit 0; includes 5 ESPN latest-story fixture tests and V3/runtime/client/cost-router suites; six installed-Chrome tests are intentionally ignored in the regular run |
| `cargo test -p controlla-browser --test headless_stress --locked -- --ignored --nocapture` | PASS; 8 isolated headless launch/evaluate/cleanup cycles in 4.87 s; each uses a fresh profile; macOS headless launch uses `--use-mock-keychain`, and no keychain popup interrupted the foreground |
| `cargo test --locked -p controlla-runtime --test espn_latest_story` | PASS, 5/5; local fixture-based browser regression, not a live ESPN click |
| `npm run check:clients` | PASS, 12 tests |
| `npm run check:docs`, `check:provenance`, `check:apps`, `check:bench`, `check:release`, `check:v3`, `check:agent-ux` | PASS; release check explicitly keeps the superiority claim disabled without Phase 11 measurements |
| Extension background test, repeated 8 times | PASS, 8/8 |
| `node extensions/chrome-controlla/test-background.cjs --reconnect-only` | PASS |
| `./scripts/check-dependencies.sh && ./scripts/package-check.sh` | PASS; clean-prefix packed npm install, stdio startup, six-tool discovery, rollback/uninstall checks succeeded for darwin/arm64 |

The headless Chrome runs printed macOS `CVDisplayLinkCreateWithCGDisplay` / GPU mailbox diagnostics but completed successfully. These did not indicate test failure or open a GUI prompt.

## Configuration and safety audit

- Codex MCP registration points to the Controlla launcher and current checkout's release runtime; the extension native-host manifest points to the paired release binary and the configured extension ID. The current V3 tool surface loaded, so this failure is not explained by an old MCP schema or missing bridge registration.
- The extension/service-worker background tests pass repeatedly. The live browser descriptor explicitly says background open does not activate the window; this feature is present and worked in the recorded run.
- The story workflow is fail-closed on missing freshness/coverage evidence and did not blindly click a headline. That safety behavior is correct, but its completeness boundary is too broad for a page with unrelated opaque frames.
- Source tree inventory: 274 tracked/non-target project files across Rust crates, packages, extension, scripts, benchmarks, apps, tests, integrations, and CI. The hot-path audit traced MCP registration, shared snapshot extraction, relevance filtering, linked-date enrichment, newest selection, click verification, browser extension background behavior, and their relevant tests/gates; this was not a manual line-by-line security audit of every unrelated source file.

## Follow-up fixes and verification — 2026-10-13

The audit's three direct execution blockers are fixed in the Controlla 4 source:

1. **Natural-language candidate ranking:** `goal.query` is tokenized into meaningful terms and used to score candidates rather than acting as a full-phrase substring gate. Goal-scoped result admission is relevance-ranked under the byte/item cap so scoreboard links cannot consume the entire result before higher-scoring story links. Tool guidance describes query as task intent and keeps exact matching in explicit find operations.
2. **Story-scope completeness:** explicit story `href_hint` scopes can be complete for the first-party story collection while unrelated cross-origin embeds remain separately reported in global completeness. Missing/omitted candidates still prevent a newest claim.
3. **Newest story and verified click:** the compiled newest-story workflow no longer repeats `Bills` as a hard `find_all` filter, so a newer headline need not contain that entity. If an exact target's page-wide semantic epoch changed before dispatch, workflow performs one fresh snapshot and reacquires the exact link only while the source document remains the same. Pointer guards retain hard document and viewport checks and revalidate target identity, text, visibility, and hit target immediately before dispatch; unrelated page-wide DOM updates do not invalidate an unchanged link. URL verification reads Chrome's frame identity directly, so a just-committed navigation does not depend on a page snapshot getter that may not yet exist.

### Live ESPN result

Ran the ignored `live_espn_headless_latest_story_smoke` against `https://www.espn.com/nfl/team/_/name/buf/buffalo-bills` using an isolated headless Chrome profile. It selected a real ESPN NFL story with linked machine-readable `datePublished` evidence, clicked it, and independently verified the resulting article URL. Final run: **PASS in 2.13 s**; the action receipt reported verified navigation and the URL verifier passed. The browser was isolated/headless and did not activate or interrupt the user's foreground.

The first live run exposed a test-startup race (headless launch returned before the destination committed); the smoke now navigates through the readiness-aware task path. The next live run exposed the page-wide semantic-epoch rejection. A later run exposed URL verification's unnecessary dependence on a page snapshot immediately after navigation. Both runtime defects were fixed; the final run passed. These failures and recoveries are retained here for traceability.

### Post-fix checks

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS after formatting |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test --workspace --locked` | PASS after reducing redundant schema descriptions; strict default tool-schema size contract passes |
| `cargo test -p controlla-runtime --test espn_latest_story live_espn_headless_latest_story_smoke -- --ignored --nocapture` | PASS, isolated live ESPN, 2.13 s |
| `npm run check:clients` | PASS, 12 tests |
| `npm run check:docs`, `check:provenance`, `check:apps`, `check:bench`, `check:release`, `check:v3`, `check:agent-ux` | PASS; the benchmark and release gates still correctly keep superiority claims disabled without paired real-client evidence |

### Remaining qualification boundary

The live simple-task failure in the audit is fixed and verified. The configured six-client Phase 11 matrix, matched Stage 2.5/competitor runs, and prompt-send-to-final-answer p50/p95 are still unverified because reset, environment, and independent-verifier adapters are not configured. The successful ESPN smoke is one isolated live run; it does not establish comparative superiority or all-site reliability. Hosted CI status for the new working-tree revision has not been observed.

## Current remaining qualification boundary

- The ESPN live newest-story workflow passed once in isolated headless Chrome as recorded above; broad live-site reliability is not established by one run.
- The 19-case real-site Phase 11 matrix and six named live-client runs remain unexecuted because the required adapters/reset/verifier/environment configuration is absent. Claude Code is also missing from this host.
- No paired prompt-send-to-verified timing, p50/p95, or competitor comparison was produced, so “fastest” or “best” claims remain disabled.
- Follow-up fixes are implemented and the current workspace test, lint, documentation, client-config, benchmark, Agent UX, release, and V3 gates pass. Current real-Chrome headless, private-workflow, and shared-click regression results are recorded in `2026-10-13-live-mode-speed-audit.md`.
