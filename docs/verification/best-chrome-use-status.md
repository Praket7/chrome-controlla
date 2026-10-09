# Best Chrome Use Verification Status

This file records implementation and evidence status without converting missing evidence into a product claim.

## Implemented engine foundations

- execution telemetry and compact runtime metrics
- semantic snapshots, short revision-bound references, and deltas
- verified fast-input policy with strict fallback classification
- bounded deterministic browser-workflow contract
- runtime-owned verification primitives
- persistent qualified-skill store and fail-closed replay policy
- targeted visual-probe references
- structured capability routing
- adaptive per-target scheduling
- compact MCP v2 tool surface
- packaged ChatGPT Desktop local plugin and v2 server
- difficult-browser-class qualification matrix
- Phase 11 realistic benchmark contract and paired analysis
- Phase 15 drift and fault-injection contract
- release gate that disables superiority claims when measured evidence is absent

## Safety invariants

A release candidate is blocked from a broad superiority claim if any measured severe wrong-target event, authority violation, or silent stale-reference mutation occurs. Unknown mutation delivery is never silently retried. Caller-supplied evidence is not treated as independent verification. Structural drift quarantines qualified skills.

## Evidence currently allowed

The repository contains narrow local fixture evidence for native shared-tab pairing, guarded fill, strict typing, fixture slide interactions, mock post behavior, navigation invalidation, bounded readback, release, and cleanup. This evidence qualifies those fixture paths only.

## Evidence still external

Remote Google Slides, Canva, and CapCut Web mutation plus reload persistence remain pending until those applications are exercised with independent verification. Competitive Phase 11 measurements against the declared adapter set are also external benchmark work. Until a measured `phase11-results.json` passes the claim gate, the repository must not describe Controlla as universally faster or better than every browser agent.

## Claim policy

The non-claim release check validates code, contracts, safety gates, benchmark configuration, and the fact that the claim gate is fail-closed. A public superiority claim requires running the release gate with measured Phase 11 paired results. If the complete gate does not pass, only narrower measured claims may be made.
