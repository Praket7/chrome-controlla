# Compact Agent Surface v3

Controlla v3 exposes six default concepts to an agent and keeps implementation-specific diagnostics out of the default schema budget.

| Tool | Purpose |
| --- | --- |
| `browser` | Create, attach, select, and inspect browser sessions and explicit target authority. |
| `snapshot` | Return compact semantic state or a revision-bound delta. |
| `act` | Perform one guarded action such as click, fill, type, press, select, upload, or navigate. |
| `workflow` | Execute a bounded deterministic action graph or near-page batch with partial receipts. |
| `extract` | Return bounded structured data, URLs, citations, or artifact metadata. |
| `verify` | Independently observe and evaluate postconditions after a mutation. |

The serialized default schemas must stay at or below 48 KiB. Results must stay at or below 512 KiB before transport, and normal semantic results should be substantially smaller. Screenshots, raw trees, tracing, skill administration, page-tool diagnostics, and benchmark controls are negotiated capabilities rather than default tools.

## Execution hierarchy

Controlla chooses the highest reliable route in this order: native page tool or WebMCP capability, qualified deterministic skill, compact semantic workflow, DOM or accessibility grounding, bounded visual probe, then strict low-level input. Deterministic operations never require a second model.

## Safety contract

Every mutation is tied to explicit target authority, document identity, and a grounded interaction epoch. A stale document, focus loss during key dispatch, conflicting target lease, or unknown mutation-delivery state stops the workflow. Unknown mutations are never retried automatically. Page-provided tools are treated as untrusted privileged capabilities with allowlists, schema validation, timeouts, cancellation, output budgets, and the same authority checks as UI automation.

## Browser modes

Foreground, background-headed, and headless sessions share one semantic and verification contract. Background mode must not activate the browser window unless the caller explicitly requests foreground interaction. Headless recovery is bounded and cannot replay a mutation whose delivery is uncertain.

## Typing modes

`Block` performs constant or chunked insertion for ordinary controls. `FastKeys` preserves ordered key event semantics with zero intentional per-character delay and periodic focus revalidation. `HumanKeys` adds bounded caller-selected delay or jitter. `Ime` preserves composition semantics. Strategy selection is explicit in receipts so a caller can distinguish speed optimizations from human-paced input.

## Multi-client behavior

Reads may share a target. Conflicting mutations require a target/document lease. Separate tabs may execute concurrently. A client cannot silently steal mutable authority from another client, and disconnect cleanup never expands authority.

## Bounded browser program (2026-10-10)

`workflow` is a single shared contract compiled before execution. Vocabulary: `navigate`, `snapshot`, `find`, `find_all`, `choose`, `click`, `fill`, `type`, `press`, `select`, `wait_for`, `observe`, `extract`, `assert`, `verify`, `checkpoint`, `page_tool`. `script` remains isolated and negotiated.

Typed variables: `Reference`, `References`, `String`, `Number`, `Bool`, bounded `Object`/`Array` (max 64 keys / 128 items). Control flow: `if`, `foreach` (max 32 items, explicit cardinality), `try_fallback` (fallback only after `not_dispatched`, never after `unknown`), `parallel_read` (read-only only), `wait_until`, `project`. Hard limits: no recursion, no unbounded loop, max 64 expanded steps, every probe/candidate/branch bounded at compile or request time.

Collection: `find_all` returns bounded references with scoring metadata; `choose` selects deterministically by `datetime` (HTML `time[datetime]` + visible timestamp parsing), `dom_order`, `numeric`, `text`, returning the selected reference plus freshness evidence or `unresolved_latest` when no trustworthy ordering exists. Verification supports URL equals/prefix/normalized, title equals/contains, selected href equals resulting URL, element/text/value/visible/selected/expanded predicates, producing a compact `EvidenceReceipt` (candidate title/href/freshness, resulting URL/title, verified boolean). Visual probes (`element_focus`, `region_focus`) are bounded inside `snapshot`/`workflow` and evidence memory holds only probed/acted/committed bytes (max 3 / 512KiB).
