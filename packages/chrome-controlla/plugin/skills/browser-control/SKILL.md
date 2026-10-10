---
name: browser-control
description: Use when controlling Chrome through Controlla; preserves explicit tab authority, fresh semantic references, verification, and unknown-delivery safety. Exposes exactly six V3 tools.
---

# Chrome browser control — Controlla V3

Controlla V3 exposes exactly six default MCP tools:

- `browser` — create, attach, select, inspect browser sessions and explicit target authority
- `snapshot` — return compact semantic state or revision-bound delta
- `act` — perform one guarded action (click, fill, type, press, select, upload, navigate)
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

## Recovery policy

- Never blind retry `unknown` — delivery status unknown means the side effect may have occurred; observe fresh state instead.
- Follow `retry_class` and `next_action` from the structured result: `safe_internal` may be repaired inside the same workflow budget (bounded scroll/remeasure), `refresh_snapshot` requires one fresh semantic snapshot, `requires_user` and `do_not_retry` must stop.
- Prefer a single bounded workflow over repeated `snapshot -> find -> act` loops.
- Use a targeted visual probe only after semantics fail, within the probe budget (max 3 crops per workflow). Full screenshot is final fallback.
- Every nonverified result includes `reason_code`, `retry_class`, `next_action`, `dispatch_may_have_occurred`; never use bare `blocked`.
