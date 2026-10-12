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

## Passwords

Snapshots expose password controls only as redacted credential inputs. Password values must never be persisted to workflows, skill/route memory, logs, benchmark fixtures, or receipts. Require explicit user authorization before filling or submitting credentials.
