# Roadmap

## Implemented and locally tested

- Bounded screenshot stability service with normalized RGBA hashing, same-document identity checks, active-renderer checks, region/timing limits, and timeout receipts.
- `visual_stable` open readiness and read-only workflow `wait_stable` step.
- Compact generated route manifest and no-safe-existing-tab guidance without a required discovery call.
- Strict native action envelope, policy/identity validation, bounded refusal and uncertain-delivery receipts, and platform-provider interface.

## Capability status

- Semantic and page-tool routes: available according to live page evidence and registered capability state.
- Visual capture: available only where Controlla's paired screenshot path verifies the active renderer and document.
- Headless: available only when the runtime has a configured Chrome executable.
- Native desktop fallback: disabled on all platforms. The experimental macOS Swift helper has no stable signed app identity, so the runtime refuses native input and `describe` reports the capability unavailable. Do not grant permissions to its ephemeral helper. Chrome page, tab, and window actions use the extension and do not require Accessibility.
- Live ESPN and browser-window tests: require the user's paired Chrome extension and are not implied by local unit or plugin tests.
- Hosted CI: report from the current branch's CI results; local tests do not imply hosted green status.

## Next platform milestone

Package a stable signed macOS helper identity before exposing native input. Then qualify permission reporting and Apple Events delivery in a live foreground test, and add real desktop-window coverage. Exercise uncertain delivery without replay. Windows and Linux remain explicit refusals until separately qualified.
