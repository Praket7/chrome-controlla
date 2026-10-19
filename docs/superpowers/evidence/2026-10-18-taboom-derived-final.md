# Taboom-Derived Controlla Implementation Evidence

**Branch:** `codex/comprehensive-chrome-audit`  
**Base:** `2998b7bd4683c3f15763ab086f7f2faae9f125bc`  
**Status:** local implementation and verification pass; native desktop fallback is disabled pending a stable signed helper identity; the 20-run live performance qualification remains outstanding

## Implemented

- Bounded, read-only visual stability sampling with normalized PNG pixel hashing, document/lifecycle checks, region support, and one deadline covering viewport lookup, the initial screenshot, and all later samples.
- `visual_stable` open readiness and `wait_stable` workflow step.
- Compact five-route capability manifest, server instructions, and a no-safe-tab fallback that reports bounded candidates and asks for an exact selection when an existing tab is required.
- Six-tool MCP surface remains within a 68 KiB schema budget (66,504 bytes in the focused test); visual readiness and workflow fallback are present in the published schemas.
- Guarded native action envelope, stale-evidence and ownership validation, bounded receipts, and fail-closed capability reporting.
- A macOS native-input prototype contains ScriptingBridge, PID/tab identity checks, bounded actions, and fail-closed receipts. It is not advertised or dispatched because its Swift helper lacks a stable signed app identity. The direct `act` route handles browser actions through the extension; helper timeout, crash, or ambiguous completion in any future native implementation must return `unknown` and never be replayed. Browser tab groups/windows remain extension-managed; OS-window control is unavailable.
- The owned-session validator now proves exact live session identity, provider-created tab ownership, non-headless mode, and unchanged focus/activation epochs instead of passing `false` or assuming ownership.
- Synthetic visual fixture benchmark covers ten warm and ten cold runs. Measured p50/p95 were 164/165 ms warm and 183/183 ms cold; these are service-fixture timings, not Chrome/MCP latency.
- Live headless ESPN/Bills/newest-story completed in three MCP calls (launch, navigation, and a workflow containing snapshot, selection, click, and verification). The fresh rerun measured 2,279 ms total, 217 ms workflow time, and 260,265 μs startup. Workflow telemetry reported 32 browser commands, 32 CDP commands (16 charged), 32 host round trips, and 16,658 observation bytes. This is one run, not a distribution.
- A separately run Playwright MCP path found and opened the same newest Bills story in four calls and 1,501 ms. It was already warm while Controlla included process startup, so the measurements are not matched and support no speed-superiority claim.

## Verification

- `cargo test --workspace --all-targets`: passed; live shared-browser and ESPN network qualification cases remain ignored behind their existing gates.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `node packages/chrome-controlla/test-agent-ux.cjs`: 21 passed.
- The ESPN/Bills/newest-story plugin fixture compiles to one browser call with an inline verified workflow. This does not measure end-to-end model latency.

## Explicit limits

- The macOS adapter's Swift helper has no stable signed app identity, so `describe` reports `helper_identity_supported: false` and native input refuses. Do not authorize the ephemeral helper; native fallback remains disabled until Controlla packages a stable signed helper. Normal background page clicks, drags, typing, and navigation use the Chrome extension and do not require Accessibility. Native coordinate clicks, drags, all text entry/navigation, OS-window control, and Windows/Linux native input remain unsupported.
- The live ESPN smoke passed in headless mode, and the current rebuilt MCP opened an exact test URL through the extension in background mode in one call (534 ms tool elapsed; 1 host round trip). Foreground/background native-input tests were not run. The Playwright observation was not a matched benchmark.
- The synthetic benchmark does not measure Chrome screenshot transport, MCP round trips, caller prompt latency, or remote CI.
- Historical fresh 6 Sol review on an earlier revision: NO-GO for platform adapter, direct `act` route, and comparative performance qualification. The browser `act` route uses the extension; native fallback remains disabled until stable helper packaging. The comparative performance target remains unqualified. The hidden-tab provenance finding was resolved earlier: the validator accepts `hidden_tab_renderer_clock_sample`, with a regression test covering it.

The semantic, page-tool, visual, and configured headless routes remain available. Native fallback is currently unavailable on every platform; Chrome page and tab actions use the extension and do not require Accessibility. A future native adapter must package a stable signed helper and pass live permission and dispatch qualification before it can be advertised.
