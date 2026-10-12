# Controlla recovery reference

Load this only after a normal fast-path call returns an error or an ambiguous mutation receipt.

## Error classes

- `not_dispatched`: the side effect did not run. Correct the stated cause. Refresh semantic state only when `retry_class` says `refresh_snapshot`.
- `unknown`: delivery may have occurred. Never replay the mutation. Run one fresh read-only verification or inspection.
- `dispatched`: input was acknowledged but the requested page-level outcome is not proved. Verify once.
- schema/tool mismatch: only here may the agent inspect the live schema or diagnose MCP registration. Do not probe random argument combinations.

## Recovery order

1. Use `reason_code`, `retry_class`, and `next_action` from the receipt.
2. If state may be stale, take one targeted snapshot rather than a broad page snapshot.
3. Reacquire a semantic reference by stable identity if available.
4. Use a targeted visual probe only after semantics fail.
5. Stop rather than replaying an unknown side effect.

## Schema/runtime mismatch

Routine browser work must not perform schema discovery. If the runtime explicitly reports a schema mismatch, reload/reconnect the MCP pair. The six public tools are `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`.

Do not read Rust or JavaScript source as a substitute for the registered tool schema. Source inspection is a developer/debugging activity, not an execution step.
