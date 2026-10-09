# Phase 9 B35: Long-task novice scenarios

## Method and scope

Fresh blind documentation/schema evaluation using current `docs/MASTER_GUIDE.md` version `master-2026-10-06-v3` and the rebuilt packaged executable `packages/chrome-controlla/dist/bin/controlla-core`. I started it as an MCP server with a newly created temporary `CONTROLLA_STATE_DIR` and sent JSONL `initialize`, `notifications/initialized`, then `tools/list`. The initialize response identified `controlla-runtime` version `0.1.0`. I inspected the returned schemas for `extract`, `workflow`, `workflow_status`, `artifact_register`, and `file_select`. No browser or app connection was made.

Outcomes judge whether the current guide and advertised tool schemas explain the novice task. They are not execution results or evidence of live app behavior, real-client usability, or successful browser operation.

## Scenarios and outcomes

### 1. Extract a virtualized list with authoritative count and resume cursor — PASS

**Evidence:** `MASTER_GUIDE.md` §8 requires stable IDs while traversing the correct scrolling container, an independently authoritative expected count, a matching account marker, and an in-container terminal marker at scroll end across two observations with no new IDs. It warns against treating recycled rows, a count match, or scrollbar bottom alone as complete evidence. It documents single-use cursors bound to target, revisions, and extraction spec.

The `extract` schema has per-section `expected_count`, `account_marker`, `terminal_selector`, `cursor`, and `id_field`; bounded fields include `max_steps`, `max_records`, `max_text_chars`, and `max_bytes`. Top-level fields include `sections`, `max_records`, `max_bytes`, and `timeout_ms`. The tool description says each section reports completeness independently. In the current guide §6, an extraction request example shows a section spec, and the follow-up example says to repeat with the same target and section spec, replacing only `cursor` with the returned token.

**Smallest gap:** The runtime accepts the caller-declared expected count and account marker; the guide says the count must be independently authoritative, but the tool does not independently establish their provenance.

### 2. Understand partial results under aggregate and per-section limits — PASS

**Evidence:** `MASTER_GUIDE.md` §8 describes deterministic per-section record/byte budgets and a global deadline. It says skipped or truncated sections report missing coverage, any limit truncation prevents `complete`, and blocked expansion or a deadline returns found rows as `partial`.

The `extract` schema exposes `max_records` and `max_bytes` inside each section spec and again at the top level, plus per-section `max_steps` and `max_text_chars` and top-level `timeout_ms`.

**Smallest gap:** `extract` has an empty `outputSchema`, so the exact structured result/status shape is described in guide prose rather than the schema.

### 3. Handle a long asynchronous workflow through the actual status tool — PASS

**Evidence:** `MASTER_GUIDE.md` §9 says long operations return `accepted` or `running` with an operation ID, directs callers to `workflow_status` with the original session and operation IDs, and says there is no `jobs.wait` or reason to resubmit after a timeout.

The packaged server's `tools/list` includes `workflow_status`, described as returning durable status and the latest persisted checkpoint. Its schema requires `session_id` and `operation_id`.

**Smallest gap:** The guide says to wait briefly between polls but does not specify an interval or backoff schedule.

### 4. Recover after an unknown outcome without retrying — PASS

**Evidence:** `MASTER_GUIDE.md` §9 directs the caller to inspect the original operation's latest checkpoint, target/revisions, and delivery through `workflow_status`. It says not to retry a possibly dispatched effect unless an authorized independent read proves it absent or a qualified endpoint guarantees idempotency; otherwise preserve `unknown` and request human reconciliation. It explicitly says this preview has no app-specific independent observer and a receipt cannot prove a saved application result.

The `workflow` schema requires an `idempotency_key`; `workflow_status` requires the original `session_id` and `operation_id`.

**Smallest gap:** No independent app-state reconciliation tool is present in this preview; the guide identifies that limit.

### 5. Distinguish an inline workflow artifact from page file selection, upload, or save — PASS

**Evidence:** `MASTER_GUIDE.md` §§1, 6, and 12 distinguish inline workflow artifact bytes in a receipt from registering bytes with `artifact_register` and selecting them with `file_select`. The guide says selection uses the page's existing file input and confirms only Chrome selection/name/size readback, not upload, app acceptance, or saving; neither route is a download/export mechanism or accepts arbitrary host paths.

The `workflow` schema supports a `script` step. `artifact_register` requires `session_id`, `filename`, and `bytes`. The rebuilt `file_select` schema now types `locator` as a reference to `FileSelectLocator` (`anyOf` supported locator forms); for example, the guide §6 shows `{"selector":"input[type='file']"}`. `file_select` also requires `session_id`, `target_ref`, `artifact_handle`, and `account_marker`; its tool description says selection/readback is not app acceptance.

**Smallest gap:** No upload, app-acceptance, or persistence verifier is available in this preview; the guide makes this boundary explicit.

## Limits

This run verified only the current guide and schemas returned by `tools/list` from the rebuilt packaged executable, using isolated temporary state. It did not call browser tools, connect to an app, or test a real MCP client. Documented extraction/workflow behavior is not evidence of representative-site qualification or external-client acceptance; `MASTER_GUIDE.md` §1 leaves those open.
