# Configured Phase 11 runner

`scripts/run-phase11-suite.mjs` configures the supported real-client commands without hand-written command exports.

It starts the disposable loopback fixture service, creates reset and independent-verifier commands, records environment identity, and runs the suite. The Controlla and Playwright convenience adapters can launch Codex for the cold, headless, single-client form and upload fixtures. Other cases explicitly return an unqualified result. The fixture state is removed afterward.

The actor disables shell execution, apps, and plugins, and uses Codex automatic approval review for MCP calls. Its fixture URL is constrained to loopback, but browser and model network access are not restricted by the harness.

Run a controlled smoke with one task and one baseline:

```sh
PHASE11_TASK_SET=/tmp/phase11-smoke.json \
PHASE11_BASELINE_IDS=playwright-mcp \
PHASE11_EXECUTION_MODES=headless \
PHASE11_REPETITIONS=1 \
PHASE11_DEADLINE_MS=180000 \
npm run bench:phase11:configured
```

Run the configured matrix with the repository defaults:

```sh
npm run bench:phase11:configured
```

The convenience adapters reject `foreground` and `background` explicitly, and headless tasks other than the bounded form and upload cases remain unqualified. A real Codex/Playwright form smoke has passed independent server readback; the Controlla actor and paired benchmark still require live qualification. A full release benchmark remains ineligible until the remaining adapters and a headed/background Chrome display harness are supplied. Explicit refusals are not browser measurements.
