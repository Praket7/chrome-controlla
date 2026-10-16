# Configured Phase 11 runner

`scripts/run-phase11-suite.mjs` now configures the complete real-client command surface instead of requiring hand-written shell exports.

It starts the disposable loopback fixture service, creates reset and independent-verifier commands, binds real Codex MCP command adapters for Controlla, the frozen Controlla revisions, Playwright MCP, Chrome DevTools MCP, agent-browser, Stagehand, and Browser Use, records the required environment identity, runs the suite, and removes the fixture state afterward.

Run a controlled smoke with one task and one baseline:

```sh
PHASE11_TASK_SET=/tmp/phase11-smoke.json \
PHASE11_BASELINE_IDS=playwright-mcp \
PHASE11_EXECUTION_MODES=headless \
PHASE11_REPETITIONS=1 \
PHASE11_DEADLINE_MS=15000 \
npm run bench:phase11:configured
```

Run the configured matrix with the repository defaults:

```sh
npm run bench:phase11:configured
```

The adapters currently reject `foreground` and `background` execution with an explicit measured unsupported result; they do not relabel headless runs as those modes. A full release benchmark therefore remains ineligible until a headed/background Chrome display harness is supplied. No mock adapter is used by this runner.
