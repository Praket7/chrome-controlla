# Repository instructions

- Keep this project limited to Chrome and web apps running in Chrome.
- Treat `docs/design/` as proposals; claim support only after the matching evidence gate passes.
- Preserve Comptrol Apache-2.0 attribution for extracted files and update `provenance/extraction.json` when extracted source changes.
- Keep Rust at the version in `rust-toolchain.toml`; keep direct dependency versions exact and commit `Cargo.lock` changes.
- Do not add monolithic Comptrol, native desktop automation, credentials, browser profiles, or unrelated assets.
- Update `docs/progress.md`, `docs/verification-matrix.md`, and `docs/blockers.md` at phase gates. Retain failed evidence.

## Controlla browser fast path

Controlla's six V3 tools are a stable agent contract: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. The compact browser skill is `packages/chrome-controlla/plugin/skills/browser-control/SKILL.md`.

For ordinary browser tasks, **act first**. If the six tools are already exposed by the client, do not list MCP servers, inspect schemas, call `browser describe`, read `MEMORY.md`, search this repository, or open Rust/JavaScript source to learn how Controlla works. Do not reread the browser skill between normal calls. Tool/schema/source inspection is recovery-only after an explicit `schema_mismatch`, `tool_unavailable`, malformed-tool error, or other diagnostic that cannot be resolved from the returned receipt.

Default execution policy:

1. Start with `browser`. Prefer the most specific destination that is user-provided, observed, or supplied by a qualified/verified Controlla route. Do not visit a site's homepage merely to rediscover a known destination.
2. Observe only what the next decision needs. Prefer one targeted semantic snapshot with bounded items, scan nodes, and `max_text_chars:0` when body text is unnecessary. Do not take broad duplicate snapshots.
3. Put deterministic multi-step work in one `browser open` with `steps` or one `workflow`. Do not bounce intermediate deterministic state through the model.
4. Verify consequential mutations only when the action/workflow receipt does not already prove the requested postcondition. Never replay an `unknown` mutation.
5. Release the shared session without closing the user's tab.

For newest/latest content, the preferred shape is **direct destination -> one targeted candidate read -> choose by observed publication evidence -> click -> verify**. Avoid homepage navigation, repeated general snapshots, source-code inspection, and manual schema discovery. If exact timestamps are unavailable, report the actual freshness evidence used rather than inventing precision.

A normal browser task should have zero repository reads, zero schema/describe calls, zero MCP-discovery calls, and at most one model-visible observation before a deterministic workflow whenever the target is already known. Treat unnecessary repo reads, documentation searches, broad snapshots, and navigation detours as performance bugs.

The route safety rule is: use a user-provided URL, an observed href, or a route produced by Controlla's qualified route/skill memory. Do not invent an unverified application route. If no qualified direct route exists, use the minimum semantic exploration required to discover one, then allow successful verified routes/workflows to be learned for later reuse.

Recovery remains fail-closed: `not_dispatched` may be corrected and retried; `unknown` requires fresh read-only verification and must not replay the side effect. Password values never belong in snapshots, workflows, logs, tests, or persistent route/skill memory.
