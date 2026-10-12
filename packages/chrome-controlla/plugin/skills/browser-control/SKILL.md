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

For a cold foreground task, call `browser` once with `action:"open"`, `mode:"foreground"`, `url`, a stable `idempotency_key` for this logical open, and optional `steps`. Reuse that same key when recovering from a lost response; use a different key only when a distinct new tab is intended. The response includes `session_id`, `tab_id`, and `workflow` when steps were supplied. In `local_unrestricted` mode, omit `session_id` and Controlla chooses a stable web tab to bootstrap pairing. In guarded mode, supply fresh `host_id` and decimal `target_ids` from discovery in the same open call. Release the returned session when finished.

Before the first call after an MCP reload, inspect the live `browser` schema. It must expose `idempotency_key` as required for `action:"open"`, and `task` must expose the tagged task variants. If a runtime error says a required property is missing while the offered schema omits it, stop: reload or repair the MCP schema/runtime pair instead of trying more argument combinations. For example, research uses `{"kind":"research","urls":["https://example.test/"]}` and navigates the selected existing tab.

Password controls are visible in snapshots only as redacted references with `credential_input:true`, `requires_user_input:true`, and `raw_value:null`. To fill a password explicitly authorized by the user, use `act` with `action:"fill"`, the fresh reference, and `allow_password:true`. This transmits the supplied value through the local MCP/browser; snapshots and action results remain redacted. Do not use password values in workflows, typing actions, tests, or logs. Do not submit a login unless the user has authorized the sign-in action.

Inside `steps`, use `snapshot` before `find` or `find_all`; add `href_hint` or `max_items` when only a small set is needed. `choose` with `key:"datetime"` ranks complete candidate sets by published time. `extract` takes a CSS `selector` and optionally a `fields` map; omitting `fields` reads matching elements into a `text` field. Put `verify` immediately after an action when its outcome needs proof. If a click returns `unknown` or `dispatched`, a following read-only `verify` can establish the outcome without repeating the click.

If workflow `find` gets zero matches, it takes one fresh snapshot and retries once. This handles controls that appear just after a click; a second zero-match result remains a failure and should be reported as such.
For navigation, click with `{"kind":"navigation"}` when any document change is the desired outcome, or give `url` only as an exact absolute HTTP(S) destination. Never pass a relative path as an exact URL. For canonical redirects, use navigation-only click followed by `verify` with `{"kind":"url_starts_with","prefix":"https://example.com/"}`. Use `{"kind":"url","equals":"..."}` when the exact URL matters.

For exact text, omit `typing_mode` by default: Controlla selects block insertion for ordinary text, fast key events where the target requires trusted keys, and IME for composition-sensitive text. In rich editors, consecutive newline delimiters normalize to one paragraph break and dispatch as a real Enter key event. Use `fast_keys` only when trusted key events are needed; use `human_keys` only when the user asks for human-paced typing. Human typing defaults to 180 ms after each character (about 55 words per minute); set `delay_ms` to override it. Its deadline scales with text length. For “newest,” use `find_all` with a narrow story/link filter and `choose` by `datetime`; do not guess from link order.

## Browser actions by mode

- Foreground shared Chrome: `browser` `action:"open"` with an HTTP(S) `url` opens a new active tab; add `steps` to execute a bounded workflow in that same call. An existing accepted `session_id` reuses its session. Use the returned `tab_id` as `chrome_tab_id` for follow-up tools.
- Background shared Chrome: discover, pair, accept, list, release, and bounded `task`; `open` is unavailable because it activates a tab.
- Private headless: `launch`, list, release, bounded `task`, `artifact_register`, and `artifact_read`.
- `browser` action `task` supports `research`, `form_draft`, `upload`, `download`, and `repeat`. `research` navigates a paired tab; it does not create a new tab. In guarded mode, `upload` and `artifact_register` require a dedicated private headless session. Local unrestricted mode can register bounded bytes or one explicit absolute local file path (up to 10 MiB), then upload through a known file input or visible trigger. Trigger upload checks Chrome's chooser event first, then searches the paired root and attached cross-origin frames for exactly one lazy file input; ambiguous inputs are refused. A visible-trigger upload returns within a 45-second operation deadline. Receipts distinguish Chrome file selection, app acceptance, and persistence.
- `workflow` also supports `{"kind":"upload","artifact_handle":"<registered handle>","trigger_reference":"@c7","account_marker":["#account","Account A"],"acceptance":{"kind":"text","reference":"@c8","equals":"Attached"}}` in a `local_unrestricted` shared session. Use `account_marker:["ax:button","Google Account: Name (account@example.com)"]` when no unique CSS selector exists. Controlla checks one exact visible root-DOM button name and excludes selected controls and menu/listbox/dialog scopes; it does not authorize from an accessibility picker entry. Otherwise use one exact CSS selector and visible name. In guarded mode, use the dedicated private headless `browser` task upload route. Replace sample references and account marker with evidence from the current page. Register the exact file once with `browser` action `artifact_register` in the same session, then pass that returned handle to the workflow. An `open` workflow may use an artifact only when the handle belongs to that same session; never pass a handle from another session. The upload step reuses the trigger/chooser route, returns per-step timing, and stops the workflow unless application acceptance is verified. A verified acceptance does not prove persistence or justify replaying a later post.
- To inspect cross-origin pickers, first use the ordinary semantic snapshot and `browser` `action:"list_shared_targets"` to get observed child `frame_id` values. Request a bounded `snapshot` with the picker `frame_id`; it returns loader-bound `@f:` references for actionable controls. If an Upload button opens a picker with a second Browse button, pass Browse's exact `@f:` reference as the upload task's `trigger_reference`. `act` can click frame references only with an explicit outcome; it returns `unknown` after dispatch until a fresh frame snapshot confirms the result. A changed frame loader invalidates the reference.
- Account-sensitive upload checks require one exact visible identity marker in the paired tab. Prefer a unique CSS selector; if the account control is a button with no unique selector, use `ax:button` plus its exact accessible name. That marker is resolved against the visible root DOM's computed name, not the picker AX tree, and fails when duplicate matches or menu/listbox/dialog context exist. A selected/highlighted picker option, including one inside a cross-origin frame, is not proof of the signed-in account. After choosing an account, proceed only when the app exposes the active identity; otherwise withhold the upload.
- For a foreground `browser` `open` retry, send the same `idempotency_key` for the same logical open. It returns the original Controlla-created tab; use a new key only for a new tab.
- Use `browser` action `describe` to read the runtime's current action lists. `act` has no `upload` or `navigate` action.

When `browser` action `describe` reports `interaction_policy:"local_unrestricted"`, `pair_shared` can use numeric `target_ids` without a prior discovery `host_id`. Once accepted, a numeric `chrome_tab_id` can address another Chrome tab; the extension attaches it on demand. Fill, select, press, and type skip their normal target checks; acknowledged input is not page verification. Click still checks its explicit outcome and returns `unknown` until observed. `describe` reports guarded and local-unrestricted artifact scopes separately and includes a schema fingerprint. The client-loaded state remains unverified until the client acknowledges that fingerprint. This local experiment is enabled only by `CONTROLLA_UNRESTRICTED=1` in the MCP process and must not be described as the guarded release behavior.

## Recovery policy

- Never blind retry `unknown` — delivery status unknown means the side effect may have occurred; observe fresh state instead.
- Follow `retry_class` and `next_action` from the structured result: `safe_internal` may be repaired inside the same workflow budget (bounded scroll/remeasure), `refresh_snapshot` requires one fresh semantic snapshot, `requires_user` and `do_not_retry` must stop.
- Prefer a single bounded workflow over repeated `snapshot -> find -> act` loops.
- Use a targeted visual probe only after semantics fail, within the probe budget (max 3 crops per workflow). Full screenshot is final fallback.
- In guarded mode, refusals include `reason_code`, `retry_class`, `next_action`, and `dispatch_may_have_occurred`; never use bare `blocked`. Input dispatch acknowledgement alone is not a page-level postcondition.
