# Uploads, artifacts, and cross-origin frames

Load this only for file transfer, upload acceptance, account-sensitive actions, or cross-origin picker work.

## Artifacts

Register the exact artifact once in the same Controlla session. Prefer compact base64 transfer when bytes must cross MCP. Artifact handles are session-scoped and must not be reused across sessions.

## Shared-session upload

In `local_unrestricted` shared sessions, upload through an observed visible trigger and require an explicit application-acceptance predicate. Chrome file selection alone is not application acceptance, and application acceptance alone is not persistence.

If an upload result is `unknown`, inspect once and do not replay the upload or a later post/submit action.

## Guarded upload

Guarded uploads use the dedicated private headless task route. Preserve account/document identity checks and refuse ambiguous file inputs.

## Account identity

Use one exact visible identity marker from the active page. Prefer a unique CSS selector. When no unique selector exists, an exact visible root-DOM button accessible name may be used. Picker selection/highlight is not proof of the signed-in account.

## Cross-origin picker

Start with the ordinary semantic snapshot. If a picker is cross-origin, obtain observed child `frame_id` values and request a bounded snapshot for the specific frame. Frame references are loader-bound and become stale when the frame loader changes.

Do not load this reference for ordinary navigation, reading, clicking, or form tasks.
