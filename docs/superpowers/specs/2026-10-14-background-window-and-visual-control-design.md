# Background window, visual control, and reliability design

**Status:** Approved and implemented at code/fixture level on 2026-10-14; live Chrome and Canva qualification pending reload.

## Goal

Make Controlla useful for work that should happen away from the Chrome window the user is using, while preserving exact target selection, reliable visual review, and verifiable actions. The user must be able to direct Controlla to an already-open tab or explicitly request an agent-owned background window. Controlla must never silently replace an existing-tab request with a new tab or window.

The design covers the supplied live Canva run and the code-audit findings that affect task accuracy, input reliability, latency, call count, packaging, and benchmark trustworthiness. It keeps the six-tool V3 MCP surface and does not add native desktop automation or a new persistent browser profile.

## Evidence and current failures

### Live Canva run reported by the user

- A semantic snapshot found seven slides and read the cover, but returned zero screenshot bytes.
- A visual probe for the canvas failed with `Command is outside the shared observe/input allowlist`.
- Selecting slide 2 returned `unknown`; a subsequent read showed slide 1 of 7. A keyboard action also left the deck on slide 1. Slides 2–7 therefore were not reviewed.
- A background click briefly activated the target tab before restoring the prior tab. The final tab state was inactive, but that does not satisfy a no-momentary-activation guarantee.
- In another task, the user asked Controlla to use an existing open tab and it created a new one instead. Target selection and tab creation are not sufficiently distinct in the agent-facing contract.

### Source-level evidence in the current main tree

- `extensions/chrome-controlla/background.js` defines `SHARED_COMMAND_METHODS` without `Page.captureScreenshot`, which is consistent with the shared visual-probe allowlist rejection. This is a likely direct cause; the live receipt should be retained to confirm the exact command that was rejected.
- The shared pointer path calls `chrome.tabs.update(tabId, {active:true})` for an inactive tab before sending mouse input, then tries to restore the previous tab. This explains the observed temporary activation.
- `browser action=open` is the create/navigate path. The agent instructions need a stronger distinction between operating on an existing target and creating a destination tab.
- The prior independent audit also identified OOPIF coordinate translation, action deadline mismatch, serial text-entry overhead, release archive omissions, oversized AX messages, scheduler feedback contamination, redundant CDP reads, stale V2 instructions, and benchmark measurement errors.

## User-facing behavior

Controlla keeps one stable set of six MCP tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. Target/window policy is expressed through these existing tools and their typed arguments, not a seventh top-level tool.

### Target policies

Every browser task resolves one explicit target policy before it can create, activate, navigate, or mutate a tab:

1. **Existing tab:** Bind to the exact user-named or uniquely identified existing tab. Do not create, replace, duplicate, or navigate another tab as a fallback. If identity is ambiguous, stop with a compact ambiguity result.
2. **Existing automation window:** Use an existing window that the user explicitly designated for Controlla. Controlla may activate tabs inside that designated window when required, but must not focus the Chrome window or change active tabs in other windows.
3. **Create background automation window:** Create a separate window only when the user or task contract explicitly requests one. Use `chrome.windows.create({focused:false, ...})`, bind all task commands to the returned owned window, and report the actual window ID and placement. Do not focus it. Chrome chooses placement using its normal window policy; Controlla does not claim monitor or virtual-desktop selection.
4. **Headless:** Reuse the existing private headless path only for task kinds it already supports. It is a separate mode and must never be silently substituted for an authenticated or user-tab task.

Requests such as “use the open Canva tab” select **Existing tab**. Requests such as “open a separate background window and work there” select **Create background automation window**. Missing or ambiguous target policy must not cause an implicit new-tab fallback.

### No-disturbance contract

- Background work must not focus Chrome or activate a tab in a user-owned window.
- An exact existing user tab remains the exact target. If its requested action cannot be performed without activating it, return `not_dispatched` with a precise capability reason; do not switch tabs or create a replacement.
- Within an explicitly Controlla-owned automation window, Controlla may keep the target tab active inside that window while the Chrome window remains unfocused. Every activation and focus event is attributed to the owned window and included in the receipt.
- The capability is not described as “zero activation” if any user-owned tab activated. Restoration afterward does not erase the activation event.
- If Chrome cannot deliver a required input or capture for an inactive target, fail clearly. Synthetic DOM events are not a fallback for trusted pointer input.

Chrome documents `windows.create({focused:false})` for an inactive window and `tabs.create({active:false})` for an inactive tab. These APIs establish window/tab state controls, not a guarantee that every inactive page accepts trusted input or renders screenshots. Each supported operation therefore requires an observed postcondition and live qualification. References: [Chrome windows API](https://developer.chrome.com/docs/extensions/reference/api/windows), [Chrome tabs API](https://developer.chrome.com/docs/extensions/reference/api/tabs), [Chrome debugger API](https://developer.chrome.com/docs/extensions/reference/api/debugger).

### Visual review and slide navigation

- Shared visual probes must have an explicitly authorized, bounded screenshot route. Add only the required screenshot/geometry methods to the shared observe allowlist, with the existing evidence-byte, crop, target-identity, and document-revision guards.
- A successful visual probe returns a nonempty PNG image content block and accurate byte metrics. A JSON receipt with no image bytes is not a successful screenshot.
- Keep targeted crops as the default. Full-page or full-window captures remain opt-in and bounded.
- Canva slide navigation uses a verified slide identity/index, not an unverified keyboard guess. The slide adapter identifies the page list/thumbnail and the active slide, performs one guarded selection, then verifies the active slide changed to the exact requested index or stable slide ID.
- A slide change with `unknown` is followed only by a fresh read-only active-slide check. It is never blindly clicked or keyed again.
- A deck-review workflow captures and verifies each requested slide separately. It reports the set actually reviewed and stops on an unavailable screenshot or unresolved slide identity; it never claims review of unseen slides.

## System design

### Target resolver and ownership registry

Add a typed target policy to the existing browser/task contract and centralize resolution before side effects. Resolution records:

- policy (`existing_tab`, `existing_automation_window`, `create_background_window`, or existing `headless` mode);
- exact Chrome window and tab IDs;
- owner (`user` or `controlla`), session ID, and operation/idempotency identity;
- whether focus or tab activation is permitted for this target;
- creation/placement result and cleanup disposition.

For existing targets, resolution uses one bounded inventory or an already-bound reference. It does not call a create path when the target is missing. For a new owned window, create it unfocused, attach the target page inside that window, and bind the session to its returned IDs. Ownership is enforced on every subsequent command. Release may close only an unchanged Controlla-owned window/tab created by that session; it must preserve all user-owned tabs and windows.

The browser contract distinguishes **select/reuse target** from **open/create destination**. Existing-tab tasks must not require callers to invoke `open`. The exact schema/action names will be fixed in the implementation plan after this spec is reviewed, preserving the six top-level tools and stable schema-budget constraints.

### Shared visual route

The shared extension transport gains a narrow, typed visual operation that is covered by its method allowlist. It accepts only a fresh target reference or bounded region, validates session/window/document ownership, applies crop and byte limits before returning data, and records exact screenshot bytes. The runtime packages the PNG as image content in the same snapshot/probe result. Unsupported or policy-blocked screenshot capture returns a typed, non-success result; it does not silently omit the image.

The implementation must check whether `Page.captureScreenshot` alone is sufficient for the Canva canvas under shared mode. If layout metrics or element geometry are needed, add only those methods and constrain parameters. Chrome's debugger API can be restricted by enterprise screenshot/DLP policy, so a policy refusal must remain explicit. References: [CDP Page domain](https://chromedevtools.github.io/devtools-protocol/tot/Page/), [Chrome debugger API restrictions](https://developer.chrome.com/docs/extensions/reference/api/debugger).

### Verified slide adapter

The Canva path should first use observed accessible slide controls and active-slide state. It should use targeted visual evidence when the deck canvas or thumbnail labels are not represented semantically. It must not assume page 1 from a keypress acknowledgement, nor accept an `unknown` action as progress. Any app-specific adapter stays fail-closed and reports unsupported structure rather than inventing a click point.

## Reliability and performance scope from the audit

The implementation plan will order these fixes by user-visible severity and dependencies:

### Correctness and safety blockers

- Translate OOPIF pointer coordinates into the main-frame viewport coordinate space and test a child frame with a nonzero offset. Keep the exact guarded target and hit-test checks. Reference: [CDP Input domain](https://chromedevtools.github.io/devtools-protocol/tot/Input/).
- Align native-host/client deadlines with extension-side operation deadlines, or add cancellation so input cannot continue after the caller receives `unknown`. Unknown delivery is inspected read-only and never replayed.
- Ensure upload helper deadlines fit within task/workflow wall deadlines and preserve an upload-specific failure result.
- Bound/compact accessibility payloads before native messaging; an oversized AX response must return an incomplete result instead of terminating the long-lived native host.
- Expire/prune consumed pairing tokens while retaining replay rejection for the token's valid lifetime.
- Clear frame-session and file-chooser waiter state on tab removal/debugger detach while retaining the existing navigation reattach exception.
- Include all extension runtime pages/assets in release archives and validate the extracted artifact.
- Align the documented Chrome minimum with flat-session support, or implement and qualify a fallback.

### Latency and command-count improvements

- Batch ordinary text insertion into bounded chunks with guarded readback where the control is qualified; preserve per-key event behavior where application semantics require it. Compare actual CDP command counts and correctness before enabling the fast path.
- Exclude same-target lock wait from global browser-congestion feedback; report target-lock wait, global-gate wait, and browser service time separately.
- Remove workflow preflight and parallel-research CDP reads that duplicate an identity already obtained by the workflow, without weakening frame/document identity checks.
- Add per-endpoint singleflight for cold connection/bootstrap so concurrent first requests do not duplicate setup work.
- Correct the historical percentile label/calculation so the execution router's p90/p95 and report semantics agree; test exact sample-rank behavior.
- Keep per-endpoint cold-start setup singleflight so simultaneous initial callers do not attach and bootstrap duplicate browser connections.

### Measurement, instructions, and qualification

- Instrument the client adapter's actual prompt-submit boundary where supported, then actual MCP receive, first browser/CDP dispatch, last dispatch, verified outcome, and result-delivery boundaries. A process-spawn timestamp must be labeled process-start timing, not user prompt-send timing; never label MCP-call arrival as first browser operation.
- Distinguish configured/live client adapters from local smoke adapters. Do not compare adapters as equivalent prompt-to-outcome paths unless their actual submission boundary is observable.
- On adapter timeout/crash, keep elapsed time but report unobserved call, command, retry, and verification counts as `null`/unknown rather than zero.
- Resample benchmark task IDs as paired clusters, retaining each task's matched modes/concurrency/repetitions. Validate percentile conventions with repeated task IDs and exact-multiple sample sizes.
- Remove V2-only session/schema guidance from current V3 instructions or label it clearly as legacy. Ordinary task instructions must choose the existing target path without schema/describe/discovery overhead.
- Keep all performance claims separated into local fixture tests, packaged artifact tests, real Chrome live qualification, live app qualification, and matched client/competitor measurements.
- Reproduce and fix exact-head hosted CI failures before describing a release as green; keep an empty/unavailable status result distinct from a passing run.

## Failure and receipt contract

- `verified`: the requested target and postcondition were observed independently; receipts include exact selected target policy and focus/activation telemetry.
- `not_dispatched`: no input/navigation/mutation occurred; includes a stable reason such as `existing_target_required`, `ambiguous_existing_target`, `activation_forbidden`, `visual_capture_unavailable`, or `slide_identity_unresolved`.
- `unknown`: an operation may have been dispatched but its outcome cannot be proven. The next step is a fresh read-only observation, never an automatic retry.
- Creation results include `created:false` or `created:true`, owned IDs, and whether the preexisting user window/tab selection changed. An existing-target task that creates a tab/window fails its acceptance test.
- Receipts distinguish `window_focused`, target `tab_active`, user-window activation changes, owned-window activation changes, temporary activation, screenshot bytes, and actual command counts. Restoration is recorded but never presented as proof that no activation occurred.

## Acceptance criteria

1. An “existing open tab” request targets the exact observed existing tab; if selection is ambiguous it stops. It creates zero tabs/windows and never silently substitutes a different tab.
2. A requested background existing-tab operation produces no activation event in a user-owned window and no Chrome window-focus change. If the needed operation cannot meet that invariant, it returns `not_dispatched` without trying a new target.
3. An explicitly requested background automation window is created with `focused:false`, is bound to the operation's ownership record, and does not change the focused Chrome window or active tab in any user-owned window. Supported position hints are reflected accurately in the receipt.
4. After release, only unchanged Controlla-owned resources may be cleaned up; every user tab/window remains open and unchanged.
5. A bounded shared visual probe returns a valid, nonempty PNG image with exact bytes and fresh document identity. Canva's live canvas probe no longer fails at the shared allowlist.
6. A seven-slide fixture and qualified live Canva deck can select slides 1–7 individually, verify each active slide ID/index, and capture each slide. No slide counts as reviewed without both verified selection and screenshot evidence.
7. Injected stale OOPIF/frame geometry, navigation, changed focus, inaccessible screenshot policy, rejected slide selection, oversized AX trees, timeouts, and unknown delivery all fail closed with honest receipts and no blind mutation retry.
8. Extension release archives contain every referenced runtime asset and pass an extract-and-load manifest check.
9. Ordinary text entry reduces command count materially on qualified controls without changing values/events expected by the application. Unqualified controls retain the existing safe path.
10. Benchmarks have truthful stage boundaries, unknown counts remain unknown, task-cluster resampling preserves paired observations, and percentile names match their implemented estimator.
11. Supported clients expose real prompt-submit timing or reports explicitly label a process-start proxy. Client setup and live qualification are separately measured.
12. CI passes on the supported Chrome and operating-system matrix. Live browser/Canva and end-to-end client evidence are separately reported; local tests alone do not qualify them.

## Out of scope

- Native mouse/keyboard automation of the desktop or control of other applications.
- Adding a persistent browser profile or silently transferring authentication state to another browser.
- Silently creating tabs/windows, changing the user's Chrome window focus, or claiming background behavior because a tab was restored after activation.
- Synthetic DOM events as a substitute for trusted input when a site rejects them.
- Universal claims that Controlla is the fastest or most capable browser agent without matched external evidence.
- Treating an action acknowledgement, slide index guess, or visible toast as independent verification.
