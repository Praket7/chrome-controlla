# Controlla Fast Agent Execution: Remaining Work

> **For agentic workers:** Use `superpowers:executing-plans` to carry out this plan. Complete the checked items in order and record evidence for each gate.

**Goal:** Make common Controlla browser tasks fast and predictable by teaching the agent the shortest correct route, removing misleading tool-contract friction, and measuring the complete task rather than isolated calls.

**Architecture:** Prefer the existing snapshot, V3 workflow, artifact, upload, and verification primitives. Fix shared outcome/reporting semantics where they cause wasted waits. Add only the missing generic workflow composition needed to avoid unnecessary round trips. Keep independent postcondition checks; never turn an uncertain result into success or blindly replay a mutation.

**Tech Stack:** Rust workspace, Chrome extension, Controlla MCP V3, existing integration fixtures and live qualification scripts.

**Scope:** This is the remaining performance and agent-usability work from the ESPN foreground and Classroom upload runs. It does not replace the Stage 1 build plan or claim all its exit gates are complete.

## Agent Operating Card

Use this sequence before exploring alternatives. Read the live tool schema once; Controlla is the browser-control route for this task.

1. **Choose the tab deliberately.** Reuse the selected tab for navigation. Call `open` only when the user asked for a new tab. Include its required `idempotency_key`. Do not retry an `unknown` mutation until fresh state has been inspected.
2. **Take one small, useful snapshot.** Scope it with a known text query or `href_hint`. Avoid whole-page dumps and speculative selectors. Use the snapshot’s visible links and controls to choose the next action.
3. **For “newest story,” rank evidence rather than guessing.** Find story links, choose by parsed `datetime`, then click the chosen candidate. Request a navigation outcome without a relative URL predicate, or use the exact absolute destination URL. Compare the observed final URL with the chosen candidate.
4. **For an announcement with a file:** find the existing composer; type using the requested speed mode; register the local file once; invoke upload on the visible upload control; require a visible attachment/accepted-file postcondition before posting. If the upload result is uncertain, inspect once and decide from the observed state. After posting, verify the announcement text and attachment in the posted item.
5. **Finish cleanly.** Release the Controlla session without closing the user’s tab. Report completion only from the observed postcondition.

The one-minute upload/post goal is a benchmark target for a prepared session (signed in, composer open or quickly reachable, local file ready, stable network), not a promise across every site or login state. Measure from the first Controlla operation through verified post visibility.

## Diagnosis: Why These Runs Took Minutes

### ESPN newest-story navigation: 3:02 total

- The open call took about 1.2 seconds. A valid four-step workflow took about 10.3 seconds: snapshot, find story links, rank by `datetime`, click.
- The click itself waited about 10 seconds and returned `unknown` even though navigation succeeded. The caller supplied a relative story prefix where the current outcome check compares a supplied URL to the observed URL as an exact string. The actual URL was absolute, so the postcondition could never match.
- Most remaining time came from the agent’s avoidable discovery detours: an open call missing its required idempotency key, broad or unproductive snapshots, a task call missing required `urls`, and repeated inspection to discover the valid route. Those were orchestration mistakes made easier by a contract that does not make required fields and outcome semantics obvious.
- The short successful route was already available: scoped snapshot → ranked story links → choose maximum parsed `datetime` → click with a satisfiable navigation condition → compare the observed URL to the selected link.

### Classroom typing, upload, and post: 4:19 total

- This duration is a whole Codex-turn measurement, not a per-call benchmark. The archived turn does not retain enough per-call timestamps to assign every second precisely; do not present a speculative breakdown as measured fact.
- Typing and posting worked. Human-paced typing intentionally emits real key events; it is slower than fast exact typing and should be selected only when the user asks for human-like typing or the task needs per-keystroke behavior.
- The upload trigger click waited about 10 seconds and returned `unknown` while the Drive picker opened in a child frame. That was a wrong click postcondition, not evidence that file transfer failed. The file later appeared attached and the announcement posted.
- The upload path can report `unknown` when the site consumes/replaces the file input and no application-acceptance predicate was supplied. The reporting lacked a verified attachment postcondition at the point it returned, so more inspection was needed. Do not change the file-transfer mechanism that succeeded; improve the evidence and outcome reporting around it.
- The current V3 workflow can chain snapshot/find/choose/click/type operations but has no first-class upload step. Artifact registration and upload therefore require separate round trips. One prior post-upload inspection was repeated after an output-script variable shadowed its output helper; that was an agent-side diagnostic-script mistake, not a repository bug. The runbook should avoid stateful helper names that collide with tool functions.
- A 45-second upload deadline is not an explanation for the full 4:19. The excess includes agent orchestration and retries/inspection. Add phase timing before attributing more time to upload internals.

## Remaining Work

### P0 — Make the shortest valid route obvious and reliable

- [x] **Clarify and validate navigation outcomes.** Exact navigation destinations now require valid absolute HTTP(S) URLs before dispatch; navigation-only remains explicit, and canonical redirect prefixes use independent `verify`. Regression covers exact match/mismatch, relative rejection, and no document change.
- [x] **Make the required `open` contract discoverable.** Runtime validation, MCP schema description, `browser describe`, tool description, AGENTS guidance, and browser-control skill specify required `idempotency_key` and same-key retry/new-key new-tab semantics. Stdio contract tests check the schema.
- [x] **Keep one focused snapshot route documented.** AGENTS and the Controlla skill specify one scoped snapshot, V3 workflow, ranked newest selection, inspect-once-after-unknown, and release.

### P1 — Report upload success from what the page accepted

- [x] **Preserve the working transfer mechanism.** The existing chooser/file-input transfer route is unchanged. The installed-Chrome fixture exercised it through the new workflow entry point.
- [x] **Return a useful, typed upload outcome.** Existing selection/app-acceptance/persistence fields are preserved. Regressions cover consumed input and require a newly satisfied acceptance predicate before reporting verified; otherwise the result remains selected/unknown with a reason.
- [x] **Avoid waiting for the wrong thing.** Trigger upload uses a bounded focused click/chooser outcome followed by independent file/app acceptance checks.
- [x] **Add upload to workflow composition.** V3 `Upload` workflow step takes an explicit registered artifact, observed trigger, account marker, and optional acceptance predicate; it reuses the same upload route and stops before subsequent mutations unless app acceptance is verified.
- [x] **Prevent diagnostic-script output shadowing.** Corrected the plan: this was an agent scratch-script naming error, not a repository defect. The usage notes now instruct keeping scratch names distinct and avoiding a second page read when a local script fails before dispatch.

### P1 — Remove avoidable calls without weakening verification

- [x] **Use fast typing by default for exact text.** Guidance omits `typing_mode` so Controlla selects Block insertion, FastKeys for trusted-event controls, or IME when appropriate; HumanKeys is reserved for explicit human-paced requests.
- [x] **Inspect only after a real uncertainty.** Guidance requires one scoped read-only inspection after `unknown`, no blind replay, and no duplicate snapshot after a verified receipt.
- [x] **Use the existing artifact once.** Artifact registration is explicit and session scoped; workflow reuses its returned handle and reports registration/upload/acceptance phases separately.
- [x] **Measure the call boundary.** Registration, browser tasks, workflow steps, and open return elapsed timings. Repeated live p50/p95 measurement remains open below.

### P2 — Prove the improvement across Stage 1 and real tasks

- [x] **Cover newest selection, upload acceptance, and text readback.** `espn_latest_story_flow_completes_and_verifies_newest` asserts the chosen `href` is the maximum-timestamp fixture; shared navigation unit tests cover exact absolute destination, mismatch, no document change, and relative-URL rejection. `shared-extension V3 FastKeys and visible-trigger file upload` exercises the child-frame chooser and requires accepted app state; workflow tests stop on selected/unknown upload receipts. Installed-Chrome typing fixtures assert exact value readback. These are local fixture tests, not Classroom app proof.
- [x] **Stop a compiled workflow after an uncertain post mutation and preserve selected tabs on release.** A regression asserts an `unknown` mutation stops before a later click/mutation while allowing a next read-only verify; the extension route fixture asserts release detaches debugger control while leaving the user's tab in Chrome. There is no generic application-level post idempotency key, so independent agent calls after `unknown` must still inspect state and must never be automatic retries.
- [x] **Run current local gates.** Stage 1 fixture script, full workspace tests, warning-denied Clippy, docs, client/V3, and dependency checks passed on the working tree based on `406b9d9`; exact evidence and the dirty-tree boundary are in `docs/verification-matrix.md`.
- [ ] **Requalify live foreground and background.** On the current installed extension/MCP revision, run representative foreground tasks (scoped navigation/ranking, typing, file upload, verified post) and background tasks. Record start/end times, tool calls, retries, unknown outcomes, focus changes, and the observed final state. Do not treat a fixture as live evidence.
- [ ] **Compare versions with matched tasks.** Complete the Stage 1 C1, C2-before, and C2-after comparison with repeated runs and report sample count, verified success, wrong-target/stale/unknown/false-positive outcomes, calls, bytes, p50/p95 latency, recovery time, and focus disruption. A single run cannot establish a performance distribution.
- [ ] **Evaluate the one-minute target fairly.** For the prepared Classroom case, report median and p95 from first Controlla call to independently verified post. State login/composer/file/network preconditions. If p95 exceeds 60 seconds, identify the measured dominant phase and optimize that phase; do not loosen the verification gate to hit the time.
- [ ] **Close remaining Stage 1 gates explicitly.** Recheck every exit condition in `CONTROLLA2_STAGE1_FIX_PLAN.md` and update the plan/status only from current-revision evidence. In particular, do not call Stage 1 complete while matched C1/C2-after metrics, live foreground/background qualification, or other listed exit gates remain unverified.

**Current result:** Local implementation and source-level qualification are strong. The duplicate-post uncertainty stop and selected-tab-preserving release regressions are implemented and covered by local tests. A fresh review caught raw URL-string equality misreporting successful browser-normalized navigations; Rust and frame-click paths now use browser URL canonicalization, with regression coverage, and a subsequent fresh review found no further issue in that area. The plan is not fully complete until live foreground/background qualification, live Classroom acceptance and persistence, matched C1/C2/after performance distributions, and the one-minute target evaluation are closed on the reloaded runtime. The visible-trigger upload workflow is local-unrestricted only; guarded uploads remain on the dedicated headless task path.

## Done Means

- The agent can follow the operating card without schema errors or exploratory calls.
- The ESPN newest-story run selects the maximum parsed timestamp, navigates to that exact candidate, and does not spend a default timeout on a mismatched URL predicate.
- Upload reporting distinguishes transfer from visible application acceptance, including when the file input is consumed.
- The Classroom workflow verifies text and the exact attachment before posting and verifies both in the posted announcement afterward.
- No successful mutation is blindly replayed after `unknown`; no user tab is closed during release.
- All local gates pass, live evidence is tied to the exact tested revision, and latency is reported as measured p50/p95 with stated preconditions.

## Review Focus

- Is each required field obvious and schema-accurate?
- Does each expected outcome match the actual action (picker launch versus page navigation)?
- Can upload success be proven from the application state without relying on a transient file input?
- Are timing claims based on per-stage traces and repeated matched runs?
- Does the plan preserve independent postcondition checks and avoid duplicate posts?
