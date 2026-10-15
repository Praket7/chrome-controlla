# Uploads, artifacts, and cross-origin frames

Load this only for file transfer, upload acceptance, account-sensitive actions, or cross-origin picker work.

## Artifacts

Register the exact artifact once in the same Controlla session. Prefer compact base64 transfer when bytes must cross MCP. Artifact handles are session-scoped and must not be reused across sessions.

## Shared-session upload

In `local_unrestricted` shared sessions, upload through an observed visible trigger. `acceptance` and `persisted` are separate optional predicates: each is measured before selection and must transition from unsatisfied to satisfied afterward. Use `persisted` only for an app state that indicates the attachment was saved; the receipt reports `application_accepted` and `persisted` independently. These page predicates are UI evidence, not server-side proof. Chrome file selection alone is not application acceptance, and application acceptance alone is not persistence.

If an upload result is `unknown`, inspect once and do not replay the upload or a later post/submit action.

## Guarded upload

Guarded uploads use the dedicated private headless task route. Preserve account/document identity checks and refuse ambiguous file inputs.

## Account identity

Use one exact visible identity marker from the active page. Prefer a unique CSS selector. When no unique selector exists, an exact visible root-DOM button accessible name may be used. Picker selection/highlight is not proof of the signed-in account.

## Cross-origin picker

Start with the ordinary semantic snapshot. If a picker is cross-origin, obtain observed child `frame_id` values and request a bounded snapshot for the specific frame. Frame references are loader-bound and become stale when the frame loader changes.

Do not load this reference for ordinary navigation, reading, clicking, or form tasks.
Shared snapshot frame handles expose `target_id` only when the provider can prove that mapping. A `null` child `target_id` means Page.getFrameTree supplied ancestry but no reliable OOPIF target identity; keep routing by the explicit `frame_id` through the provided frame operation and do not substitute the root tab ID as the child's target. Session IDs and measured transforms remain unavailable unless the provider reports them.

Screenshot `image_sha256` and the compatibility `visual_revision` alias identify decoded PNG bytes only. `render_revision` is null until a renderer/app freshness signal exists, and `target_lifecycle="active"` means the extension observed the tab as complete and unfrozen around capture. Frozen or unknown lifecycle is refused; a freeze during capture discards the image. For chunked transfers, `base64_sha256` separately verifies the exact Base64 text delivered across the native-messaging boundary. Width and height are read from the PNG header and checked against the visual evidence.
