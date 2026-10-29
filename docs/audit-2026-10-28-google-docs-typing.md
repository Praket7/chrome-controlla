# Controlla and Google Docs: failure diagnosis — 2026-10-28

Task: type a ~3,900-character essay into an already-open Google Doc through the V3 six-tool
surface, at human pace. Result: Controlla inserted nothing. Every guarded attempt was
refused before dispatch. The text was eventually entered with operating-system keystrokes.

This document records the observed failures and their causes. It is a session diagnosis,
not a code audit: no claim is made about paths that were not exercised.

## Evidence

Each row is a receipt returned by `target/release/controlla-v2 mcp` through the documented
stdio client.

| Step | Result |
| --- | --- |
| `browser action=open` on `docs.google.com/document/create` | `Created tab navigated away before identity verification` — the tab existed and had loaded the new document; the redirect to `/document/d/<id>/edit` is the normal creation flow |
| `discover_shared_tabs` with no filter | 1 tab (an unrelated Spotify tab); `discover_domain="docs.google.com"` returned 8. No field indicated the first result was filtered |
| `click @c75` ~1 s after snapshot | `target_replaced`, `not_dispatched` |
| `type @c1` <200 ms after a fresh snapshot | `action_delivery_unknown`, reason `document changed since snapshot` |
| workflow `find` for the editor textbox | `ReadFailure: targeted semantic refresh was incomplete` |
| `reference: "textbox 'Document content'"` | `workflow find scope is incomplete or ambiguous` |
| `click` with `center_x`/`center_y` | `pinch_action_mismatch`, `not_dispatched` |
| `act action=page_tools` | `unsupported_capability` |
| `browser action=task` with `program` | `steps and program are supported only by browser action=open` |
| `type` with `typing_mode: block` | `target_replaced` |
| After client restart | `target remained busy for up to 20 seconds (current owner: another MCP process)` |
| Snapshot completeness on the doc | `complete: false`, reasons `item_or_byte_limit`, `cross_origin_frames_unread`, 4,239 nodes scanned |
| After a failed workflow | `shared extension attachment failed: "Target is not attached."` while `accept_shared` still answered `accepted: true` |

## Root causes, in order of impact

### 1. References are bound to a document revision, and Docs never stops mutating

A snapshot reference is valid only while the tab's semantic revision is unchanged. Google
Docs rewrites its DOM continuously (caret, autosave, Grammarly re-render), so the window
between `snapshot` and `act` is effectively zero. Observed at ~1,000 ms and at ~200 ms.

The extension's own rebinding path (`EDITOR_GUARD`) can recover a replaced node, but only by
matching `expected_value`. For a canvas editor there is no readable current value: the
`Document content` textbox reports `bounds: null` and no value, so the precondition cannot
be expressed and the rebind cannot run.

**Fix:** resolve at dispatch by stable identity — role, accessible name, tag, surrounding
text, unique-match requirement — and report `resolution: "rebound"` in the receipt. Make
`expected_value` optional for editors whose value is unreadable, and never silently type
into a node that did not match the identity.

### 2. `find` is the documented escape hatch, and it is gated on whole-page completeness

`find` requires a complete semantic scope. Docs always embeds cross-origin frames (offline
iframe API, ads, Grammarly), so `complete` is permanently `false`, and every `find` on Docs
fails with "refresh was incomplete". The role/name form of `act.reference` routes through
the same machinery and fails identically.

This is a category error: completeness should describe the requested scope, not every
third-party frame on the page.

**Fix:** separate `scope_complete` from page `complete`; allow `find` inside a declared
region or a known frame even when unrelated frames are unread, and say which frames were
skipped.

### 3. The one-call execution path is unavailable on an existing tab

The plan makes `browser(action="task")` the dominant path. In practice `steps` and `program`
are rejected there with "supported only by browser action=open". The composition that could
have made this cheap — observe, find, click, type in one guarded call — cannot be used on
the document a user already has open, which is the common case.

**Fix:** accept bounded steps/program on `task` for an existing session, reusing the same
VM, lease and cancellation fences as `open`.

### 4. Session acceptance is bookkeeping, not liveness

`accept_shared` returned `accepted: true` while the debugger had already detached; the next
call failed with `Target is not attached`. Recovering required a full re-pair, with no
diagnostic distinguishing "paired but detached" from "not paired".

**Fix:** probe attachment during accept and return `attached: false` with the exact recovery
step; re-attach automatically on detach and report it in the receipt.

### 5. Leases outlive the process that owns them

The tab lease is 120 s with a 20 s queue timeout. When the client process died, the tab
stayed "owned by another MCP process" until expiry, blocking the retry. `stdio-runtime.json`
publishes a process generation and heartbeat, but leases are not reconciled against it.

**Fix:** on acquire, if the recorded owner generation is absent from the live runtime
records, take over immediately and note the takeover; shorten the read lease separately from
the write lease.

### 6. Cold open rejects the normal Docs creation redirect

`/document/create` → `/document/d/<id>/edit` is how Docs creates a document, but identity
binding treats the redirect as "navigated away" and fails the open even though the tab is
usable. Creating a document is therefore impossible through `open`, and the tab is left
behind for manual cleanup.

**Fix:** permit same-origin server redirects during identity binding, re-bind to the settled
document, and keep the idempotency key tied to the settled identity.

### 7. A refused precondition is reported as an unknown mutation

`type` with a stale reference returned `action_delivery_unknown` (`status: unknown`,
`mutation_retry_policy: reconcile_read_only_first`) even though nothing was dispatched. That
classification forces a read-only reconciliation before every retry and, worse, tells the
caller a mutation might have occurred when it provably had not.

**Fix:** `stale or consumed` and `document changed` are preconditions, not ambiguous
deliveries. Return `not_dispatched` with the precondition named. Reserve `unknown` for
dispatch that was attempted and unacknowledged.

### 8. There is no coordinate route for surfaces without bounds

`click` with `center_x`/`center_y` is rejected as `pinch_action_mismatch`; only reference
clicks are allowed. Docs' editor textbox has `bounds: null`, so the one control that must be
clicked cannot be located or clicked. The visual route exists (`visual_click`,
`screenshot` + bounded region) but is not wired to this case.

**Fix:** allow a guarded coordinate click bound to a fresh same-document visual revision,
with the same no-blind-retry rule as other mutations.

### 9. Docs support is declared unqualified in code but presented as supported in docs

The extension ships `GOOGLE_DOCS_ADAPTER` with `qualified: false` and a note that it is
"characterization only until an installed-Chrome qualification row passes". Nothing connects
that adapter to the V3 `type` path, and the installed-app matrix correctly leaves the row
unqualified — yet the agent-facing skill presents `type` as generally available. The first
real test of that gap was this task.

**Fix:** either qualify Docs end to end (trusted key events against the accessibility text
surface, with readback) or state the limitation where an agent will read it before trying.

### 10. Receipt compaction truncates the cause

Reasons are cut exactly where they carry information: `document changed s`, `stale or
consumed `. A caller without source access cannot tell a revision race from a consumed
reference. The compact receipt budget is reasonable; the failure reason should be exempt
from it.

## Why a simple task took so long

- **No native tool surface.** The MCP was not exposed to the agent session, so every call
  went through the documented stdio client over a named pipe, speaking JSON-RPC by hand.
  That harness added its own failure modes: a write without a trailing newline produced
  `invalid JSON line`; a write blocked forever when the reader had died; killing the client
  orphaned the tab lease.
- **Each retry cost six to ten calls.** Fresh snapshot, an action inside the revision
  window, a refusal, then re-pair and re-accept before trying again. With a 120 s lease
  timeout after a crash, one bad attempt could cost two minutes of waiting.
- **The failure was misclassified every time.** `unknown` instead of `not_dispatched` meant
  each refusal demanded a read-only reconciliation round before the next attempt.
- **The target is structurally hostile to the current guard.** Docs is a virtualized canvas
  with cross-origin frames. Freshness and whole-page completeness — the two properties the
  guard depends on — are unsatisfiable there by construction. No amount of retrying fixes a
  precondition that cannot hold.
- **The fallback was outside the tool.** OS keystrokes work because they are trusted input,
  but they need an Accessibility grant, which is a user action, not an API call.

## What actually worked

CDP trusted key events at a human pace, once the guard stopped asking a canvas editor for a
readable value it cannot produce. `HumanKeys` delivers one keydown/keyup pair per character
to the editor that owns the selection, which is the same input path a person uses; the
editor ignores page-JavaScript-created key events, so the earlier console experiments were
misleading. Verification comes from the document, not the editable element: select all,
then let Docs answer the copy event into a caller-owned buffer (or read the plain-text
clipboard flavor, captured and restored), and compare the result with the typed text.

## Recommended order of work

1. Dispatch-time reference resolution by stable identity, with `rebound` receipts.
2. Scoped completeness so `find` can work inside a region of an incomplete page.
3. Precondition refusals reported as `not_dispatched`.
4. Steps/program on `task` for existing sessions.
5. Attachment liveness on accept, and lease takeover when the owner generation is dead.
6. Same-origin redirect handling in cold open.
7. Guarded coordinate click for canvas surfaces, followed by a qualified Docs adapter.

## Resolution — same day

Typing now works end to end on an installed Chrome profile:

| Piece | Change |
| --- | --- |
| Guard | A canvas/blank-frame editor is reported as unreadable (`value_readable: false`, `source_kind: same_origin_blank_frame`) and the type guard no longer demands a value or focus precondition it cannot satisfy. |
| Rebind | A revision change between snapshot and dispatch re-resolves the same control by role, accessible name, and tag; the receipt reports `resolution.status: rebound` with one attempt. |
| Verification | After dispatch, Controlla selects all and asks the document to copy its selection into a `DataTransfer` (`document_copy_event`), with the clipboard path (`clipboard_document_text`) as fallback. Evidence is a character count, a hash, and matched/sampled windows, never page text. |
| Hidden tabs | Chrome keeps a hidden tab's selection out of reach, so the receipt is `dispatched_unverified` with `verification: "tab_hidden"` and a next action instead of comparing the user's untouched clipboard and calling the insert a mismatch. The clipboard is not read or written in that case. |

Live receipts from 2026-10-28 on the installed extension and a fresh Google Doc:

- Foreground: `status: verified`, `verification: clipboard_document_text`,
  `document_text.characters: 81`, `matched_windows: 3/3`, `clipboard_restored: true`; an
  earlier foreground pass verified through `document_copy_event` with `exact_contains: true`.
- Background: `status: dispatched_unverified`, `verification: tab_hidden`, no key or clipboard
  commands after dispatch, and a next action to bring the tab forward.

Still open: saved Drive persistence is not proven by these receipts (they prove the live
editor's own copy output), cross-origin embeds keep whole-page completeness partial, and a
Docs qualification row still needs the standard installed-Chrome matrix sweep.
