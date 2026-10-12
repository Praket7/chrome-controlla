---
name: browser-control
description: Use when controlling Chrome or a web app with Controlla. Start browser work immediately through the stable six-tool contract. Do not inspect MCP schemas, call describe, read repository source, search MEMORY.md, or reread documentation before normal browser actions.
---

# Chrome Controlla fast path

Controlla exposes exactly six normal tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`.

## Rule zero

If those six tools are visible, **act immediately**. Do not list MCP servers, inspect schemas, call `browser describe`, search this repository, read `mcp.rs`, `v3_full.rs`, `observe.rs`, or load extra documentation to learn routine arguments. The plugin keeps schemas synchronized internally. Source/schema inspection is recovery-only after an explicit unrecoverable schema/tool error.

Optimize **user prompt to verified result**, not only browser execution time.

Default shape:

`most specific destination -> one targeted observation only if needed -> one deterministic workflow -> verification only if still needed -> release`

Do not visit a homepage when a specific destination is user-provided, observed, or available through qualified Controlla route memory. Do not take duplicate broad snapshots. Do not send deterministic intermediate state back to the model merely so it can issue the obvious next action.

## Use `browser` first

`browser` is the primary entry point for ordinary tasks. A normal open no longer requires the agent to invent an idempotency key. The plugin supplies a short-lived retry-safe key internally.

```json
{"action":"open","url":"https://example.com/specific-page"}
```

The result includes an opaque `context`. Use that handle for follow-up tools instead of tracking session and tab IDs.

```json
{"context":"<ctx>","goal":{"query":"target words","roles":["link"],"exhaustive":false},"max_items":20,"max_text_chars":0,"max_scan_nodes":12000}
```

When exactly one Controlla context is active, follow-up tools may omit `context` as well.

## High-level compiled goals

For supported common tasks, give `browser` the goal and let Controlla compile the mechanics. Do not manually reproduce the same snapshot/find/choose/click chain.

Newest item example:

```json
{"action":"open","goal":{"kind":"open_newest_item","site":"espn.com","entity":"Buffalo Bills","item_type":"story"}}
```

For the qualified ESPN Bills route, Controlla performs internally:

`Bills team page -> one targeted exhaustive story observation -> find candidate story links -> choose by datetime evidence -> click -> verify`

It does **not** open ESPN home, browse menus, or take exploratory homepage snapshots. The external agent sees one tool call while Controlla uses the existing bounded browser/workflow machinery underneath.

A compiled goal is confidence-gated. Public direct routes may come from a user URL, observed href, verified route memory, or a qualified public site profile. Reused routes are still verified. Structural drift or repeated verification failure quarantines the route and forces the normal semantic fallback. Private account routes, credential-bearing URLs, and destructive/submission endpoints are never learned as public fast paths.

## Tool choice

| Need | Tool |
|---|---|
| open, navigate, start a task, compiled common goal | `browser` |
| one new semantic observation | `snapshot` |
| known multi-step work | `workflow` |
| one isolated mutation | `act` |
| bounded structured output | `extract` |
| proof when existing receipt is insufficient | `verify` |

Prefer one `workflow` to repeated `snapshot -> act -> snapshot` loops. Use a targeted snapshot and `max_text_chars:0` when prose is irrelevant. For newest/latest, use complete narrow candidates and publication evidence rather than DOM order.

## Verification

If a mutation receipt already proves the requested postcondition, do not verify again. Never blind retry `unknown`. If a receipt returns `unknown` or only `dispatched`, perform one fresh read-only verification and never replay the mutation. `not_dispatched` may be corrected according to its structured reason and retry guidance.

## Modes

Stay in guarded mode for normal work. `local_unrestricted` is an explicit temporary compatibility mode only when the task truly requires it; load `references/advanced.md` before using it and return to guarded mode afterward.

## Cleanup

Release the shared session when finished while preserving the user's tab:

```json
{"action":"release","context":"<ctx>"}
```

## Progressive disclosure

Load additional material only when the task actually requires it:

- `references/common-recipes.md` for less common workflow patterns
- `references/uploads-and-frames.md` for files, account identity, app acceptance, or cross-origin pickers
- `references/advanced.md` for VM control flow, virtual lists, page tools, modes, typing details, or raw compatibility arguments
- `references/recovery.md` only after a real failure

Normal browser tasks should require **zero repository reads, zero schema/describe calls, zero MCP-discovery calls, zero homepage detours when a qualified direct route exists, and normally no more than one model-visible observation before deterministic execution**.
