# Taboom-Derived Controlla Capabilities Design

**Date:** 2026-10-18  
**Status:** Proposed for implementation planning  
**Reference audit:** Taboom `a30c48b415305c34473adb1652b4ef1ded967e1c`

## Goal

Improve Controlla's completion rate on visually unstable and browser-UI-only Chrome tasks while preserving its compact six-tool interface, background-first behavior, document-bound safety model, and low prompt-to-verified-result latency.

## Findings that shape the design

Taboom's strongest transferable ideas are a bounded visual settling primitive, one-call ordered input with an observation at the end, native desktop input, and agent-facing workflow guidance. Controlla already has the one-call workflow path, stronger semantic references, context handles, document identity, error receipts, safe routing, secure fill, and confirmation gates. Recreating Taboom's 39 low-level tools would increase model decisions and round trips, so these changes remain inside the existing `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify` contracts.

Taboom's native input runs inside its own virtual desktop. That is useful for browser chrome, extension pages, native file choosers, canvas-only controls, and challenge handoff, but it does not prove that a minimized or background macOS Chrome window can be controlled through the user's desktop. The native fallback is therefore an explicit capability with platform reporting and refusal semantics, not an unconditional promise.

## Architecture

### 1. Visual stability is a bounded read-only primitive

Add a `visual_stable` readiness policy and a matching workflow step. Both use a bounded screenshot region, a quiet interval, and a hard timeout. The runtime hashes normalized screenshot pixels at a fixed polling cadence and reports the sample count, elapsed time, region, and final visual revision. A timeout returns `stable: false`, `reason_code: visual_stability_timeout`, and a `next_action` that tells the agent to inspect the current state; it never dispatches a mutation.

The primitive must reuse Controlla's existing visual capture/evidence identity rules. A visual sample is valid only when it is tied to the same session, tab, document identity, active target lifecycle, and bounded region. It must not make a screenshot payload part of the default response unless the caller requested visual evidence.

### 2. Native fallback stays behind existing tools

Add a `desktop_fallback` capability to the existing workflow/act route rather than a seventh MCP tool. The route is selected only when the caller supplies a fresh visual reference and the semantic/page-tool routes cannot address the target. It runs through a platform adapter exposed by the native host and returns a structured receipt with:

- `status`: `completed`, `not_dispatched`, or `unknown`
- `capability`: `desktop_pointer`, `desktop_keyboard`, or `desktop_window`
- `dispatch_acknowledged` and `dispatch_may_have_occurred`
- visual reference identity and postcondition evidence
- `next_action` for refusal or uncertain delivery

The fallback is restricted to a Controlla-owned session, an explicit foreground or owned automation-window policy, a fresh visual reference, and bounded coordinates/actions. It never types a secret value directly, never bypasses a challenge handoff, never submits an order without the existing confirmation gate, and never activates a user-owned background tab implicitly.

The native-host protocol should expose capability discovery and a single bounded action envelope. Platform implementations are isolated behind a trait so macOS, Windows, and Linux can be validated independently. A missing platform backend is a clean `capability_unavailable` refusal.

### 3. Agent guidance is generated, compact, and optional

Extend the existing server instructions and plugin skill with a small route manifest generated from the runtime capability registry. It should state when to use semantic, page-tool, visual, and native fallback routes; the maximum safe retry behavior; and the exact no-safe-existing-tab fallback. Do not add a large prompt catalog or require a discovery call for normal work. The manifest is available through the existing `browser action=describe`/skill path and is also summarized in the initialize instructions.

## Data flow

1. `browser open` and `workflow` continue to select the cheapest verified route.
2. `ready_when.visual_stable` or `workflow.wait_stable` invokes the visual stability service.
3. If semantic execution is impossible, a workflow may use `desktop_fallback` with a visual reference created by `probe`.
4. The runtime validates session, target, document, visual revision, ownership, mode, and action bounds before asking the native host to dispatch.
5. The native host returns dispatch metadata; the runtime performs the requested bounded visual or semantic verification.
6. Receipts remain compact deltas with `reason_code`, `retry_class`, and `next_action` fields.

## Error and safety rules

- Never replay `unknown`; require one read-only reconciliation.
- A stale visual reference is `not_dispatched` with `retry_class: refresh_visual`.
- An unavailable platform adapter is `not_dispatched` with `retry_class: capability_unavailable`.
- A native action whose delivery cannot be acknowledged is `unknown` and must include whether it may have taken effect.
- Visual stability timeouts are read-only and do not block a later explicit action, but the receipt must make the incomplete condition visible.
- Native fallback cannot be used for secret entry, order submission, challenge bypass, or user-owned background activation.

## Performance targets

- Existing ESPN/Bills/newest-story workflow remains at three MCP calls or fewer in the benchmark harness.
- `visual_stable` adds no more than one native-host round trip per polling interval and returns a delta receipt by default.
- Normal semantic tasks perform zero native fallback calls and zero extra discovery calls.
- Capability guidance adds no new tool call to ordinary tasks.

## Verification

The implementation is accepted only when unit tests, protocol/schema tests, native bridge tests, workflow tests, and the ESPN benchmark all pass. The benchmark must report MCP call count, browser command count, CDP command count, host round trips, observation bytes, and prompt-to-verified-result latency. Platform-specific native fallback support is reported separately for each host; an unimplemented platform is not counted as passing.

## Non-goals

- Replacing Controlla's six-tool surface with Taboom's 39-tool surface.
- Copying Taboom code; Taboom is AGPL-3.0-or-later.
- Claiming universal minimized-window or cross-application control before platform tests prove it.
- Solving CAPTCHAs autonomously or weakening user confirmation for consequential actions.
- Rewriting Controlla's routing, evidence, session, or workflow architecture.
