---
name: browser-control
description: Use when controlling Chrome or a web app with Controlla. Start browser work immediately through the stable six-tool V3 contract. For normal navigation, reading, clicking, typing, forms, and extraction, do not inspect MCP schemas, call describe, read repository source, or search documentation before acting. Use recovery references only after an explicit failure.
---

# Chrome Controlla fast path

Controlla exposes exactly six normal tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`.

**Use them immediately.** If those tools are visible, do not list MCP servers, inspect schemas, call `browser describe`, read `MEMORY.md`, search this repository, or open implementation files such as `mcp.rs`, `v3_full.rs`, or `observe.rs` to learn routine arguments. Do not reread this skill between normal calls. Discovery and source inspection are recovery-only after an explicit schema/tool mismatch.

## Default decision rule

| Intent | First choice |
|---|---|
| open/navigate/start a browser task | `browser` |
| one new semantic observation | `snapshot` |
| deterministic multi-step work | `workflow` or `browser open` with `steps` |
| one isolated mutation | `act` |
| bounded structured page data | `extract` |
| prove an uncertain result | `verify` |

For ordinary tasks, optimize **user prompt to verified result**, not only browser execution time.

Preferred shape:

`most specific destination -> one targeted observation if needed -> one deterministic workflow -> verification only if still needed -> release`

Do not visit a homepage when a user-provided, observed, or qualified Controlla route already identifies the specific destination. Do not take duplicate broad snapshots. Do not return deterministic intermediate state to the model just to issue the next obvious browser command.

## Canonical hot-path calls

Cold foreground open:

```json
{"action":"open","mode":"foreground","url":"https://example.com/specific-page","idempotency_key":"goal-specific-stable-key"}
```

If the next steps are already known, include them in that same `browser open` rather than waiting for another model turn.

Targeted semantic read after open:

```json
{"session_id":"<session>","chrome_tab_id":"<tab>","goal":{"query":"target words","roles":["link"],"exhaustive":false},"max_items":30,"max_text_chars":0,"max_scan_nodes":12000}
```

Use `max_text_chars:0` when page prose is not needed. Increase coverage only when the task requires it.

One isolated click:

```json
{"session_id":"<session>","chrome_tab_id":"<tab>","action":"click","reference":"@c1","outcome":{"kind":"navigation"}}
```

Structured extraction uses the current session/tab plus a CSS `selector`; add a `fields` map only when named fields are needed. Do not inspect source code to discover this.

A workflow uses the current `session_id`, `chrome_tab_id`, and `steps`. Save found references with `save_as` and reuse them as `$name` inside later workflow steps. Prefer a single workflow to repeated `snapshot -> act -> snapshot` loops.

## Newest/latest content

Use this route:

**direct specific page -> one targeted candidate read -> complete candidate set when required -> choose by observed publication evidence -> click -> verify**

For exact publication metadata, `find_all` the narrow candidate set and `choose` with `key:"datetime"`. Do not infer newest from DOM order. If a site exposes only relative ages such as `2h`, use only comparable observed evidence and report that limitation instead of inventing an exact timestamp.

Example behavior for “open the newest Buffalo Bills story on ESPN”: if a qualified/verified Bills destination is available, open that page directly. Do **not** open ESPN home, take general homepage snapshots, browse menus, then rediscover the Bills page. Read the Bills story candidates once, choose the newest supported candidate, click it, and verify the destination.

## Direct-route safety

A direct destination is allowed when it is:

1. supplied by the user,
2. an observed href from current page evidence, or
3. supplied by Controlla's qualified route/skill memory.

Do not invent unverified application routes. If no qualified route exists, perform the minimum semantic exploration needed to discover one. Successful independently verified routes/workflows may be learned for later reuse; failed, drifted, credential-bearing, or ambiguous routes must not be replayed.

## Verification and recovery

If a mutation receipt already proves the declared postcondition, do not verify it again. If it returns `unknown` or only `dispatched`, perform one fresh read-only verification and never replay the mutation. `not_dispatched` may be corrected according to its returned `reason_code`, `retry_class`, and `next_action`.

Load `references/recovery.md` only after a failure or schema/tool mismatch.

## Rare paths

Load these only when the task requires them:

- `references/common-recipes.md` for additional execution patterns
- `references/uploads-and-frames.md` for files, app acceptance, account identity, or cross-origin pickers
- `references/advanced.md` for workflow VM control flow, virtual lists, page tools, modes, typing details, or credentials

## Cleanup

Release the Controlla shared session when finished and preserve the user's tab. A normal browser task should require zero repo reads, zero schema/describe calls, zero MCP-discovery calls, and normally no more than one model-visible observation before deterministic execution when the destination is already known.
