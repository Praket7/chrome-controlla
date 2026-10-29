# Advanced Controlla browser behavior

Load this only when the task actually needs one of these features.

## Workflow VM

`workflow` supports bounded `step`, `if`, `foreach`, `try_fallback`, and `parallel_read`. Fallback is permitted only after read failure or known `not_dispatched`, and never after a successful or unknown mutation. A mutation with `unknown`/`dispatched` may continue only to immediate read-only verification. `parallel_read` accepts only read-only steps and is rejected before dispatch when any child could mutate state.

Steps include navigate, snapshot, find, click, double_click, context_click, hover, touch, swipe, multi_touch, scroll, scroll_to, fill, select, type, press, extract, verify, and widget data transfer. Prefer these over separate `act` calls when the sequence is already known.

`browser(action="task", task_program=...)` compiles the same typed steps (plus bounded control flow) into that VM, so one call can navigate, observe, fill, click, and verify without returning intermediate model turns. Programs are bounded to 128 expanded steps and 16 nesting levels; `drag` and selector `wait_for` are rejected rather than approximated.

## Virtualized lists

Use `collect_virtual` when the page recycles rows or reveals items only through scrolling. Bound `max_scrolls` and treat `coverage_complete:false` as incomplete evidence.

## Typing

Omit `typing_mode` normally. Controlla chooses block insertion, fast trusted key events, IME, or a qualified editor adapter from the resolved target. Root and child-frame text entry use one `Controlla.typeTextTransaction` per operation; chunking and readback stay inside the extension, and no path sends one native message per character. Use `human_keys` only when the user explicitly asks for human-paced input. A verified editor readback confirms the live field value, not server-side save or persistence.

## Page tools

Use a page-native tool only when it is explicitly available and its schema matches the expected bounded schema. Do not perform page-tool discovery on every task.

## Modes

Foreground uses the real selected Chrome profile and may activate a window. Background must not activate it. Private headless uses an isolated browser/session. Normal user-visible interaction should not launch private headless merely for speed.

Headless is isolated browser lifecycle mode, not an `open` mode. For one private task, call `browser` with `action=task`, `mode="headless"`, a read-only `Research` task without `tab_ids`, and no session or tab IDs; Controlla launches at `about:blank`, navigates to the task URLs, verifies, and releases inside that call. Form drafts, mutations, uploads, downloads, and repeat actions need an existing session because they depend on a page, reference, or artifact. Use this only for private research, not normal visible interaction. For a multi-step session, call `action=launch` with `mode="headless"`, then use returned `target_ids[0]` directly with `action=task`; skip `list` and standalone `snapshot`. Repeat the mode on `list` and `release`. Existing-session tasks omit `mode`; `release_after_task: true` releases after the task, including errors. If a result says `private_session_released: false`, do not relaunch or repeat the task; use `cleanup_retry_action` or its session ID to release first. Use `list` only for tab inventory. `open` accepts only `foreground` or `background`; page tools take `session_id` and do not accept `mode`.

## Passwords

Snapshots expose password controls only as redacted credential inputs. Password values must never be persisted to workflows, skill/route memory, logs, benchmark fixtures, or receipts. Require explicit user authorization before filling or submitting credentials.
# Visual settling and desktop fallback

Use `ready_when: {"kind":"visual_stable","quiet_ms":300,"timeout_ms":3000}` on `browser action=open`, or `{"kind":"wait_stable","quiet_ms":300,"timeout_ms":3000}` inside a workflow. Add a bounded integer-pixel `region` when only part of the viewport matters. The step is read-only: if it times out, inspect the current page and do not dispatch the planned visual mutation from that receipt.

## Deprecated compatibility paths

`Controlla.frameHumanTypeCharacter` remains registered for one compatibility cycle so already-packaged clients do not break. No V3 task, workflow, or `act` typing path calls it: they use `Controlla.typeTextTransaction`, and the extension tests assert a 1,000-character frame type sends exactly one such request and no per-character frame typing. Remove the shim only after two green package-compatibility cycles; do not route new work through it.

Use native fallback only when the live capability report advertises it and the task needs foreground pointer movement/scrolling or a non-submitting key. It refuses coordinate clicks, drags, all text entry, and navigation because native visual evidence cannot establish safe intent. Use fresh semantic extension actions for page controls and navigation; keep order confirmation, credential entry, challenge hand-off, and background actions on their guarded extension routes. See `docs/desktop-fallback.md` for exact platform status.

## Execution policy and recovery

`execution_policy: "execution_first"` is the default. It lets the runtime
resolve a backend mismatch, frame route, viewport problem, stale reference,
scroll container, focus drift, or dialog route without a model turn. The receipt
reports `automatic_recovery: true` and a `recovery.plan` naming each internal
rung (`trusted_cdp`, `frame_local_cdp`, `oopif_child_session`,
`editable_descendant`, `semantic_reacquire`, `dialog_route`, ...).
`execution_policy: "strict"` restores the older conservative classification in
which those conditions are surfaced to the caller.

Recovery is bounded and non-mutating. It never re-dispatches an effect whose
delivery is uncertain; `unknown` still requires one read-only verification and
never a replay.

## Capability boundaries moved out of the fast path

- Screenshot evidence: `verified` means the PNG decoded and its
document/lifecycle/revision still matched. `unavailable` means Chrome exposed no
valid compositor surface. `stale` means navigation or topology changed and the
pixels were discarded. An empty or malformed PNG is never evidence.
- `act(action="click", center_x, center_y)` without a reference clicks one page
coordinate after a bounded visual revision and exact hit test. It returns
`requires_verify: true` and is never replayed.
- A stale document revision is a precondition, not a failed write: `act` returns
`not_dispatched` with `dispatch_may_have_occurred: false`. For typing, the
runtime repairs once by resolving the same editor in the live document and its
same-origin frames by role plus accessible name.
- Canvas editors (Google Docs is the reference case) cannot report typed text
from the editable element. A visible tab verifies through `document_copy_event`
or `clipboard_document_text` with `document_text.hash`; a hidden tab returns
`dispatched_unverified` and `verification: "tab_hidden"`. A verified editor
readback proves the live document content, not server-side persistence.
- Cross-origin frames: `find` and targeted snapshots report unread frames under
`cross_origin_frames_skipped` and set `coverage: partial`. Use
`include_frames=true` for a bounded accessibility aggregation over up to eight
child frames, or the explicit `frame_id` path for one frame.
- Password entry requires `secure_fill` with `allow_password=true`, the exact
credential-free `expected_origin`, and an `account_marker`. If the user supplied
the password in chat, pass it only in that action's `value`; otherwise omit it
and Controlla opens its one-use local prompt. The receipt is a redacted boolean.
- Headless is an isolated lifecycle mode, not an `open` mode. A one-call private
`Research` task launches, navigates, verifies, and releases internally. Form
drafts, mutations, uploads, downloads, and repeat actions need an existing
session because they depend on an observed page, reference, or artifact.
- `browser action=release` detaches Controlla and keeps the user's tab open.
- Upload receipts separate file selection, application acceptance, and
persistence; report only the level observed.
