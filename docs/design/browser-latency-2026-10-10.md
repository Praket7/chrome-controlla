# Browser execution latency: evidence and next gates

The live Bills run established a successful foreground workflow, not universal site coverage. Its slow work was (1) discovery and pairing before execution, (2) repeated native bridge and CDP exchanges, and (3) reading linked articles because the listing omitted usable publication dates. Repeated debugging and qualification consumed development time beyond the browser operation itself.

The follow-up implementation lets `browser open` pair, accept, open a foreground tab, and execute a bounded workflow in one MCP call. A targeted first-party link snapshot reports completeness for its **reachable first-party link scope** even when unrelated cross-origin frames remain unread; the global snapshot still reports those frames and `truncated:true`. On the live Bills page, the new call found eight story links, enriched missing dates, chose the latest, clicked it, and independently verified the article URL in 1,629 ms total (1,036 ms workflow). A cold one-call Wikipedia language-link task completed in 640 ms total (218 ms workflow), including a canonical redirect. These are single observed runs, not p50/p95 results.

## Current general changes

- A batch-capable shared extension now returns snapshot content and the post-snapshot frame identity in one native message. The pre-snapshot identity check remains. A real-Chrome fixture counts three host messages for the first snapshot, down from four; older extensions use the sequential path.
- When a listing supplies an exact same-origin article URL and an unambiguous `datePublished` in JSON-LD, newest-item selection uses that record without fetching the linked page. Missing or conflicting metadata retains the bounded linked-page fallback. The workflow reports `step_ms` and `elapsed_ms` for each deterministic run so site-specific bottlenecks can be measured.
- A click with a lost mouse-dispatch acknowledgement now still performs bounded read-only outcome observation. It never replays the click; a satisfied outcome can be reported as verified while retaining `dispatch_acknowledged:false`. This can avoid a model recovery turn when the page changed but its transport receipt was lost.
- Cold foreground `open` accepts optional `steps`; it waits for the requested page to commit, then executes the same bounded workflow. Existing sessions can be reused. A transient navigation error is retried only on read-only snapshots and verification. After `unknown` or `dispatched`, the next read-only `verify` can resolve the effect without replaying it.
- Workflow runtime failures retain earlier receipts and the failing step index. Compound workflows omit redundant page dumps from intermediate snapshot and action receipts. `extract` now defaults to a `text` field, and `url_starts_with` verifies canonical redirects.

| Public tool | Main cost to measure | Current rule |
|---|---|---|
| `browser` | Cold launch, discovery, pairing, acceptance, and page readiness | `open` compiles these into one call, with optional workflow steps; reuse an accepted session when available. |
| `snapshot` | Native/CDP exchanges, DOM traversal, payload bytes | Batch independent readback and identity; keep explicit omission limits. |
| `act` | Guard, target recheck, input, outcome observation | Preserve checks and never replay an uncertain mutation; inspect its postcondition. |
| `workflow` | Model turns and repeated individual tool calls | Compile bounded steps into one call, compact receipts, and report `step_ms`/`elapsed_ms`. |
| `extract` | Selector scan and returned bytes | Use bounded targeted fields; cross-site latency remains unmeasured. |
| `verify` | Independent outcome read | Keep the read; it is the evidence that prevents a fast false completion. |

## Why these boundaries remain

Chrome's native messaging serializes JSON over a persistent port; fewer host exchanges can reduce overhead, but no wall-clock gain is claimed without repeated paired measurements ([Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)). Schema.org defines `NewsArticle.url` and `datePublished`, so exact URL matching can avoid extra page reads when a site publishes them ([Schema.org NewsArticle](https://schema.org/NewsArticle)).

Playwright uses auto-waiting, locator re-resolution, and retrying assertions to handle page churn ([actionability](https://playwright.dev/docs/actionability), [locators](https://playwright.dev/docs/api/class-locator), [assertions](https://playwright.dev/docs/test-assertions)). Stagehand exposes `act`, `observe`, and `extract`, with server-side cache hits for repeated inputs in its hosted environment ([Stagehand reference](https://github.com/browserbase/stagehand/blob/main/packages/docs/v3/references/stagehand.mdx)). Browser Use combines an agent loop with DOM extraction and hosted execution ([Browser Use repository](https://github.com/browser-use/browser-use)). Controlla's current advantage in the tested path is deterministic multi-step execution against the user's foreground Chrome with one MCP call; its unmeasured gaps are cross-site success distribution, cache/replay benefit, and competitor wall time under equal conditions. BrowserGym provides a broader task suite for that comparison ([BrowserGym](https://github.com/ServiceNow/BrowserGym)).

## Qualification before broader speed claims

Run the same cold and warm scenarios across distinct site structures: plain DOM, open shadow DOM, same-origin and cross-origin frames, dynamic lists, rich editors, and pages with and without structured metadata. Record p50/p95 wall time, per-step time, host round trips, browser operations, bytes, false completions, wrong-target events, and independent postconditions. Compare exact build heads and identical Chrome/client environments. Keep failures and timeouts in the distribution. The existing benchmark runner is still a stub; there are no controlled competitor measurements or cross-site latency results for this change.

`npm run check:bench` passes for the existing baseline and offline suites. The real-browser competitor runner remains a stub, so no controlled head-to-head result supports a claim that Controlla is the fastest or best overall. The held-out freeze and lock were left untouched.
