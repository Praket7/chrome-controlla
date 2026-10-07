# Phase 9 B35 extraction and file selection usability review

## Method

- Read the current `docs/MASTER_GUIDE.md` extraction and file selection sections before using the packaged MCP.
- Started `packages/chrome-controlla/dist/bin/controlla-core mcp` with a fresh temporary `CONTROLLA_STATE_DIR`.
- Sent JSONL MCP `initialize` with protocol version `2025-11-25`, followed by `notifications/initialized` and `tools/list`.
- The process exited successfully with no stderr. Repeated the packaged MCP query while recording this note to check the current `file_select` schema.
- No browser or application was connected. Results below are a documentation/schema usability evaluation, not live page behavior.
- Follow-up run: repeated the same JSONL initialization and `tools/list` sequence against the freshly packaged executable with temporary state at `/var/folders/yt/j9qyd4k52m53js3dxv_xr4200000gq/T/controlla-b35-rerun-0x_u_grh`. The process exited successfully with no stderr; `tools/list` returned 16 tools. The live `file_select` schema used `FileSelectLocator` with nine `anyOf` alternatives and `additionalProperties: false` on each variant.

## Scenarios

| # | Scenario | Result | Guide and live schema evidence |
|---|---|---|---|
| 1 | Extract records from a virtualized list where rows recycle as the user scrolls. Identify the scrolling container and decide when traversal supports “all items.” | **Pass** | Guide §2 line 55 and §8 lines 178–184 specify scanning the actual scroll container by viewport, deduplicating stable IDs, and treating rendered rows or a bucket count alone as insufficient. The live `extract` schema requires `container`, `record`, `id_field`, and `max_steps`; it also exposes per-section and aggregate record/byte limits. |
| 2 | Decide whether a displayed count of 42 is authoritative when only eight rows have been observed. | **Pass** | Guide §6 line 145 says the count must come from an independent authoritative source, not the traversed rows. §8 lines 178 and 182 require an independently authoritative count matching unique IDs, matching account identity, and a terminal marker at scroll end across two observations with no new IDs. The schema permits `expected_count` to be omitted or null, allowing uncertain totals to remain unknown. |
| 3 | Resume a partial extraction without silently changing its scope or restarting after a rejected cursor. | **Pass** | Guide §6 lines 145 and 158 instructs using that section’s cursor with the same section ID and unchanged spec, replacing only the cursor. If it is missing, stale, expired, or rejected, reobserve identity/state before starting again. The live schema places nullable `cursor` in each section’s extraction spec. |
| 4 | Register bytes, select them into the intended file input, and state what the result proves. | **Pass** | Guide §1 line 29 and §6 lines 162–165 say to register bounded bytes, use the returned opaque handle with a complete current Direct CDP target reference and account marker, and locate the actual `<input type="file">`. They distinguish browser selection from transfer, app acceptance, and persistence; no host path or native picker is used. Live `tools/list` confirms `artifact_register` requires `session_id`, `filename`, and `bytes`; `file_select` requires `session_id`, `target_ref`, `locator`, `artifact_handle`, and `account_marker`. In the follow-up package, `locator` references `FileSelectLocator`, an `anyOf` with nine typed alternatives (role/name, label, placeholder, text, test ID, alt text, href substring, CSS selector, backend node ID). Each variant disallows additional properties, matching the guide’s supported locator forms and strictness claim. |
| 5 | Handle a partial extraction or an operation whose effect may have occurred before a timeout. | **Pass** | Guide §8 lines 182–184 says mismatched identity is unknown, constrained results are partial, and partial rows plus missing coverage should be returned. §9 lines 188–190 directs polling `workflow_status` for the original session and operation and says not to equate timeout with no effect. The live schema confirms `workflow_status` accepts the original `session_id` and `operation_id`. |

## Result

All five task scenarios were answerable from the guide and packaged tool listing without a browser connection. The follow-up package exposes the documented typed, strict locator alternatives in its live schema. No browser or app behavior was exercised. No files other than this review note were edited.
