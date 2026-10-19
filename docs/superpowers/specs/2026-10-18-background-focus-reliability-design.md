# Background Execution, Focus Semantics, and Agent Reliability

**Date:** 2026-10-18  
**Status:** Approved for implementation

## Problem

Controlla still makes background work feel focus-dependent in two ways. The runtime correctly downgraded foreground presentation failure to a warning, but stale watcher revisions can still stop an otherwise valid semantic action before dispatch. The agent then receives a refusal and often repeats discovery or the mutation. The current instructions contain the right capabilities, but the rules are spread across long sections and leave too much room for a smaller model to confuse presentation, execution, ownership, and uncertain delivery.

## Goals

1. Make semantic page work independent of OS focus, Chrome minimization, and the selected application.
2. Keep explicit, capability-specific gates for operations that truly require a visible active surface or native input.
3. Recover one stale semantic reference inside the server without asking the model to rediscover the same tab.
4. Make every refusal and validation failure actionable structured tool output.
5. Give the model a compact deterministic route with an unambiguous no-safe-tab fallback.
6. Prove the contract with focused Rust, extension, and agent-UX tests.

## Non-goals

- Replacing the extension, debugger transport, or six-tool MCP surface.
- Silently activating a user-owned tab or stealing OS focus.
- Retrying an operation whose dispatch outcome is unknown.
- Bypassing CAPTCHA, Cloudflare, MFA, password, or order confirmation safeguards.
- Claiming live Chrome qualification from local unit tests alone.

## Design

### 1. Presentation and execution are separate

`mode=background` is the default for semantic DOM work. Snapshot, find, extract, semantic click, fill, select, type, press, and scroll may run against an inactive, unfocused, minimized, or hidden tab when the retained document and target checks pass. These operations must not call an activation path.

`mode=foreground` expresses a presentation preference. The open route attempts the normal extension presentation transition when allowed, but a failure to focus Chrome produces `presentation_warning="chrome_window_not_focused"` and `presentation_verified=false`; it does not suppress workflow steps or mark the semantic operation incomplete.

Capability-specific gates remain explicit:

| Capability | Background allowed | Gate when unavailable |
| --- | --- | --- |
| Semantic DOM observation and mutation | Yes | `not_dispatched` only for stale, detached, masked, or unsafe targets |
| Navigation by observed exact href | Yes | Structured target or navigation refusal |
| Trusted pointer, hover, touch, visual drag | Only in a Controlla-owned automation window | `activation_forbidden` with an owned-window recovery |
| Secure password entry and native dialogs | No | Foreground interactive session required |
| CAPTCHA, Cloudflare, MFA | No automatic bypass | User hand-off and resume after observed completion |

### 2. Bounded stale-reference recovery

The first semantic mutation attempts the retained reference. If the receipt is `status=not_dispatched` with `reason_code=stale_watcher_revision`, the server performs exactly one internal recovery:

1. Refresh the same session and Chrome tab.
2. Require the same document identity, or a documented same-document navigation identity when the operation allows it.
3. Re-snapshot only the bounded semantic scope.
4. Reacquire using the stored role, accessible name, observed href, and ancestor constraints.
5. Require one unique candidate with the same target safety properties.
6. Retry the mutation once with the fresh reference.

The receipt includes `reacquisition.status` (`rebound`, `not_rebound`, or `not_attempted`), the original reason, the new semantic revision, and `next_action`. Any document change, duplicate match, ownership loss, masked target, or incomplete scope stops without dispatch. `unknown` remains terminal until a read-only reconciliation proves the effect absent and the user or model chooses a new action.

The existing `changed_target` recovery remains supported and uses the same one-attempt budget. A single operation can never perform more than one internal re-acquisition.

### 3. Compact route selection

The server first evaluates the exact bound context, then returns at most eight rejected candidates with compact reason codes. It never borrows an unrelated user tab to open a new URL. For a new-target request with no safe match, it cold-opens the exact URL in a new Controlla-owned tab. For a request that explicitly requires an existing tab, it returns `status=not_dispatched`, `reason_code=no_safe_tab_match`, the bounded candidates, and `next_action="select_exact_tab_or_open_new_target"`.

The route receipt must be sufficient for the model to choose its next step without a second broad discovery call. URLs are origin-plus-truncated-path only, and query strings and fragments are redacted from candidate evidence.

### 4. Structured tool-result errors

The V3 call boundary converts invalid parameters and stale-reference failures into normal tool results. Every error result contains:

```json
{
  "status": "not_dispatched",
  "reason_code": "stale_watcher_revision",
  "reason": "The observed tab changed after discovery.",
  "dispatch_may_have_occurred": false,
  "next_action": "refresh_and_reacquire_once"
}
```

`unknown` uses `dispatch_may_have_occurred=true` and `next_action="verify_read_only_before_retry"`. The result must not name nonexistent tools or require the model to infer protocol-level validation details.

### 5. Model-facing instructions

The runtime `instructions` and `browser-control` skill share one short contract:

- Use the six normal tools immediately; do not inspect schemas or repository files for routine tasks.
- Prefer direct `browser open` with bounded `steps` for new URLs and one workflow for known multi-step work.
- Use one targeted snapshot only when a fresh semantic reference is necessary.
- Background is the default for semantic work; foreground is presentation and has warning-only focus failure.
- Follow `not_dispatched.next_action`; reconcile `unknown` once before any new mutation.
- Use workflow `save_as` values as bare names or `$name`; direct semantic references use `@cN`/`@f:`.
- If no safe existing tab matches, use the exact new-target fallback or ask for an exact tab selection according to the task.
- Confirm exact order, amount, account, and final action before consequential submission; hand CAPTCHA, Cloudflare, and MFA to the user.

The instructions include three canonical recipes: direct open plus steps, exact existing-tab continuation, and sensitive/consequential work. Repeated capability prose moves to reference documentation.

### 6. Observability

Add counters and debug fields for watcher revision invalidations, internal re-acquisition attempts, rebound success, refusal reasons, and presentation warnings. Keep secrets, OAuth tokens, and full query strings out of receipts and logs.

## Acceptance criteria

- A background semantic workflow succeeds while Chrome is minimized or another app is active, without changing OS focus.
- A foreground open with an unfocused Chrome window dispatches its steps and returns a presentation warning.
- A stale watcher revision is rebound once when the same unique target remains valid.
- A changed document, duplicate target, or unsafe ownership state refuses without a blind retry.
- Invalid arguments and stale refs are visible as tool results with `reason_code` and `next_action`.
- A no-safe-tab route is compact and includes the exact fallback.
- The agent-UX tests prove the direct ESPN/Bills/newest-story route stays within the intended call budget.
- Focused Rust and Node suites pass; live extension/MCP qualification is reported separately after reload.

## Alternatives considered

1. **Watcher redesign first:** per-tab generations and event coalescing could reduce invalidations further, but it is a larger transport change and is deferred unless the new event-storm tests show residual churn.
2. **Instruction-only change:** improves model behavior but leaves stale dispatch failures in the server, so it is insufficient.
3. **Universal foreground activation:** would make presentation look reliable while stealing focus and still would not solve stale references, so it is rejected.

## Risks and mitigations

- A page can change between re-snapshot and retry. The same-document and unique-target checks plus the one-attempt limit keep the retry bounded.
- Some semantic controls genuinely require trusted native input. The capability matrix exposes that boundary instead of pretending background support.
- Compact receipts can omit useful detail. Keep full diagnostics in local logs while returning bounded model-visible evidence.
