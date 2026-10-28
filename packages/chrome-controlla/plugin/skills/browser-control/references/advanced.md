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
