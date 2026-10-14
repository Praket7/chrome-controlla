# Chat-provided password entry and browser latency design

## Goal

Let an agent use a password the user supplied in chat to fill one freshly identified Chrome password field, while keeping that value out of Controlla-generated observations, workflows, persistent state, logs, traces, and receipts. In parallel, improve the fidelity of prompt-to-outcome evidence and reduce measured avoidable overhead in the simple browser path.

This does not erase or redact the user's original chat message. The MCP client may retain tool-call arguments, and the password exists transiently in client, MCP, extension, and Chrome process memory. Controlla will document this boundary and will not claim secure erasure of process memory.

## Password input contract

- `act` accepts a one-shot password value only for `action=secure_fill`; ordinary `fill` continues to refuse password fields and password values.
- The secret is never accepted by `workflow`, snapshots, extraction, route/skill learning, or persistent evidence.
- The runtime validates the fresh reference and foreground session, then forwards the secret only to the paired extension. Existing checks for target tab, document, frame loader, field type, visibility, emptiness, editability, authorization lease, and independent readback remain mandatory.
- When no one-shot value is supplied, the current extension-owned local prompt remains available.
- Both success and failure receipts contain only status and non-secret verification evidence. The password and its length are omitted.
- Trace, test, error, and diagnostic paths must not serialize raw tool arguments or secret-bearing extension messages. Tests use synthetic sentinels only to assert non-leakage.
- Best-effort owned-buffer cleanup is allowed, but no guarantee is made that allocators or downstream Chrome internals erase copies.

## Latency and measurement

- Existing CLI runs are labeled `process_invocation_to_*`, not user prompt-send timing. A true `prompt_sent` span is emitted only by a client adapter that exposes the actual submission boundary; otherwise the report clearly marks it unavailable.
- Reports retain per-client and per-stage spans, input/context volume, MCP calls, browser commands, host round trips, verification, fallback, and cold/warm classification. Unknown internal model-call counts fail the qualification gate.
- Trace the current two-step navigation/extraction route by phase, identify repeated browser-host operations, and batch or remove only operations shown to be redundant while retaining identity checks and independent verification.
- Compare the modified route with the checked-in pilot on the same task and environment. The pilot is small and process-start based; it is not a universal speed claim.
- Reduce pairing latency only with a cross-platform mechanism that has bounded resource use and a regression test; otherwise retain polling and report its measured contribution.

## Smoke-runner handling

- Do not pass sensitive prompt contents through a process argument if the installed client offers a supported stdin or equivalent input channel.
- If no safe channel exists, mark that adapter as unsuitable for sensitive prompt tests and keep secrets out of its fixtures. Hashing or truncating prompt text is not a substitute for preventing OS argument exposure.

## Acceptance criteria

1. `secure_fill` accepts a one-shot value and the existing local prompt fallback; other actions and workflow fields cannot use it.
2. Regression tests prove the synthetic secret is absent from MCP receipts, extension diagnostics, trace files, persisted data, and ordinary snapshots, and that stale/changed targets refuse or return unknown without blind retry.
3. Credential input is dispatched once to the exact guarded field and verified with redacted evidence; failure after possible dispatch returns unknown.
4. Prompt timing never labels process invocation as user SEND. Actual SEND timing is present only where the adapter can observe it.
5. Missing internal model-call telemetry cannot pass the zero-call qualification gate.
6. The simple-route command/round-trip profile is measured before and after, with correctness and verification retained.
7. Smoke adapters do not put sensitive prompt text in process listings when a supported safe input channel exists.
8. Pairing changes are retained only if latency improvement is demonstrated without unacceptable scan/resource cost.

## Out of scope

- Removing or editing the user's chat history.
- Claiming end-to-end secrecy from the model/client or Chrome after a password is supplied in chat.
- Password storage, vaulting, automatic login, password discovery, or reuse across pages.
- A universal fastest-browser claim or a broad competitor qualification from the existing one-task pilot.
- Unrelated Controlla cleanup or behavior changes.
