# Execution-core qualification matrix

This page separates four kinds of evidence. They are not interchangeable.

| Evidence | Proves | Does not prove |
|---|---|---|
| Deterministic fixtures (`bench/fixtures`, extension Node tests, V3 Rust tests) | Exact command counts, one-transaction typing, frame routing, rebase safety, stale-session recovery | That a live application accepted the input |
| Installed-Chrome matrix (`bench/tasks/phase12-installed-apps.json`) | The operator's account saw the requested application state | Competitor parity, or behavior on a different account/tenant |
| Matched competitor run | Relative latency and call counts under the same task | Application acceptance on its own |
| Client smoke evidence | A supported client completed a real task through the MCP | Browser capability coverage |

## Current status

Run:

```sh
node bench/runners/phase12-installed-apps.mjs
```

Without an `--evidence` file, every row is reported unqualified. That is the
truthful default for a checkout that has not executed the live matrix; it is
why the repository does not claim Google Docs, Word for Web, Gmail, Canva, or
"human-intervention resistant" support from fixture tests alone.

To qualify rows, run each task on installed Chrome with your own account and
record a `phase12_evidence` file:

```json
{
  "version": 1,
  "kind": "phase12_evidence",
  "results": [
    {
      "id": "google-docs-type-1000",
      "application_accepted": true,
      "client_completed": true,
      "assertions_passed": true,
      "evidence_hash": "sha256:...",
      "controlla_version": "3.x.y",
      "recorded_at_ms": 0
    }
  ]
}
```

Then:

```sh
node bench/runners/phase12-installed-apps.mjs --evidence evidence.json --require-qualified
```

`--require-qualified` exits nonzero while any row lacks current evidence, so a
release script can refuse capability claims instead of trusting prose.

## Rows

The matrix covers: Google Docs typing and edit operations, Word for Web
typing, Gmail compose, a generic React form, CodeMirror, Monaco,
contenteditable IME, a cross-origin iframe form, a virtualized table, canvas
drag/zoom, and human interference. Each row names its correctness assertions in
`bench/tasks/phase12-installed-apps.json`.

## Release rule

A release may describe a capability only when its row is `qualified` in this
run. "Fastest", "best", "universal", and application-support claims additionally
require the matched competitor evidence enforced by
`scripts/check-best-chrome-release-gate.mjs` and
`scripts/check-v3-release-gate.mjs`.
