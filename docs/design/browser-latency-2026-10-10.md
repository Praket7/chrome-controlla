# Browser execution latency: evidence and next gates

The live Bills run established one successful foreground workflow, not universal site coverage or completion of the 20-task plan. Its slow work was (1) discovery and pairing before execution, (2) repeated native bridge and CDP exchanges around snapshot and action guards, and (3) reading linked articles because the listing omitted usable publication dates. Repeated debugging and qualification consumed development time beyond the browser operation itself.

The later coverage review found unread cross-origin frames on that page. The current build now reports those pages as incomplete and refuses to certify a page-wide newest choice. The earlier 666 ms result remains a historical observation of a narrower reachable-DOM scope, not a passing qualification of the stricter current rule.

## Current general changes

- A batch-capable shared extension now returns snapshot content and the post-snapshot frame identity in one native message. The pre-snapshot identity check remains. A real-Chrome fixture counts three host messages for the first snapshot, down from four; older extensions use the sequential path.
- When a listing supplies an exact same-origin article URL and an unambiguous `datePublished` in JSON-LD, newest-item selection uses that record without fetching the linked page. Missing or conflicting metadata retains the bounded linked-page fallback. The workflow reports `step_ms` and `elapsed_ms` for each deterministic run so site-specific bottlenecks can be measured.
- A click with a lost mouse-dispatch acknowledgement now still performs bounded read-only outcome observation. It never replays the click; a satisfied outcome can be reported as verified while retaining `dispatch_acknowledged:false`. This can avoid a model recovery turn when the page changed but its transport receipt was lost.

| Public tool | Main cost to measure | Current rule |
|---|---|---|
| `browser` | Cold launch, discovery, exact-tab pairing, and acceptance | Reuse an accepted session; keep pairing bound to fresh inventory and browser-side identity. |
| `snapshot` | Native/CDP exchanges, DOM traversal, payload bytes | Batch independent readback and identity; keep explicit omission limits. |
| `act` | Guard, target recheck, input, outcome observation | Preserve checks and never replay an uncertain mutation; inspect its postcondition. |
| `workflow` | Model turns and repeated individual tool calls | Compile bounded steps into one call and report `step_ms`/`elapsed_ms`. |
| `extract` | Selector scan and returned bytes | Use bounded targeted fields; cross-site latency remains unmeasured. |
| `verify` | Independent outcome read | Keep the read; it is the evidence that prevents a fast false completion. |

## Why these boundaries remain

Chrome's native messaging serializes JSON over a persistent port; fewer host exchanges can reduce overhead, but no wall-clock gain is claimed without repeated paired measurements ([Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)). Schema.org defines `NewsArticle.url` and `datePublished`, so exact URL matching can avoid extra page reads when a site publishes them ([Schema.org NewsArticle](https://schema.org/NewsArticle)).

Actionability checks must remain in the hot path: visibility, stability, hit target, and enabled state protect the requested target ([Playwright actionability](https://playwright.dev/docs/actionability)). Re-resolving a locator against current DOM is a proven way to handle rerenders, but it also costs work ([Playwright locators](https://playwright.dev/docs/locators)). Event-driven navigation and DOM observation could reduce polling; WebDriver BiDi defines a bidirectional event model, but replacing the current guarded CDP path needs separate compatibility and safety evidence ([W3C WebDriver BiDi draft](https://www.w3.org/TR/webdriver-bidi/)).

## Qualification before broader speed claims

Run the same cold and warm scenarios across distinct site structures: plain DOM, open shadow DOM, same-origin and cross-origin frames, dynamic lists, rich editors, and pages with and without structured metadata. Record p50/p95 wall time, per-step time, host round trips, browser operations, bytes, false completions, wrong-target events, and independent postconditions. Compare exact build heads and identical Chrome/client environments. Keep failures and timeouts in the distribution. The existing benchmark runner is still a stub; there are no controlled competitor measurements or cross-site latency results for this change.

`npm run check:bench` currently stops at a pre-existing Phase 10 baseline lock digest mismatch. The held-out freeze and lock were left untouched so benchmark provenance is not silently rewritten.
