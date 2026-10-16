# Verified Replay Cache Design

## Goal
Reduce repeated-task latency and browser round trips while preserving Controlla's target identity, lease, cancellation, and independent-verification contracts.

## Behavior
- Cache only verified, non-secret semantic plans.
- Key entries by origin, current structural signature, normalized intent, and action configuration.
- Reacquire every target against the current document before dispatch.
- Dispatch a valid replay in one bounded batch where the adapter supports it.
- Independently verify every stored postcondition after dispatch.
- On stale identity, target mismatch, or failed postcondition, invalidate the entry and perform one fresh observation path; never blindly replay or retry an unknown effect.
- Passwords, password values, account secrets, and chat credentials are never cached, logged, or persisted. Secure fill remains a one-shot foreground operation with origin and account checks.

## Metrics
Every extraction replay route reports a bounded `cache_status` (`hit`, `miss`, `stale`, or `healed`) and whether fresh observation was required. Metrics contain no page secrets. Mutation replay continues to use the existing qualified-skill path, whose step receipts independently verify every effect.

## Compatibility
Existing qualified skills, WebMCP routes, deterministic workflows, leases, checkpoints, and response-budget contracts remain authoritative. The replay cache is advisory and may only short-circuit a path after live target reacquisition succeeds.

## Acceptance
- Repeated identical read or safe interaction tasks use fewer browser round trips after the first verified run.
- Any structural or target drift causes a safe miss and fresh resolution.
- Unknown effects are never replayed.
- Secure-fill values never appear in cache state or cache metrics.
- Existing full runtime, benchmark, release, and live fixture suites remain green.
