# Chrome Controlla continuation handoff

Updated: 2026-10-27

## Active checkout

- **Working tree:** `/Users/pcg/Documents/Codex/2026-10-26/controlla-performance`
- **Repository:** `https://github.com/Praket7/chrome-controlla.git`
- **Branch:** `codex/controlla-performance-continuation`
- **HEAD at handoff:** `7746946fd0b5925edc3fce44fb736bb1b2153741`
- Continue in this checkout. The pre-existing `handoff.md` and `HANDOFF_CONTROLLA.md` describe older checkouts and older runtime state; recheck this handoff and `git status` before editing. Preserve the current uncommitted changes.

All implementation work for the current performance/web-writing continuation is in this checkout. The source tree is not a clone of the older `2026-10-06`, `2026-10-12`, or `controlla 4` paths mentioned in the older handoffs.

## Current uncommitted work

`git status` showed these seven modified tracked files before this handoff was added:

- `crates/controlla-runtime/src/mcp/v3_full.rs`
- `extensions/chrome-controlla/background.js`
- `extensions/chrome-controlla/test-background.cjs`
- `packages/chrome-controlla/plugin/skills/browser-control/references/advanced.md`
- `docs/progress.md`
- `docs/verification-matrix.md`
- `docs/blockers.md`

The runtime change adds guarded per-character typing for accessible child-frame textboxes. It binds frame, loader, and backend-node identity; checks the expected text and focus/selection before input; dispatches one character at a time; performs readback and key cleanup; and marks uncertain delivery instead of replaying. Child-frame IME and password typing remain unsupported. The extension command and regression harness cover keydown/character/keyup, stale loader/value refusal, and focus theft. The docs record the local evidence and keep live writing qualification open.

This handoff is a new file; it does not replace either older handoff. At the time it was created, no commit or push had been made; see the Git history for later integration.

## Verification recorded for this checkout

The latest repo ledger records these passes:

- `cargo test --workspace --locked --offline --quiet`
- `cargo test -p controlla-runtime frame_editor_reference_binds_frame_loader_and_backend_node`
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`
- `node --check extensions/chrome-controlla/background.js`
- `node --test extensions/chrome-controlla/test-background.cjs`
- `npm run check:docs` and `git diff --check`
- `./scripts/package-build.sh` and `./scripts/package-check.sh` (darwin/arm64 package lifecycle)

`cargo fmt --all -- --check` remains blocked by existing formatting differences in untouched `crates/controlla-runtime/tests/v2_stdio.rs` and `tests/v3_tasks.rs`; those files were not reformatted. These results are local evidence, not hosted CI or live app qualification. See the dated entries in `docs/progress.md`, `docs/verification-matrix.md`, and `docs/blockers.md`.

## Latest live Docs attempt

After the user clicked inside the blank document body, Controlla rediscovered, paired, and accepted the existing tab **AP Seminar Debate - Gauri - Google Docs**:

- Document: `https://docs.google.com/document/d/1Bklxh_OMDru3kKqmgCj-ZMsilWtgDfBG1W2c14PTe7s/edit?tab=t.0`
- Chrome tab ID at test time: `1649785784`
- The Docs body appeared blank (only zero-width/blank placeholder text).
- `act(action="type", typing_mode="human_keys")` was explicitly not dispatched with `reason=not_editable`.
- A click on the semantic editor target was refused before dispatch with `reason_code=unsafe_page`; the receipt said not to retry that same action.
- A fresh read-only snapshot confirmed the test string was absent and the document body remained blank. The shared session was released without closing the user tab.

Therefore Controlla successfully paired with and read the live tab, but **live HumanKeys typing into Google Docs is still unqualified and currently blocked**. Do not report a successful write. To continue, the user can click inside the blank Docs body; then take a fresh snapshot and reassess the editor reference before another guarded attempt. Do not retry the rejected reference/action unchanged.

The local fixtures do not prove Google Docs behavior or persistence. Native desktop Word is outside this Chrome-only repository's scope.

## Last-known installed/runtime identity

Prior live work in this task recorded the unpacked extension ID as `aajfigcgcboikdndhkclcngjcjpjaifn`, the release binaries under this checkout's `target/release/`, and the Codex launcher at `/Users/pcg/.chrome-controlla/codex-mcp-launcher-stage3`. Verify those paths and the active Hotload child before rebuilding or asking for another reload; local configuration may have changed. The user has already reloaded the extension and Codex MCP for this continuation.

## Resume checklist

1. Inspect `git status` and preserve all dirty changes.
2. Read the latest web-writing notes in `docs/progress.md`, `docs/verification-matrix.md`, and `docs/blockers.md`.
3. For a live Docs retry, require a fresh editor reference, an observed focused Chrome window, a tiny reversible test, readback, and an observed cleanup. If Controlla refuses before dispatch or reports uncertainty, verify read-only once and do not replay an uncertain mutation.
4. Continue the remaining performance scope tracked in the docs: corrected paired baseline, broader identity checks/task programs, and live application/client qualification. Keep app acceptance separate from actor/client completion and from synthetic fixture evidence.
