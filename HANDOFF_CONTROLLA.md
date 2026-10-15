# Controlla 4 — New Chat Handoff

**Updated:** 2026-10-15  
**Purpose:** Give the next Codex chat the exact working checkout, reload source, runtime wiring, current uncommitted state, local Stage 1 evidence, limitations, and remaining live qualification.

## Start here

The active source tree for edits is:

`/Users/pcg/Documents/Codex/2026-10-12/controlla 4`

Do not confuse it with older Controlla 3/4 copies or the initial download checkout. Git remote is `https://github.com/Praket7/chrome-controlla.git`. At handoff creation the current branch was `codex/controlla-2-stage3`, and HEAD was `2eb1dc302950f76e3213158ae07a658f6ea7b33f`. Recheck git state before changing files; this checkout has uncommitted work that must be preserved.

**Stage 1 code and wiring are complete locally and have passed the recorded local acceptance gates.** The current turn added the cross-layer implementation, regressions, documentation updates, and this refreshed handoff. No commit or push was requested or performed. The configured extension and Codex MCP were not reloaded because the user reserved those manual live steps.

## Chrome extension source to reload

The unpacked Chrome extension source is:

`/Users/pcg/Documents/Codex/2026-10-12/controlla 4/extensions/chrome-controlla`

This is the source folder to select/keep in Chrome’s Extensions page. The user's recent Chrome screenshot identified the loaded-from path under this same `controlla 4/extensions/chrome-controlla` location. The extension manifest is Manifest V3, name **Chrome Controlla Shared Tab Bridge**, version **0.1.0**.

The exact native-messaging allowed origin is:

`chrome-extension://pdaljlefgpejgjfennalcaidmmkioeda/`

The bridge registration file is:

`/Users/pcg/Library/Application Support/Google/Chrome/NativeMessagingHosts/chrome_controlla_bridge.json`

It points to:

`/Users/pcg/Documents/Codex/2026-10-12/controlla 4/target/release/controlla`

To reload edited extension code: open `chrome://extensions` in the **Praket** Chrome profile, find the unpacked Chrome Controlla Shared Tab Bridge with that ID, and press its Reload button. Keep its loaded-from folder set to the path above. Do not remove/reinstall it unless necessary; preserving the unpacked folder avoids accidentally loading an older copy.

## Configured Codex MCP

Codex config is `/Users/pcg/.codex/config.toml`; its `chrome-controlla` server command is:

`/Users/pcg/.chrome-controlla/codex-mcp-launcher`

That launcher creates a unique `CONTROLLA_STATE_DIR` under `~/.chrome-controlla/codex-clients/client-<pid>`, optionally sets the local Chrome executable, then runs:

`/Users/pcg/Documents/Codex/2026-10-12/controlla 4/target/release/controlla-v2 mcp`

The release binaries existed at handoff time:
- `target/release/controlla` — native bridge executable
- `target/release/controlla-v2` — configured MCP executable

After Rust runtime edits, rebuild the release binaries and restart/reload the Codex MCP process. After extension edits, reload the unpacked Chrome extension. If both layers changed, do both. The MCP tool catalogue observed in Codex exposes six tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, `verify`. Recheck that exact live catalogue after reload; a checkout/build does not prove Codex is using it.

Each concurrent MCP process needs a distinct absolute state directory. Do not reuse a directory owned by a live process.

## Repository state at handoff

The worktree contains uncommitted Stage 1 implementation, test, report, fixture and documentation changes. Preserve them. Do not reset, checkout over, clean, or stash without first inspecting the changes.

### Stage 1 implementation now present

The current worktree completes the Stage 1 local code/wiring scope across:
- a shared runtime/extension capability contract, pre-dispatch validation and exact packaged-asset build identity;
- screenshot payload validation, bounded chunk assembly, SHA-256/PNG integrity, cancellation, and MCP image delivery;
- application upload acceptance evidence distinct from selection and persistence, including an isolated local server fixture that checks exact bytes/hash and ordinary plus `role=status` output;
- event-based navigation readiness and document identity;
- owned target/window lifecycle, observed foreground presentation, safe close/move/activate and mode-inferred release;
- deadline propagation, dispatch uncertainty, cancellation fencing, target-scoped serialization and interference recovery;
- final serialized snapshot byte limits and complete/truncated evidence semantics;
- public route, runtime, extension, package and documentation acceptance coverage.

The dated acceptance ledger is [verification-matrix.md](docs/verification-matrix.md); the item-level Stage 1 plan is [2026-10-14-controlla-stage-1-reliable-contracts.md](docs/superpowers/plans/2026-10-14-controlla-stage-1-reliable-contracts.md). Fresh independent Luna Medium and Luna High reviews concluded **ALL CODE/WIRING COMPLETE LOCALLY**. The first High review found missing screenshot acceptance cases; those were added and passed locally, then approved by a fresh separate High re-review. The plan intentionally leaves exact configured reload and live target/focus/upload/navigation/interference qualification open.

### New files added in the current audit/handoff pass

- `HANDOFF_CONTROLLA.md` — this file.
- `docs/superpowers/plans/2026-10-14-controlla-stage-1-reliable-contracts.md`
- `docs/superpowers/plans/2026-10-14-controlla-stage-2-complete-capabilities.md`
- `docs/superpowers/plans/2026-10-14-controlla-stage-3-speed-and-qualification.md`
- `bench/reports/2026-10-14-capability-audit.json` — sanitized receipts and sample timings; no pairing token.
- `bench/fixtures/capability-audit-20261014/server.py` and `events.json` — local-only browser capability fixture and recorded non-sensitive test events.

The three plans cover Stages 1–3. Stage 1 code/wiring and local gates are complete; the Stage 1 plan marks the remaining reload/live gates separately. Stages 2–3 remain separate future scope. The sanitized audit report and localhost fixture remain in the paths above.

## User goal and constraints for continuing

The user's goal is a highly capable Chrome-use agent with low prompt-to-action and prompt-to-verified-outcome time, high success/accuracy, few model and browser commands, and useful foreground, background and headless operation.

Capabilities specifically emphasized:
- select an exact existing tab or explicitly choose where an owned window/tab is created;
- preserve account/profile and distinguish windows/tabs;
- background work that does not briefly steal focus or activate user tabs;
- visual screenshots of every requested slide, including when another window covers Chrome;
- reliable click, scroll, touch, drag/drop, keys, fast insertion, block typing, human-paced typing, forms and rich editors;
- upload and retrieve files with actual application acceptance evidence;
- sign-in/authorized secure password typing with no secret in receipts/logs;
- preserve concurrent human work, move/rebind targets safely, and support multiple sessions;
- do not report unknown actions as success or blindly replay them.

Do not remove target identity, account checks, mutation guards or policy restrictions merely to make a demo pass. The correct goal is low unnecessary friction with clear, tested capabilities. The long Canva plus Google Classroom assignment prompt in the user's message was an **example target workload**, not authorization to edit that real deck or submit that actual assignment. For audit posts/uploads, the user's explicit exception is **Testing Google Classroom only**. No external post was made in the audit. Do not send/publish elsewhere without the required authorization. Test-created browser tabs must be closed while existing user tabs are preserved.

User prefers direct execution after authorization, evidence-backed completion, and concise status. They have repeatedly asked for comprehensive work rather than repeated partial plans. If asked to implement, follow the three plan documents through completion, update ledgers, run requested checks, rebuild/reload as necessary, and distinguish local tests, installed-extension live results, CI, and competitor evidence. Do not claim “best/fastest” without matched evidence.

## Live MCP audit — exact observations

Audit date: 2026-10-14. The runtime was available through the configured six-tool MCP after reload. Tests used an isolated local fixture on localhost and an existing Canva tab for read-only checks. Individual wall times below are single call samples, **not** actual prompt-SEND-to-result measurements or p50/p95 estimates. The raw-ish sanitized aggregate is in the report path above.

### Confirmed working samples

- Inventory returned existing Chrome tabs and window IDs in about 11 ms. The claim that Controlla cannot identify any windows is too broad; the actual gap is choosing/controlling/placing across them safely and completely.
- Opening an owned background window took about 442 ms, 2 browser commands. The target tab was active inside its own window while the window remained unfocused.
- A mouse click in that owned unfocused window produced a trusted browser event and incremented the local test counter; MCP verified it. About 42 ms, 18 browser commands/17 host round trips.
- Touch tap on a test button produced a trusted event and verified state. About 70 ms, 21 commands/20 round trips. This does not qualify swipe/multitouch.
- Fill worked and verified. About 28 ms/6 commands. Its event was synthetic, appropriate to fill semantics, and is not proof that all typing modes work.
- A headless region screenshot returned an actual image block.
- A headless download completed; the 34-byte test file was verified by SHA-256. A first path was safely rejected because its parent involved a symlink; the canonical `/private/tmp` path passed.
- Existing Canva tab was paired as a background session. The attempted slide select was refused before dispatch as `activation_forbidden`; no slide changed and no temporary tab activation was reported.

### Confirmed defects/limitations

- **Shared screenshot:** runtime sends `captureBeyondViewport:true`; extension validator requires it to be false. Shared probes failed on both Canva and local fixture. The headless probe worked. Fix contract and test the runtime-generated request through the extension.
- **Action arguments:** V3's shared precheck accepts a `value` only for `fill` and `secure_fill`; direct `type`, `select` and `slider` are rejected before their implementations. A workflow with exact find also failed because the unrelated unread cross-origin iframe made the full scope incomplete.
- **Upload:** guarded shared mode accepted a staged artifact handle but extension refused file selection because it required unrestricted local mode. In private headless mode the local server received all 11 test bytes and a trusted file change event, but after 126 browser commands the task receipt was only “selected”; it failed to detect ordinary output/status text (“Accepted …”). Selected, consumed, accepted and persisted must remain distinct evidence levels.
- **Downloads:** shared signed-in download route returns unsupported; headless download works.
- **Navigation:** a research request for `https://example.com` returned unknown after about 7.4 sec/67 commands while canonical `https://example.com/` completed in about 0.92 sec/35 commands. The readiness check uses exact URL equality and Chrome canonicalizes the root slash. Warmth confounded this two-sample timing comparison, but the equality bug is independently visible in source.
- **Simple action command count:** a verified background click used 18 commands/17 round trips. A click on the offscreen fixture button reached the page, but the test first gave the wrong output selector and incorrectly timed out; independent Chrome AX plus server event showed the click succeeded. A later verification attempt also used CSS scope where API expects an opaque reference. These are test/API clarity errors, not click-dispatch failures. The issue still demonstrates why receipts need clear predicate target types.
- **Foreground:** a click through the foreground route verified, but its preceding open receipt said `window_focused:false`; actual foreground presentation is not qualified.
- **Existing inactive Canva:** Controlla refused the click before dispatch. It cannot yet claim arbitrary inactive-tab pointer control.
- **Canva:** semantic read returned seven slide controls but about 128,633 observation bytes, with partial overall coverage. No screenshots were returned by the shared path. Deck stayed on slide 7; slides 1–6 were not reviewed.
- **Model telemetry:** route predicted zero internal model calls but actual count remained null and gate inconclusive. Cannot count this as a proven zero-model path.
- **Release:** background/headless session release failed when mode was omitted; explicit mode released. Session mode should be inferred from session.
- **Background guarantees:** owned-window click and touch showed no window focus, but a single fixture does not prove zero activation across tabs, OS states, minimized/frozen pages, or all sites.

### Audit correction

The bottom-button click was initially checked against the wrong element. Independent Chrome AX showed the adjacent output changed to “Bottom clicked” and the fixture server recorded the event. Do not repeat the erroneous claim that this button failed. The verification interface should still disambiguate CSS selectors from opaque semantic references.

## Research already done

The prior audit used four Exa search angles (34 requested result slots, not 34 unique/full-text sources), two Parallel Search queries, alphaXiv search/content, and SciSpace literature search. Selected primary references are recorded in the stage 3 plan. Useful design signals:
- event-driven Chrome target/frame management and debugger restrictions;
- background page lifecycle and compositor capture semantics;
- Playwright-style actionability plus independent outcome checks;
- persistent browser sessions and deterministic local workflow execution;
- workflow reuse guarded by structural evidence;
- benchmark observation reduction against success, repeat-run reliability, and matched baseline comparisons.

Do not present another paper's speedup as Controlla performance. The audit did not run matched competitor baselines or measure actual prompt submission timing.

## Remaining Stage 1 qualification

1. Preserve the uncommitted worktree and recheck its current status before edits.
2. The user must reload the unpacked extension in the **Praket** profile from the exact source path above and restart/reload Codex so it starts the rebuilt configured MCP.
3. Confirm the six live tools and the runtime/extension protocol plus build identity before any live mutation.
4. Run the safe live fixture, screenshot, navigation, upload-acceptance, target/window, focus and human-interference cases in the Stage 1 plan. Only Testing Google Classroom may receive test posts; never submit the example assignment.
5. Update the Stage 1 plan, progress, blockers and verification matrix from observed evidence. Until then, report Stage 1 as locally code-complete with live exit open.
6. Stages 2–3 are not included in this Stage 1 completion request. “Fastest/best” remains unproven without matched repeated benchmarks.

## Verification already performed

- Full locked Cargo workspace tests passed, with configured/live tests left ignored; the delayed upload acceptance fixture passed separately in isolated headless Chrome against localhost.
- Warning-denied workspace Clippy, release build and clean-prefix package install/upgrade/rollback/uninstall checks passed.
- Extension background harness, six-tool client smoke contracts, Agent UX, V3/apps/docs/provenance/dependency/benchmark/release gates, formatting and `git diff --check` passed.
- Upload fixture confirmed the exact registered bytes and SHA-256 reached the server and both ordinary output and an independent `role=status` acceptance signal appeared. Persisted state remained null.
- No live site, configured extension reload, Codex MCP restart, or live user-profile action was run. Those gates remain open by explicit user instruction.
- Temporary audit fixture was stopped and temporary browser tabs were closed; existing user tabs were preserved.

## Memory note for a new chat

A relevant prior memory entry describes the reliable direct-MCP setup pattern: build from the intended checkout, align extension ID/native-host allowed origin, use a distinct absolute `CONTROLLA_STATE_DIR` per process, restart Codex and verify the live six-tool catalog. That pattern was checked against the current local launcher/manifest/bridge above; it is not a substitute for checking fresh runtime state.

## New-chat opening instruction

Start by reading this handoff and the Stage 1 plan. Continue from this checkout and dirty tree. Stage 1 code/wiring is locally complete and independently reviewed; do not repeat implementation. The remaining work is live qualification after the user reloads the exact extension and restarts Codex. Keep local completion distinct from live Stage 1 exit.
