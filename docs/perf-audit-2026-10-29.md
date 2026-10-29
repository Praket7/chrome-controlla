# Execution-path audit — latency, command count, and verification

Date: 2026-10-29. Scope: the paths a normal agent call actually takes —
`snapshot`, `act` (click/type), `verify`, `browser open`, and the cross-process
lease that gates them. Numbers come from the `metrics` block each receipt
returns; they are not estimates.

## Where the time went

| Path | Measured | Composition |
|---|---|---|
| `act type` (human keys), 15 chars | 2.9 s, 59 CDP commands | 3 input commands per character (keyDown, char, keyUp), ~45 ms round trip each, plus pacing |
| `snapshot` (ranked target) | 32 ms, 5 CDP commands | `Page.getFrameTree` twice (before/after identity), one page evaluation, one release |
| `snapshot` (full page, 4,103 nodes) | 103 ms | Same commands; time is DOM scan and serialization |
| `browser open` (foreground, new tab) | 2.0 s | Tab creation and identity binding dominate; 2 CDP commands |
| Cross-process lease after an unknown write | up to 20 s, then refusal | The retained lock outlived its own TTL and stayed held by a live process |

Two of these were defects rather than costs:

1. **Long typing was cut off, not merely slow.** The runtime told the extension
   the transaction deadline was at most 60 s (`clamp(1, 60_000)`) while the
   human-pacing budget assumed 50 ms per character. A 3,900-character document
   therefore aborted mid-text around 30 s with `text_transaction_uncertain`,
   leaving a partially typed document and no way to know how much landed.
2. **A retained lease could block a target indefinitely.** `release_unknown`
   only released entries still flagged `unknown`, and a nested reacquire could
   clear that flag while keeping the file lock. `release_generation` also
   required the owner string to match, so a rewritten owner left the OS lock
   held with nobody able to release it. The record's own `expires_unix_seconds`
   was never consulted for a live process.

## Changes

| Change | Effect | Verification |
|---|---|---|
| `typeTextTransaction` sends one `keyDown` carrying `text` plus `keyUp` per character instead of `keyDown` + `char` + `keyUp` | One third fewer input commands per character, ~45 ms saved per character; the native inserter is unchanged because Chrome synthesizes keypress/input from `keyDown.text` | `node extensions/chrome-controlla/test-background.cjs` (fixtures updated to insert on keyDown text) |
| Transaction deadline passed to the extension raised from 60 s to its documented 600 s ceiling; MCP wall clamp raised from 600 s to 900 s | Human-paced typing can finish a full document in one transaction instead of aborting mid-text | `cargo build`, live essay run (see below) |
| `human_typing_timeout_ms` budgets 90 ms of bridge round trip per character instead of 50 ms, ceiling 900 s | The default timeout now covers the commands the transaction actually sends | unit tests in `v3` |
| `release_unknown` releases an expired lease even when the `unknown` flag was rewritten; `release_generation` matches the hold generation, not the visible owner | A retained lock can no longer outlive its TTL | `cargo test -p controlla-runtime --lib process_lease` (11 pass, including two new cases) |
| `stale_owner` takes over an expired lease held by a live process whose generation still matches | Another MCP process can recover a target after a crash or an abandoned reconciliation instead of waiting forever | `expired_lease_of_a_live_process_is_taken_over` |
| Recovery ladders execute at the dispatch sites (scroll, swipe, pinch, click, block/fill, key typing, IME, download preflight) instead of returning the plan | A repairable guard is repaired before the caller sees it; the receipt records `recovery.executed` and `recovery.resolved_via`, on success and on exhaustion | `cargo test -p controlla-runtime --test guard_diagnostics`, `recovery_exec` unit tests, `v3_rich_editor_fast_paragraphs_and_human_pace_are_verified`, live probes below |
| The editor guard treats a `role=textbox` shell as an editor only when it is actually editable; the editable-descendant rung returns the resolved element so the guard is retargeted at a real remote object | A wrapper editor that holds one inner `contenteditable` is repaired in one rung instead of failing `focus_or_selection_changed` on a wrapper that cannot take focus | `npm run check:editor-ladder` (five Playwright cases against the production guard and rung source), live probe below |
| Successful text receipts carry the ladder evidence, and the final click revalidation refusal attaches what already ran | A verified receipt now states which rung repaired the transaction, so verification does not hide the recovery that made it possible | `npm run check:editor-ladder`, live probes below |
| `Controlla.typeTextTransaction` and `Controlla.fastType` wait for their own transaction budget instead of the flat 30 s host command timeout | A 700-character human-paced transaction needs ~50 s; the 30 s bound cut the reply off while the editor was still typing and reported `unknown`, so nothing could be verified. Typing now waits for `deadline_ms` plus delivery slack; other commands keep 30 s | `typing_commands_wait_for_their_own_transaction_budget`; live: 788 characters typed and verified in 51 s |

## Executed ladders, live

Two local pages were driven through the real MCP server (release binary,
private headless Chrome) on 2026-10-29. The pages exist so a guard fails in a
recoverable way and the receipt can be read against a separate observation.

| Page | Action | Receipt | Independent observation |
|---|---|---|---|
| `div[role=textbox]` shell wrapping one `contenteditable` | `act type` | `status: verified`, `typing_mode: block`, `recovery.executed: ["editable_descendant"]`, `recovery.resolved: true`, `automatic_recovery: true`, `model_action_required: false` | `verify text_present "ladder ran"` returned `passed` |
| `button` with `pointer-events: none` inside `body` (no plausible ancestor) | `act click` | `status: not_dispatched`, `reason_code: pointer_events_none`, `recovery.executed: ["ancestor_pointer"]`, `recovery.exhausted: true`, `recovery.stage: internal_ladder_exhausted`, `model_action_required: false` | the page's output remained `idle`; no replay |

Before this change the first case refused with an unclassified focus failure and
the second returned the ladder as a plan. The shell case also exposed why the
old editor guard could not recover: it classified `role=textbox` as editable
without asking whether the node could hold a caret, and the rung had no way to
hand back a new remote object. Both are fixed and covered by
`npm run check:editor-ladder`.

## Long-form typing, live

A 3,974-character Google Docs essay was typed into a freshly created document
with human-paced key events. Reopening the document in its read-only mobile view
returns 3,985 normalized characters and contains both the opening sentence and
the final sentence of the source, and the type receipt reported
`status: verified`, `verification: clipboard_document_text`. That path selects
all, lets the document answer the copy itself, and matches sampled windows
across the whole typed text, so it is a whole-document comparison rather than a
first-line probe. The document is
https://docs.google.com/document/d/1VaeeJytbu5sFlrCLMGp0e5llBWp98ZxnuyrqcTBa9Fs/edit.
A 788-character paragraph measured 51 s (65 ms per character at a 45 ms pace),
which fits the two-command-per-character path. The document's name still reads
"Untitled document": Docs commits a title rename only from trusted keyboard
input, and its controlled input reverts a programmatic fill before the commit,
which is a known gap rather than a silent success.

## Boundary

- The extension change is verified by the extension harness against the real
  `lib/input.js`. The live timings above prove that typing, verification, and
  the longer transaction budget work end to end. They do not prove the running
  Chrome already serves the new per-character sequence: an unpacked extension
  keeps its loaded code until `chrome://extensions` reloads it. Reload it once
  after pulling this change to get the one-third command reduction.
- Canvas editors (Google Docs) still cannot report their value. Verification
  selects all, asks the document for its copy output, and compares derived
  evidence (length, hash, matched windows); a hidden tab is reported as
  `dispatched_unverified` with `verification: "tab_hidden"` rather than
  comparing the user's clipboard.
- `snapshot` pays for two frame-tree reads to prove identity did not change
  during the scan. Removing one would save ~15 ms and weaken the only guard
  that separates a stale reference from a live one; not changed.
- The download ladder is deliberately non-mutating: its scroll and pointer
  rungs only scroll the link into view and hit-test it, because a download must
  not turn a recovery into a click. If the guard still refuses after those
  rungs, the receipt reports the executed steps and stops.
