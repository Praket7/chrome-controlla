# Taboom-derived capability benchmark

## Local deterministic evidence

- `node packages/chrome-controlla/test-agent-ux.cjs`: 21/21 passed. The ESPN/Bills goal compiler test asserts one `browser` call with the workflow embedded and no extra model-visible discovery call.
- `cargo test -p controlla-runtime --test taboom_derived_benchmark`: ten warm and ten cold synthetic screenshot-source runs measured p50/p95 at 164/165 ms and 183/183 ms, plus a bounded-region fixture. These are test-source times, not Chrome, MCP, or user-perceived latency measurements.
- Visual-stability unit tests cover document replacement, inactive renderers, quiet settling, timeout, bounds, and normalized-pixel equivalence across PNG color encodings.

## Live observations

- One fresh isolated headless Controlla rerun completed ESPN → Buffalo Bills → newest story in three MCP calls: launch, navigation, and workflow with snapshot, selection, click, and verification. It took 2,279 ms end to end, including 260,265 μs startup; the workflow itself took 217 ms. Telemetry reported 32 browser commands, 32 CDP commands (16 charged), 32 host round trips, and 16,658 observation bytes. One run is not a latency distribution.
- A separate Playwright MCP observation opened the same freshest Bills story in four calls and 1,501 ms. Playwright was pre-warmed; Controlla included process startup. The harnesses were not matched, so this does not establish comparative speed.
- A rebuilt local MCP server opened `https://example.com/` through the loaded Chrome extension in background mode in one `browser` call. The receipt reported 534 ms tool elapsed, one host round trip, and `created_by_controlla=true`. This is a single extension-path smoke, not a matched Playwright benchmark or prompt-to-action latency measurement.

## Not measured in this checkout

The live extension-path smoke provides one timing sample, not a stable latency distribution. Foreground/background window cases remain ignored qualification tests and require the user's loaded extension. Do not treat synthetic p50/p95, the single extension smoke, or the unmatched Playwright run as proof of comparative speed.

The current implementation adds repeated bounded screenshot captures for `visual_stable`; the test-source timing does not establish that these captures meet the one-host-round-trip-per-poll target. Measure that in the live paired-host harness before advertising the latency target.
