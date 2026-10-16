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

## Lease and target cleanup

- `already leased`, `already paired`, or `target occupied` means another live
  Controlla session owns that tab. Do not retry `pair_shared` with the same
  target. Call `browser action=list` to identify the owning session if the
  current client exposes it; otherwise release the session that created the
  lease with `browser action=release`, then rediscover the tab and pair it
  again with its current document identity.
- A reconnect acknowledgement is not proof of repair. If `native-status` or a
  recovery receipt reports `recovery_required`, rediscover and re-pair the
  exact tab. The recovery journal is cleared only after that owner-side
  identity check; do not treat a stale lease ID as reusable.
- Always release the session after the task. Release detaches Controlla from
  user tabs; it does not close them. If release reports cleanup pending, use
  the returned cleanup action once, then inspect instead of creating a second
  session.
- `unknown` clicks cannot be made verified by relabeling the receipt. Read the
  requested postcondition once with `verify` or a targeted snapshot. If it is
  present now, report the state as observed and keep causality unproven; if it
  is absent, stop rather than clicking again.
