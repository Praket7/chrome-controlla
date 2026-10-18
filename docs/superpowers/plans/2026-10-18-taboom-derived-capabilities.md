# Taboom-Derived Controlla Capabilities Implementation Plan

> For agentic workers: REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Add bounded visual settling, a guarded native desktop fallback, and compact generated route guidance inspired by Taboom without changing Controlla's six-tool MCP surface.

**Architecture:** Keep semantic DOM/page-tool execution as the default. Add visual stability as a read-only service reused by ready_when and workflow execution. Add native desktop input behind the existing workflow/act route through a capability-advertised native-host action envelope, with platform adapters and explicit refusal/unknown receipts. Generate compact guidance from the existing capability registry and retain the current no-discovery default.

**Tech Stack:** Rust workspace, Tokio, serde/rmcp JSON schemas, Chrome extension/native-messaging bridge, existing visual probe/evidence store, Node.js plugin tests, Rust integration and live benchmark harness.

**Spec:** docs/superpowers/specs/2026-10-18-taboom-derived-capabilities-design.md

## Global Constraints

- Preserve the six public tools: browser, snapshot, act, workflow, extract, and verify.
- Semantic/page-tool routes remain the default; native fallback is opt-in through an existing workflow or act operation.
- Background mode must not focus Chrome or activate a user-owned tab.
- Every mutation-capable operation returns completed, not_dispatched, or unknown with reason_code, retry_class, and next_action where applicable.
- Unknown delivery is never replayed automatically; one read-only reconciliation is required.
- Secret entry, order submission, challenge bypass, and user-owned background activation are never delegated to the native fallback.
- All visual references are bound to session, tab, document identity, target lifecycle, visual revision, and bounded geometry.
- All timeouts, polling intervals, action counts, coordinates, payloads, and receipt sizes remain bounded by existing command budgets.
- No Taboom source code is copied; ideas are reimplemented under Controlla's existing license and architecture.
- Ordinary agent tasks must not require a schema, resource, or capability-discovery call before the first action.
- The ESPN/Bills/newest-story benchmark must remain at three MCP calls or fewer.

## Review Focus

- A page that keeps animating must return an honest stability timeout without dispatching a mutation; test this in the visual-stability unit and workflow tests.
- A visual reference from the previous document or tab must be refused before native dispatch; test this in the native action validation tests.
- A native host that lacks the requested platform capability must return a structured capability_unavailable refusal; test this in native bridge integration tests.
- Native delivery that may have occurred but cannot be acknowledged must return unknown and prevent replay; test this in workflow receipt tests.
- The agent must still receive a compact no-safe-existing-tab fallback and no extra discovery call; test initialize instructions, browser action=describe, and the plugin UX tests.

---

### Task 1: Freeze the baseline and add fixtures

Files:
- Create crates/controlla-runtime/tests/visual_stability_fixtures.rs
- Create crates/controlla-runtime/tests/native_fallback_fixtures.rs
- Modify crates/controlla-runtime/tests/live_espn_foreground.rs
- Modify packages/chrome-controlla/test-agent-ux.cjs
- Create docs/superpowers/evidence/2026-10-18-taboom-derived-baseline.md

Interfaces:
- Consumes existing V3ReadyWhen, workflow schemas, plugin route profiles, and benchmark telemetry.
- Produces deterministic fixtures and a baseline record that later tasks must not regress.

- [ ] Step 1: Record the current baseline. Run cargo test --workspace --all-targets and node packages/chrome-controlla/test-agent-ux.cjs. Record pass counts, ESPN call count, browser/CDP/host round trips, observation bytes, and prompt-to-verified-result latency. Do not claim live verification if the browser fixture is unavailable.
- [ ] Step 2: Add deterministic frame fixtures for changing, changing, stable, stable and for a sequence that never repeats before timeout.
- [ ] Step 3: Add native fallback JSON fixtures for valid, stale, unowned, unsupported-capability, acknowledged, and delivery-uncertain envelopes. Assert malformed and oversized envelopes fail closed.
- [ ] Step 4: Run the baseline tests. Commit with message: test: freeze Taboom-derived capability baseline.

### Task 2: Implement the bounded visual stability service

Files:
- Create crates/controlla-runtime/src/visual_stability.rs
- Modify crates/controlla-runtime/src/lib.rs
- Test crates/controlla-runtime/tests/visual_stability_fixtures.rs

Interfaces:
- Consumes existing screenshot/visual evidence identity types and the runtime command budget.
- Produces VisualStabilityConfig, VisualStabilitySample, VisualStabilityResult, and wait_for_visual_stability for MCP and workflow callers.

- [ ] Step 1: Write failing tests named visual_stability_requires_same_document_identity, visual_stability_requires_active_target_lifecycle, visual_stability_returns_stable_after_quiet_interval, visual_stability_times_out_without_claiming_stable, and visual_stability_rejects_unbounded_region_and_timeout. Assert reason codes stale_visual_reference, inactive_target, invalid_visual_stability_bounds, and visual_stability_timeout.
- [ ] Step 2: Define bounded types. VisualStabilityConfig contains optional region, quiet_ms, timeout_ms, and poll_ms. VisualStabilityResult contains stable, reason_code, elapsed_ms, samples, and visual_revision. The async service accepts a VisualFrameSource, VisualIdentity, and config and returns a bounded result or VisualStabilityError.
- [ ] Step 3: Validate quiet_ms in 150..=2000, timeout_ms in quiet_ms..=10000, poll_ms in 50..=500, and reject regions outside current screenshot geometry.
- [ ] Step 4: Implement digest polling. Hash normalized pixels for the bounded region, reset the quiet timer on change, and recheck identity and active lifecycle every sample. Never include image bytes unless separately requested.
- [ ] Step 5: Run cargo test -p controlla-runtime visual_stability -- --nocapture. Expected: all visual-stability tests pass.
- [ ] Step 6: Commit with message: feat: add bounded visual stability service.

### Task 3: Wire visual stability into readiness and workflows

Files:
- Modify crates/controlla-runtime/src/mcp/v3_full.rs at the V3ReadyWhen, V3WorkflowStep, workflow-host, step-name, execution, and schema sections.
- Modify crates/controlla-runtime/src/mcp/v2.rs only if the compatibility schema mirrors V3 readiness.
- Test crates/controlla-runtime/src/mcp/v3_full.rs and crates/controlla-runtime/tests/v2_stdio.rs.

Interfaces:
- Consumes visual_stability::wait_for_visual_stability and existing ProbeRegionArgs/visual identity validation.
- Produces V3ReadyWhen::VisualStable and V3WorkflowStep::WaitStable with compact receipts.

- [ ] Step 1: Add failing tests named ready_when_visual_stable_has_bounded_schema, workflow_wait_stable_is_read_only, workflow_wait_stable_returns_timeout_receipt_without_mutation, and visual_stability_is_rejected_on_non_open_actions_when_not_supported. Assert region, quiet_ms, timeout_ms, and deny_unknown_fields behavior.
- [ ] Step 2: Extend V3ReadyWhen with VisualStable { region, quiet_ms, timeout_ms }. Update label, readiness validation, schema descriptions, and open-readiness receipts. A timeout marks open incomplete and skips mutation steps.
- [ ] Step 3: Extend V3WorkflowStep with WaitStable { region, quiet_ms, timeout_ms }. Mark it read-only, add its step name, execute it through the shared service, and return stable, elapsed_ms, samples, visual_revision, and reason_code without image payloads by default.
- [ ] Step 4: Preserve mutation gating. WaitStable must not clear pending_readback, must not verify an unknown mutation, and cannot bypass the immediate Verify requirement after uncertain input.
- [ ] Step 5: Run cargo test -p controlla-runtime v3_full -- --nocapture, cargo test --workspace --all-targets. Expected: schema, workflow, and safety tests pass.
- [ ] Step 6: Commit with message: feat: expose bounded visual stability in open and workflow.

### Task 4: Add the compact generated route manifest

Files:
- Modify crates/controlla-runtime/src/compact_catalog.rs.
- Modify crates/controlla-runtime/src/mcp/v3_full.rs server instructions.
- Modify packages/chrome-controlla/plugin/agent-ux-proxy.cjs.
- Modify packages/chrome-controlla/plugin/plugin.json.
- Modify packages/chrome-controlla/plugin/skills/browser-control/SKILL.md.
- Test compact_catalog.rs and packages/chrome-controlla/test-agent-ux.cjs.

Interfaces:
- Consumes capability registry route names, existing six-tool descriptions, and route-selection fallback text.
- Produces a bounded route_manifest object included in browser action=describe and summarized in initialize instructions.

- [ ] Step 1: Write tests asserting exactly semantic, page_tool, visual, desktop_fallback, and headless route classes. Each class must include when_to_use, required_evidence, forbidden_cases, and next_action_on_failure. Instructions must still say to open directly without discovery.
- [ ] Step 2: Implement a pure manifest builder in compact_catalog.rs capped at 8 KiB. Derive availability from the runtime registry; do not scan tabs or call the native host merely to build guidance.
- [ ] Step 3: Include the manifest in browser action=describe and summarize only route order and the no-safe-match fallback in initialize instructions.
- [ ] Step 4: Add one short plugin paragraph covering semantic first, visual stability before visual mutation, native fallback only with fresh visual evidence and owned foreground/automation-window scope, and no native fallback for secrets/orders/challenge bypass.
- [ ] Step 5: Run cargo test -p controlla-runtime compact_catalog v3_full -- --nocapture and node packages/chrome-controlla/test-agent-ux.cjs. Commit with message: docs: expose compact generated browser route guidance.

### Task 5: Define the native desktop fallback contract

Files:
- Create crates/controlla-runtime/src/desktop_fallback.rs.
- Modify crates/controlla-runtime/src/lib.rs.
- Modify crates/controlla-runtime/src/native_host.rs.
- Modify crates/controlla-runtime/src/protocol_contract.rs.
- Test crates/controlla-runtime/tests/native_fallback_fixtures.rs.

Interfaces:
- Consumes session leases, visual identities, target ownership, command budgets, and native-message framing.
- Produces DesktopCapability, DesktopAction, DesktopActionReceipt, DesktopFallbackProvider, and one bounded native-host envelope.

- [ ] Step 1: Write tests named desktop_action_requires_owned_session_and_fresh_visual_identity, desktop_action_rejects_secret_and_consequential_action_classes, desktop_action_rejects_user_owned_background_activation, desktop_action_rejects_out_of_bounds_coordinates_and_payloads, desktop_action_unknown_receipt_for_uncertain_delivery, and desktop_capability_unavailable_is_not_dispatched.
- [ ] Step 2: Define a tagged envelope with version, request_id, lease_id, capability, mode, visual_identity, and one action. Initially support bounded click, move, drag, scroll, key, and type actions only.
- [ ] Step 3: Reject secrets, submit/order markers, challenge-solving claims, arbitrary shell commands, and unbounded text. Keep type separate from secure_fill.
- [ ] Step 4: Define completed, not_dispatched, and unknown receipts with dispatch_acknowledged, dispatch_may_have_occurred, reason_code, retry_class, and next_action. Never retry inside the provider.
- [ ] Step 5: Register the provider command in the browser protocol allowlist and classify calls as desktop_fallback in telemetry.
- [ ] Step 6: Run cargo test -p controlla-runtime desktop_fallback native_fallback -- --nocapture. Commit with message: feat: define guarded native desktop fallback contract.

### Task 6: Add platform adapters behind the contract

Files:
- Create crates/controlla-runtime/src/desktop_platform.rs.
- Create crates/controlla-runtime/src/desktop_platform/macos.rs.
- Create crates/controlla-runtime/src/desktop_platform/windows.rs.
- Create crates/controlla-runtime/src/desktop_platform/linux.rs.
- Modify crates/controlla-runtime/src/main.rs and Cargo.toml.
- Test crates/controlla-runtime/tests/native_bridge_process.rs.

Interfaces:
- Consumes DesktopAction from Task 5.
- Produces PlatformDesktopProvider implementing DesktopFallbackProvider and a capability probe that never claims unsupported operations.

- [ ] Step 1: Run a throwaway platform API probe before adding dependencies. Verify macOS Quartz/CoreGraphics event injection and window enumeration, Windows SendInput/UIA boundaries, and Linux X11/Wayland availability. Record results; do not shell through arbitrary user commands.
- [ ] Step 2: Add per-platform adapter tests for capability reporting, coordinate bounds, unsupported action refusal, and receipt conversion. Keep actual OS injection behind feature-gated desktop tests.
- [ ] Step 3: Implement the macOS adapter first. Require Accessibility/Input Monitoring permission, focused-window identity, and the requested owned window policy. Refuse missing permission or background activation requests.
- [ ] Step 4: Add Windows and Linux modules with honest capability reporting. Return capability_unavailable until platform tests pass.
- [ ] Step 5: Run cargo test -p controlla-runtime native_bridge -- --nocapture and cargo test --workspace --all-targets. Commit with message: feat: add platform-gated desktop fallback adapters.

### Task 7: Route native fallback through act and workflow

Files:
- Modify crates/controlla-runtime/src/mcp/v3_full.rs at workflow schema, workflow host, execution, and action validation sections.
- Modify crates/controlla-runtime/src/mcp/v2.rs compatibility translation if required.
- Modify crates/controlla-browser/src/providers.rs and cancellation.rs.
- Test crates/controlla-runtime/src/mcp/v3_full.rs and crates/controlla-runtime/tests/native_bridge_roundtrip.rs.

Interfaces:
- Consumes DesktopFallbackProvider, capability manifest, visual stability, and existing workflow verification.
- Produces a DesktopFallback workflow step and an act action using the same guarded route without adding a public MCP tool.

- [ ] Step 1: Add failing tests named desktop_fallback_requires_visual_reference, desktop_fallback_runs_only_in_allowed_mode, desktop_fallback_requires_immediate_visual_or_semantic_verification, desktop_fallback_refusal_does_not_consume_the_workflow_lease, and desktop_fallback_unknown_blocks_later_mutations_until_verify.
- [ ] Step 2: Expose only bounded fields: reference, action, coordinates, optional key/text, mode, and explicit outcome predicate. Reject password-like references and confirmation-sensitive actions before dispatch.
- [ ] Step 3: Revalidate session ownership, tab/document identity, visual revision, target lifecycle, window policy, and current geometry. Return not_dispatched with retry_class refresh_visual on mismatch.
- [ ] Step 4: Dispatch through the provider and require immediate verify or visual postcondition. Preserve the unknown fence and compact delta receipt.
- [ ] Step 5: Run cargo test -p controlla-runtime v3_full native_bridge_roundtrip -- --nocapture and cargo test --workspace --all-targets. Commit with message: feat: route guarded desktop fallback through existing tools.

### Task 8: Add end-to-end benchmark and failure coverage

Files:
- Create crates/controlla-runtime/tests/taboom_derived_benchmark.rs.
- Modify live ESPN tests and packages/chrome-controlla/test-agent-ux.cjs.
- Create docs/benchmarks/taboom-derived-capabilities.md.

Interfaces:
- Consumes all completed routes, telemetry, and the existing ESPN/Bills fixture.
- Produces repeatable call-count, latency, and fallback verification evidence.

- [ ] Step 1: Measure at least 10 warm and 10 cold ESPN/Bills/newest-story runs. Assert at most three MCP calls, no unrelated-tab scan, and telemetry for route selection, browser commands, CDP commands, host round trips, observation bytes, and verification latency.
- [ ] Step 2: Exercise animation settling, timeout, document replacement during polling, and a stable region with unrelated changes outside the region.
- [ ] Step 3: Exercise browser chrome target, stale visual reference, unsupported platform, foreground owned window, user-owned background window, and uncertain delivery. Assert secret/order/challenge bypass is never admitted.
- [ ] Step 4: Run cargo fmt --all -- --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace --all-targets, and node packages/chrome-controlla/test-agent-ux.cjs. Resolve all new warnings and format drift introduced by this branch.
- [ ] Step 5: Run live tests only when the configured extension/native host is available. Record executed, skipped, or blocked; do not report unit/integration results as live verification.
- [ ] Step 6: Commit with message: test: benchmark Taboom-derived browser capabilities.

### Task 9: Document rollout, permissions, and release boundaries

Files:
- Modify browser-control SKILL.md, advanced.md, recovery.md, README.md, and docs/ROADMAP.md.
- Create docs/desktop-fallback.md.

Interfaces:
- Consumes final schemas, route manifest, platform capability results, and benchmark evidence.
- Produces compact agent instructions that are honest about platform limits.

- [ ] Step 1: Document semantic-first route selection, visual stability before visual mutation, native fallback only with fresh evidence and owned scope, and the exact no-safe-existing-tab fallback.
- [ ] Step 2: Document macOS Accessibility/Input Monitoring and equivalent platform gates. Permission absence returns capability_unavailable and does not retry.
- [ ] Step 3: Document that native fallback cannot type secrets, submit orders, bypass challenges, or activate user-owned background tabs.
- [ ] Step 4: Separate implemented local behavior, platform-tested behavior, live browser verification, hosted CI, and future platform work in the roadmap.
- [ ] Step 5: Run node packages/chrome-controlla/test-agent-ux.cjs and cargo test --workspace --all-targets. Commit with message: docs: document visual stability and desktop fallback boundaries.

### Task 10: Final branch review and handoff

Files:
- Review all files changed by Tasks 1–9.
- Update docs/superpowers/evidence/2026-10-18-taboom-derived-final.md.

- [ ] Step 1: Confirm no seventh MCP tool, unrelated refactor, copied Taboom code, unbounded native command, or implicit user-window activation was introduced.
- [ ] Step 2: Check every new refusal and unknown receipt for reason_code, retry_class, next_action, and correct dispatch uncertainty. Check that no-safe-existing-tab fallback remains compact and actionable.
- [ ] Step 3: Run git diff --check, cargo fmt --all -- --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace --all-targets, and node packages/chrome-controlla/test-agent-ux.cjs.
- [ ] Step 4: Record exact commit, tests, benchmark runs, platform adapters, live browser status, and skipped checks. Do not call a platform or live workflow verified without observed evidence.
- [ ] Step 5: Commit with message: chore: record Taboom-derived capability verification.

## Execution order and gates

Tasks 1–4 are the low-risk speed and guidance improvements and can ship independently. Task 5 must pass before native platform implementation begins. Task 6 is platform-gated and can ship with only adapters that have real permission and desktop tests. Task 7 depends on Tasks 2, 5, and 6. Tasks 8–10 are required before calling the work complete.

The first release milestone is Tasks 1–4 plus the benchmark. The native fallback milestone is Tasks 5–7 plus at least one verified platform adapter. Unsupported platforms remain explicit refusals rather than unfinished promises.

