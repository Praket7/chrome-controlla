# Stage 2 Phase 11 runs

`phase11-realistic.json` pins Controlla 1 to `62e6407d0e75b83a1be8d69468476601a6e6da37` and pre-Stage-2 Controlla 2 to `406b9d9794d6cb59dc75d22c7e6fe7a8554b1e67`. Pinned adapters receive `source_ref` in every run request and must echo the exact revision; the runner rejects revision drift. Each complete run covers the 22 task scenarios, three cache modes, four concurrency levels, and ten paired repetitions.

Configure the JSON argv commands for each installed adapter using `PHASE11_<ADAPTER>_COMMAND_JSON`, plus `PHASE11_FIXTURE_BASE_URL`, `PHASE11_ENVIRONMENT_JSON`, `PHASE11_RESET_COMMAND_JSON`, and `PHASE11_VERIFIER_COMMAND_JSON`. Adapter commands must check out the requested source revision and independently report the listed metrics. No mock command output qualifies as browser or competitor evidence.

Run Controlla 1 against the frozen pre-Stage-2 Controlla 2 baseline:

```sh
PHASE11_CANDIDATE_ID=controlla-1 \
PHASE11_BASELINE_IDS=controlla-2-pre-stage2 \
npm run bench:phase11:suite -- --output=bench/reports/controlla-1-vs-controlla-2.json
```

Run Stage 2 Controlla against pre-Stage-2 Controlla 2 and every maintained structured adapter:

```sh
PHASE11_CANDIDATE_ID=controlla \
npm run bench:phase11:suite -- --output=bench/reports/stage2-vs-baselines.json
```

The full suite writes a new file only, so choose a fresh `--output` path for each run. `npm run check:bench` validates the runner and contracts; it does not execute Chrome or establish performance results.
