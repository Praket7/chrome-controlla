# Advanced Controlla browser behavior

Load this only when the task actually needs one of these features.

## Workflow VM

`workflow` supports bounded `step`, `if`, `foreach`, and `try_fallback`. Fallback is permitted only after read failure or known `not_dispatched`, and never after a successful or unknown mutation. A mutation with `unknown`/`dispatched` may continue only to immediate read-only verification.

## Virtualized lists

Use `collect_virtual` when the page recycles rows or reveals items only through scrolling. Bound `max_scrolls` and treat `coverage_complete:false` as incomplete evidence.

## Typing

Omit `typing_mode` normally. Controlla chooses block insertion, fast trusted key events, or IME based on the target. Use `human_keys` only when the user explicitly asks for human-paced input.

## Page tools

Use a page-native tool only when it is explicitly available and its schema matches the expected bounded schema. Do not perform page-tool discovery on every task.

## Modes

Foreground uses the real selected Chrome profile and may activate a window. Background must not activate it. Private headless uses an isolated browser/session. Normal user-visible interaction should not launch private headless merely for speed.

Headless is isolated browser lifecycle mode, not an `open` mode. For one private task, call `browser` with `action=task`, `mode="headless"`, a read-only `Research` task without `tab_ids`, and no session or tab IDs; Controlla launches at `about:blank`, navigates to the task URLs, verifies, and releases inside that call. Form drafts, mutations, uploads, downloads, and repeat actions need an existing session because they depend on a page, reference, or artifact. Use this only for private research, not normal visible interaction. For a multi-step session, call `action=launch` with `mode="headless"`, then use returned `target_ids[0]` directly with `action=task`; skip `list` and standalone `snapshot`. Repeat the mode on `list` and `release`. Existing-session tasks omit `mode`; `release_after_task: true` releases after the task, including errors. If a result says `private_session_released: false`, do not relaunch or repeat the task; use `cleanup_retry_action` or its session ID to release first. Use `list` only for tab inventory. `open` accepts only `foreground` or `background`; page tools take `session_id` and do not accept `mode`.

## Passwords

Snapshots expose password controls only as redacted credential inputs. Password values must never be persisted to workflows, skill/route memory, logs, benchmark fixtures, or receipts. Require explicit user authorization before filling or submitting credentials.
