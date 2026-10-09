---
name: browser-control
description: Use when controlling Chrome through Controlla; preserves explicit tab authority, fresh semantic references, verification, and unknown-delivery safety.
---

# Chrome browser control

Use the compact Controlla tools as a safety-preserving hierarchy:

1. Start with `browser_session` and explicitly discover and pair only the tabs the user selected. Seeing a tab is not authority to mutate it.
2. Prefer `browser_snapshot` and `browser_find`. Use only the exact `@cN` references returned by the latest semantic state.
3. Use `browser_workflow` for bounded multi-step work when the references and expected state are known.
4. Use `browser_act` for guarded mutations. Supply expected values when the schema supports them.
5. After a mutation, use the returned fresh state or `browser_verify` before claiming success.
6. If any mutation returns `unknown`, inspect the browser state before deciding what to do next. Never automatically retry an unknown outcome.
7. Use `browser_probe` only when semantic state is insufficient; avoid full-page visual work by default.
8. If navigation, document replacement, human edits, or target drift invalidate a reference, take a fresh snapshot and resolve the target again.
9. Do not bypass selected-tab authority, stale-reference checks, or verification by switching to lower-level browser access.
