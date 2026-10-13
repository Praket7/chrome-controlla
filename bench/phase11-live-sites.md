# Phase 11 live-site qualification

The live-site matrix is an executable, paired track separate from the deterministic fixture suite. It covers the 19 CAP-style action/perception classes in `tasks/phase11-real-sites.json`. Every row is currently `unqualified`; the matrix is not a result or a performance claim.

Run a selected candidate/baseline pair only after both adapters, a reset adapter, and an independent verifier are configured:

```sh
PHASE11_CONTROLLA_COMMAND_JSON='["/absolute/path/to/controlla","mcp"]' \
PHASE11_CONTROLLA_1_COMMAND_JSON='["/absolute/path/to/adapter","--source-ref=62e6407d0e75b83a1be8d69468476601a6e6da37"]' \
PHASE11_RESET_COMMAND_JSON='["node","/absolute/path/to/reset-live-site.mjs"]' \
PHASE11_VERIFIER_COMMAND_JSON='["node","/absolute/path/to/independent-live-verifier.mjs"]' \
PHASE11_ENVIRONMENT_JSON='{"browser_version":"Chrome <exact version>","model":"<exact model>","prompt_hash":"<sha256>","token_budget":10000,"machine":"<machine profile>","run_mode":"live-site","verifier_version":"<version/hash>"}' \
npm run bench:phase11:live-sites
```

The runner refuses non-HTTPS destinations, URL credentials, host drift, missing environment identity, missing commands, unsupported tasks, and incomplete independent verification. It writes a unique JSON result file; preserve failed rows. The live matrix defaults to the pinned Controlla 1 internal baseline, one repetition, and concurrency one. Increase modes/concurrency/repetitions explicitly after confirming each adapter's site reset and account isolation. Public-demo interactions stay on the demo page and must not submit real account actions.

Each public claim must cite the task-set ID, exact browser, model, prompt hash, token budget, machine, run mode/warm state, and verifier version. Report wall time, call/token counts, retries, visual bytes, human intervention, authority violations, stale mutations, success, and independently verified success. The separately declared `computer-use` vision track is not treated as a structured baseline.

The matrix includes real public demo/read-only destinations for website-specific semantics. The background-tab and user-interference classes require a controlled Chrome profile and browser-level verifier; the listed URLs alone cannot qualify them. Live pages may change or block automation. Preserve such failures and mark that row unsupported/unqualified instead of swapping in an easier fixture.
