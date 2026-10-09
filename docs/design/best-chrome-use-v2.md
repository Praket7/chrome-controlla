# Best Chrome Use v2

## Goal

Controlla is designed as a verified execution engine for AI-driven Chrome work. The priority is not to maximize raw action count. The priority is to complete browser work with low model overhead while preserving target identity, authority, stale-reference rejection, bounded recovery, and independent verification.

## Capability hierarchy

The runtime chooses the cheapest qualified route in this order.

1. Native or structured app capability
2. Qualified deterministic skill
3. Semantic workflow using revision-bound references
4. DOM and accessibility grounding
5. Targeted visual probe
6. Strict low-level input

A faster route never bypasses the existing target, document, frame, principal, browser-generation, or authority checks.

## Semantic state

Semantic snapshots expose compact, stable short references rather than raw page HTML. References are bound to the current browser, target, frame, document, and semantic snapshot revisions. Navigation, renderer replacement, node replacement, authority change, or browser-generation change invalidates references. Delta output is preferred after the first snapshot so repeated work does not resend the entire page state.

## Input strategy

Ordinary text controls may use a verified fast path when the exact target and expected value still match. Masked, custom, trusted-event, contenteditable, IME-sensitive, or otherwise unqualified controls fall back to an adapter or strict input path. Ordinary text entry must be O(1) or O(chunks) in browser protocol calls rather than O(characters) whenever the verified fast path is safe.

## Browser workflows

The v2 workflow contract is bounded to twenty steps and supports navigation, finding, clicking, filling, typing, key presses, selection, waits, observation, extraction, assertions, verification, checkpoints, and isolated scripts. Symbolic references must be defined before use. Scripts cannot be mixed with other steps. Navigation is limited to credential-free HTTP(S) URLs. Live authority and revision checks are still repeated at execution time.

## Reusable skills

A reusable skill starts as a candidate and can become qualified only after distinct successful training and validation runs plus trusted runtime verification. Qualified replays use zero Controlla internal model calls. Exact site scope and structural signatures are preconditions. Structural drift, severe safety failures, expiry, or a failed rolling verification window quarantine or expire the skill instead of replaying it blindly.

## Visual fallback

Screenshots are not the default observation surface. Visual work is scoped to the smallest useful element or region and is bound to the current target and document revision. A visual reference expires when the underlying page identity changes.

## Scheduling

Serialization remains per target and per document. Independent tabs may execute concurrently. Adaptive scheduling can raise or lower concurrency based on browser latency and failures, but resource pressure never weakens stale-target or authority checks.

## Verification and uncertain delivery

Caller-provided evidence is not independent verification. Mutating workflows require runtime-owned fresh evidence for qualification. A mutation whose delivery cannot be determined is recorded as unknown and is never silently retried. Positive transport evidence that arrives after a deadline may upgrade delivery evidence to sent without reviving a job whose operation state is already unknown.

## ChatGPT Desktop

The packaged local plugin starts the compact `controlla-v2` stdio server from the plugin package. Browser access still requires the native bridge and explicit shared-tab pairing. Installation or tool discovery does not grant every tab and does not prove browser reachability.

## Qualification

Dynamic browser classes are tracked in `apps/qualification-matrix.json`. Fixture qualification is separate from remote application acceptance. Google Slides, Canva, and CapCut Web remain pending until remote mutation, reload persistence, and independent verification are demonstrated. The narrow local fixture evidence must not be described as broad application compatibility.

## Benchmarking

The existing Phase 10 benchmark remains frozen. Phase 11 adds realistic forms, extraction, multi-tab, SPA, canvas/visual, virtual-list, upload, repeated-workflow, human-interference, stale-navigation, long-workflow, and concurrent-tab tasks. Computer-use vision is a separate track from structured browser adapters.

Latency samples include failed runs at their timeout or cap rather than conditioning latency only on successes. Comparisons use paired task-cluster bootstrap analysis. A broad superiority claim is eligible only when all release gates pass on measured competitor runs.

## Public claim gate

A broad best-in-class claim is blocked unless measured evidence shows all of the following at once.

- success noninferiority lower confidence bound greater than minus two percentage points
- at least thirty percent fewer model round trips
- at least twenty-five percent lower median wall time
- zero severe wrong-target events
- zero authority violations
- zero silent stale-reference mutations

If the measured evidence supports only a subset of categories, the public claim must be narrowed to those categories. Missing benchmark evidence is not evidence of superiority.
