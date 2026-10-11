---
name: browser-control
description: Use when controlling Chrome through Controlla; describes guarded mode and the temporary local_unrestricted mode. Exposes exactly six V3 tools.
---

# Chrome browser control — Controlla V3

Controlla V3 exposes exactly six default MCP tools:

- `browser` — discover/pair shared tabs, cold-open a foreground tab with an optional workflow, launch private headless, run bounded tasks, and inspect/release sessions
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

For a cold foreground task, call `browser` once with `action:"open"`, `mode:"foreground"`, `url`, and optional `steps`. The response includes `session_id`, `tab_id`, and `workflow` when steps were supplied. In `local_unrestricted` mode, omit `session_id` and Controlla chooses a stable web tab to bootstrap pairing. In guarded mode, supply fresh `host_id` and decimal `target_ids` from discovery in the same open call. Release the returned session when finished.

Inside `steps`, use `snapshot` before `find` or `find_all`; add `href_hint` or `max_items` when only a small set is needed. `choose` with `key:"datetime"` ranks complete candidate sets by published time. `extract` takes a CSS `selector` and optionally a `fields` map; omitting `fields` reads matching elements into a `text` field. Put `verify` immediately after an action when its outcome needs proof. If a click returns `unknown` or `dispatched`, a following read-only `verify` can establish the outcome without repeating the click.
For navigation that may redirect within a known destination, use `verify` with `{"kind":"url_starts_with","prefix":"https://example.com/"}`. Use `{"kind":"url","equals":"..."}` when the exact URL matters.

## Browser actions by mode

- Foreground shared Chrome: `browser` `action:"open"` with an HTTP(S) `url` opens a new active tab; add `steps` to execute a bounded workflow in that same call. An existing accepted `session_id` reuses its session. Use the returned `tab_id` as `chrome_tab_id` for follow-up tools.
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
