---
name: browser-control
description: Use when controlling Chrome through Controlla; describes guarded mode and the temporary local_unrestricted mode. Exposes exactly six V3 tools.
---

# Chrome browser control — Controlla V3

Controlla V3 exposes exactly six default MCP tools:

- `browser` — discover/pair shared tabs, open an active foreground tab in an accepted session, launch private headless, run bounded tasks, and inspect/release sessions
- `snapshot` — return compact semantic state or revision-bound delta
- `act` — perform one action (click, fill, type, press, select, page_tools, page_tool)
- `workflow` — execute a bounded deterministic action graph with partial receipts
- `extract` — return bounded structured data, URLs, citations, artifact metadata
- `verify` — independently observe and evaluate postconditions after a mutation

No other default tools exist. Screenshots, raw trees, skill admin, and benchmark controls are negotiated capabilities.

## Decision table

| Need | Tool |
|---|---|
| session/tab/mode/navigation/task | `browser` |
| fresh semantic state | `snapshot` |
| known multi-step operation | `workflow` |
| single mutation | `act` |
| structured page output | `extract` |
| proof | `verify` |

Common tasks should use one compiled `workflow` instead of many individual `act` calls. Repeated qualified tasks may use deterministic replay with fresh guards.

## Browser actions by mode

- Foreground shared Chrome: `discover_shared_tabs` → `pair_shared` with the returned `host_id` and selected decimal `target_ids` → poll `accept_shared` → `open` with `session_id`, `mode:"foreground"`, and an HTTP(S) `url`. `open` activates a new tab and returns its `tab_id`, URL, and document ID. Use that `tab_id` as `chrome_tab_id` for `snapshot` and `act`.
- Background shared Chrome: discover, pair, accept, list, release, and bounded `task`; `open` is unavailable because it activates a tab.
- Private headless: `launch`, list, release, bounded `task`, `artifact_register`, and `artifact_read`.
- `browser` action `task` supports `research`, `form_draft`, `upload`, `download`, and `repeat`. `research` navigates a paired tab; it does not create a new tab. In guarded mode, `upload` and `artifact_register` require a dedicated private headless session.
- Use `browser` action `describe` to read the runtime's current action lists. `act` has no `upload` or `navigate` action.

When `browser` action `describe` reports `interaction_policy:"local_unrestricted"`, `pair_shared` can use numeric `target_ids` without a prior discovery `host_id`. Once accepted, a numeric `chrome_tab_id` can address another Chrome tab; the extension attaches it on demand. Click, fill, select, press, and type skip their normal target checks; acknowledged input returns `dispatched` without claiming a page outcome. `artifact_register` and `task` kind `upload` can select a staged file through a shared Chrome tab's existing `input[type=file]`; the upload receipt confirms file-input readback, not that the app posted or saved it. This local experiment is enabled only by `CONTROLLA_UNRESTRICTED=1` in the MCP process and must not be described as the guarded release behavior.

## Recovery policy

- Never blind retry `unknown` — delivery status unknown means the side effect may have occurred; observe fresh state instead.
- Follow `retry_class` and `next_action` from the structured result: `safe_internal` may be repaired inside the same workflow budget (bounded scroll/remeasure), `refresh_snapshot` requires one fresh semantic snapshot, `requires_user` and `do_not_retry` must stop.
- Prefer a single bounded workflow over repeated `snapshot -> find -> act` loops.
- Use a targeted visual probe only after semantics fail, within the probe budget (max 3 crops per workflow). Full screenshot is final fallback.
- In guarded mode, refusals include `reason_code`, `retry_class`, `next_action`, and `dispatch_may_have_occurred`; never use bare `blocked`. In local unrestricted mode, `dispatched` is an acknowledgement without a page-level postcondition.
