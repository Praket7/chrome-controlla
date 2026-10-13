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

1. Start with `browser`. Prefer the most specific destination rather than a site's homepage.
2. A destination may come from the user, an observed href, qualified Controlla route/skill memory, **or a high-confidence public route synthesis**. Speculative public routing is allowed only for bounded HTTP(S) URLs with no credentials/secrets and must be verified against the requested entity/page immediately after navigation. If verification fails, discard the route and fall back once to semantic discovery. Never synthesize account indexes, private/authenticated routes, destructive-action endpoints, or hidden application state.
3. Observe only what the next decision needs. Prefer one targeted semantic snapshot with bounded items, scan nodes, and `max_text_chars:0` when body text is unnecessary. Do not take broad duplicate snapshots.
4. Put deterministic multi-step work in one `browser open` with `steps` or one `workflow`. Do not bounce intermediate deterministic state through the model.
5. Verify consequential mutations only when the action/workflow receipt does not already prove the requested postcondition. Never replay an `unknown` mutation.
6. Release the shared session without closing the user's tab.

For newest/latest content, the preferred shape is **direct destination -> one targeted candidate read -> choose by observed publication evidence -> click -> verify**. Avoid homepage navigation, repeated general snapshots, source-code inspection, and manual schema discovery. If exact timestamps are unavailable, report the actual freshness evidence used rather than inventing precision.

A normal browser task should have zero repository reads, zero schema/describe calls, zero model-visible MCP-discovery calls, and at most one model-visible observation before a deterministic workflow whenever the target is already known or can be safely routed. Guarded cold open performs its fresh target inventory internally as part of `browser action=open`; do not call discovery separately unless intentionally selecting a particular existing tab. Treat unnecessary repo reads, documentation searches, broad snapshots, and navigation detours as performance bugs.

Successful speculative routes become reusable only after independent page-identity verification. Persisted route/skill memory must be bounded, origin-scoped, expiring, and quarantined on structural drift or repeated verification failure. Failed, ambiguous, credential-bearing, or sensitive routes must not be learned or replayed.

Recovery remains fail-closed: `not_dispatched` may be corrected and retried; `unknown` requires fresh read-only verification and must not replay the side effect. Password values never belong in snapshots, workflows, logs, tests, or persistent route/skill memory.
