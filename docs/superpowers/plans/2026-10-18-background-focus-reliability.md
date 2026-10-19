# Background Focus Reliability and Agent Guidance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Controlla semantic browser work independent of OS focus, recover one stale semantic reference server-side, normalize refusal errors, and give models a compact deterministic route.

**Architecture:** Preserve the six-tool MCP and extension transport. Add a shared presentation/execution contract at the V3 open and action boundaries, extend V2 retained-target recovery to handle stale watcher revisions, keep pointer/native gates capability-specific, and synchronize the runtime instructions with the browser-control skill.

**Tech Stack:** Rust workspace (`controlla-runtime`), Chrome extension JavaScript, Node agent-UX tests, Markdown skill/docs.

**Spec:** `docs/superpowers/specs/2026-10-18-background-focus-reliability-design.md`

## Global Constraints

- Do not change the six normal tool names or the extension pairing protocol.
- Do not activate or focus a user-owned tab as a hidden side effect.
- Do not retry `unknown` mutations.
- Keep secret, OAuth, and full query-string data out of receipts and logs.
- Every production change follows a red-green test cycle.
- Preserve the no-safe-tab fallback and bounded candidate output.

## Review Focus

- Does any background semantic route still call a focus or activation path?
- Can a stale revision cause two writes, a write to a new document, or an unbounded retry?
- Are `unknown`, `not_dispatched`, and presentation warnings distinguishable?
- Can the model complete a new-target task without schema probing, homepage detours, or broad discovery?
- Do error results expose a concrete next action without naming unavailable tools?
- Are secure-fill, native-dialog, CAPTCHA, MFA, order-confirmation, and user-owned pointer safeguards preserved?

## Tasks

### Task 1: Add failing runtime tests for the approved contract

- [ ] **Files:** `crates/controlla-runtime/src/mcp/v2.rs`, `crates/controlla-runtime/src/mcp/v3_full.rs`
- [ ] Add tests for stale watcher revision being eligible for one re-acquisition, refusal after document identity changes, and the existing warning-only foreground open behavior.
- [ ] Add a test that invalid tool arguments are returned as structured results with `reason_code` and `next_action`.
- [ ] Run the focused Rust tests and confirm the new stale/error tests fail for the current implementation.

**Expected:** Existing warning-only tests pass; the new stale and structured-result tests fail for the missing behavior.

### Task 2: Implement bounded stale watcher re-acquisition

- [ ] **Files:** `crates/controlla-runtime/src/mcp/v2.rs`, related reference/receipt helpers and tests.
- [ ] Extend the retained click/mutation retry predicate to include `stale_watcher_revision` only for `not_dispatched` receipts.
- [ ] Reuse the existing bounded snapshot and `reference_reacquire` path; require same tab, document identity, unique role/name/href/ancestor match, and one retry maximum.
- [ ] Preserve `unknown` as terminal and include `reacquisition` evidence in the receipt.
- [ ] Run the Task 1 focused tests and the existing V2 mutation tests.

**Expected:** Stale revisions rebound once when safe; changed documents and ambiguous targets remain not dispatched; no unknown retry is added.

### Task 3: Harden structured invalid and stale tool results

- [ ] **Files:** `crates/controlla-runtime/src/mcp/v3_full.rs`, error helpers/tests.
- [ ] Audit every V3 invalid/stale route that can escape `call_tool` and normalize it through the structured tool-result path.
- [ ] Ensure receipts consistently include `status`, `reason_code`, `reason`, `dispatch_may_have_occurred`, and `next_action` where applicable.
- [ ] Replace any stale or invalid error text that names tools outside the six-tool surface.
- [ ] Run the new structured-result tests and the V3 focused test set.

**Expected:** No tested invalid or stale request becomes a bare JSON-RPC `-32602`; model-visible output gives one actionable next step.

### Task 4: Make routing and focus semantics explicit in tests

- [ ] **Files:** `crates/controlla-runtime/src/mcp/v3_full.rs`, `packages/chrome-controlla/test-agent-ux.cjs`, `extensions/chrome-controlla/test-background.cjs`.
- [ ] Add route tests for bounded rejected candidates, redacted URLs, exact new-target cold-open fallback, and explicit existing-tab `no_safe_tab_match` refusal.
- [ ] Add agent-UX coverage for foreground warning with workflow dispatch, background semantic work without activation, and direct ESPN/Bills/newest-story call budget.
- [ ] Add extension coverage for inactive semantic operations and continued refusal of inactive user-owned pointer input.
- [ ] Run the focused Node suites and confirm all new tests pass.

**Expected:** The route is deterministic, compact, and safe; the background/focus matrix is executable rather than instruction-only.

### Task 5: Rewrite runtime instructions and the browser-control skill

- [ ] **Files:** `crates/controlla-runtime/src/mcp/v3_full.rs`, `packages/chrome-controlla/plugin/skills/browser-control/SKILL.md`, relevant instruction tests.
- [ ] Replace duplicated prose with the three canonical recipes and a compact state machine from the spec.
- [ ] State the focus/capability matrix, one stale-reacquisition rule, unknown-outcome rule, `$name` syntax, and no-safe-tab fallback in the first screenful.
- [ ] Keep sensitive-work confirmation and challenge hand-off rules explicit.
- [ ] Update instruction assertions to reject schema probing, homepage detours, and blind retries while requiring the new fallback language.
- [ ] Run the instruction and agent-UX tests.

**Expected:** A smaller model can choose the route from one instruction block without reading repository files or probing schemas.

### Task 6: Add observability and regression checks

- [ ] **Files:** watcher/receipt helpers, extension tests, runtime tests, and docs as needed.
- [ ] Add bounded counters or receipt fields for stale invalidations, re-acquisition attempts/outcomes, and presentation warnings without exposing secrets.
- [ ] Add an event-storm regression test showing window/group updates do not rewrite unrelated tab identity revisions.
- [ ] Run the extension background suite, focused Rust suites, agent-UX suite, formatting, and the repository’s required checks.

**Expected:** Revision churn is measurable, unrelated tab references stay stable across topology updates, and all required checks pass.

### Task 7: Fresh whole-branch review and fix pass

- [ ] Review the complete diff against the spec and Review Focus.
- [ ] Re-run all affected tests after any Critical or Important fix, using a failing regression test first.
- [ ] Record deferred minor findings and any rulings in the execution ledger.
- [ ] Verify the final diff, branch status, and exact extension/MCP reload boundary.

**Expected:** The branch satisfies the spec, all required checks are green, and live qualification limits are stated accurately.

## Verification commands

```bash
node extensions/chrome-controlla/test-background.cjs
cargo test -p controlla-runtime --locked --bin controlla-v2 v2:: -- --nocapture
cargo test -p controlla-runtime --locked --bin controlla-v2 v3_batch_delivery_tests -- --nocapture
node packages/chrome-controlla/test-agent-ux.cjs
cargo fmt --all -- --check
git diff --check
```

Run the repository’s broader required test/build command if the checkout documents one; do not treat focused tests as proof of live Chrome qualification.
