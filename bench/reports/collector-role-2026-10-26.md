# Targeted collector measurement, 2026-10-26

Base: `0ef14468ee2b2597b7a486d95fda758904f429de`; candidate: working tree with early role/href filtering. Chrome 155.0.8059.85 on macOS, Node 26.7.0, Playwright 1.63.0. The machine was concurrently running other checks, so these timings are diagnostic rather than a controlled release measurement.

The page contained 1,000 visible buttons and one target link. Four warmup pairs preceded 30 measured pairs in alternating order, within one browser page. The expected link and complete scoped search were checked on every measured run. The raw rows are in `collector-role-2026-10-26.json`.

| Collector metric | Base | Candidate |
| --- | ---: | ---: |
| Median | 85.4 ms | 2.15 ms |
| p95, nearest rank | 95.7 ms | 2.8 ms |
| Layout reads per run | 1,001 | 1 |
| Expected selection | 30/30 | 30/30 |
| Nodes scanned | 1,002 | 1,002 |

Reproduce from the repository root with `CONTROLLA_CHROME='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' node bench/runners/collector-role-benchmark.mjs /tmp/collector-role.json`. The runner reads its baseline from Git and its candidate from the working tree. This measures neither cold starts nor browser tasks, model turns, independent server acceptance, or real sites. Both versions still traverse every node in this fixture.
