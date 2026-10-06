# Phase 4 implementation report

## Status

Partial implementation; Phase 4 acceptance is **not complete**. This commit provides typed guard primitives and focused fixtures only. No claim is made that browser inputs are dispatched safely or that native state is isolated.

## Changes

- Added `crates/controlla-browser/src/input.rs` with typed semantic locator/action/snapshot/decision/invalidation/evidence records.
- `validate_step` fails closed on navigation/account/document revision or dependency snapshot differences; strict-background plus a declared native requirement returns `NeedsForeground`.
- `perform_input` validates immediately before calling an injected action callback. It is a fixture seam, not a CDP dispatch function.
- `on_external_change` records an explicit invalidation key. `measure_check_dispatch_race` times an arbitrary closure only; it is not a measured real browser race.
- Exported the types and functions from the browser crate. No dependencies or imported source were added. Updated the existing extracted `lib.rs` digest/edit note in `provenance/extraction.json`.
- Updated `docs/progress.md`, `docs/verification-matrix.md`, and `docs/blockers.md` with partial status and remaining gates.

## Tests and evidence

Focused command:

```text
cargo test -p controlla-browser --locked --offline input::tests
```

Output:

```text
running 4 tests
test input::tests::race_window_is_measured_and_external_change_invalidates ... ok
test input::tests::guarded_dispatch_preserves_unicode_and_reports_observed_fixture_value ... ok
test input::tests::strict_background_never_routes_through_native_clipboard_or_focus ... ok
test input::tests::relevant_changes_yield_and_unrelated_changes_may_continue ... ok

test result: ok. 4 passed; 0 failed; 0 ignored
```

The tests are callback/type-level fixtures. Unicode is echoed through a fixture callback, not written to an actual field. Strict-background guarantees only that the callback is not invoked when native routing is declared necessary.

Red-first evidence was **not recorded**: tests were run after implementation, so no prior failing run is claimed.

Workspace command:

```text
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test --workspace --locked --offline --quiet
```

Output summary:

```text
Clippy completed successfully with -D warnings.
Browser unit tests: 68 passed, 1 ignored, 0 failed.
Phase2 integration: 7 passed, 0 failed.
Runtime unit tests: 3 passed, 0 failed.
CLI integration: 8 passed, 0 failed.
Jobs integration: 10 passed, 0 failed.
Registry integration: 3 passed, 0 failed.
Formatting check passed.
```

Docs/provenance/dependency command:

```text
npm run check:provenance && node scripts/check-doc-links.mjs && git diff --check && ./scripts/check-dependencies.sh
```

Output:

```text
Verified 13 destination digests and 22 retained tests.
Checked local Markdown links in 14 files.
All checks exited 0; dependency script emitted no error output.
```

An initial combined check exited nonzero because rustfmt had not yet been applied and the extracted `lib.rs` provenance digest was stale. After formatting and updating the digest/edit note, the checks above passed.

## Not implemented / unqualified

- Semantic locator resolution, actual CDP input dispatch, and actual input value/position postconditions.
- Masked, contenteditable, grapheme/IME, or event-dependent behavior.
- Click geometry/hit target revalidation and overlay interception.
- Drag geometry refresh and DOM/canvas result fixtures.
- Live field edit/account switch/unrelated churn/frame swap injection at check/dispatch.
- Real browser check-to-dispatch race measurement. The included timing helper merely measures closure duration.
- Internal clipboard insertion and independent native focus/cursor/clipboard observers. B24/B25 remain unqualified.
- No page content expands authority; the new API accepts caller-supplied revisions only.

## Acceptance boundary

The repository's existing target/frame/browser generations and identity comparisons remain useful stale-reference checks, but application account/document revisions are caller-supplied. Browser-only fixtures cannot prove OS focus/cursor/clipboard isolation. The actual Phase 4 gate remains open pending browser dispatch/DOM fixtures, race injection, and native observation for OS claims.

## Continuation — revision-bound CDP actions and installed Chrome evidence

### Status

The implemented supported-control scope is complete and locally verified. The wider Phase 4 acceptance remains partial: caller-supplied account/document revisions have no authoritative app observer; masks, contenteditable and IME/event-dependent controls are unqualified; real overlay/drag/canvas effects are unqualified; and native focus/cursor/clipboard isolation remains unverified without an independent OS observer. Strict-background text routes use page/CDP operations only, and mouse/native routes return `NeedsForeground`. Page content never expands authority.

### Continuation changes

- Replaced the callback-only input seam with revision-bound commands through `BrowserConnection::target_ref_command`, using the existing target/frame references and fail-closed supported semantic locators.
- Added readback/postconditions for ordinary text input and textarea fill, insert, and printable-ASCII sequential keys. IME and unsupported field types explicitly return unsupported. Click and drag require caller-declared observable predicates, recheck fresh geometry/hit target, bound finite coordinates to viewport, dispatch CDP mouse events, and verify the predicate.
- The websocket fixture verifies stale identity/dependency handling, ambiguous/empty locators, strict-background mouse behavior, click/drag sequences and declared predicates, and a navigation event injected after the last focus/probe check. The mutation request following the injected event is withheld. Its actual fixture interval from initial guarded probe to fill request arrival measured **2773 us** in the recorded run; the residual gap between local validation and Chrome/page effects is non-atomic.
- The ignored installed-Chrome fixture exposed that newly created targets were attached after manager bootstrap but their Page/Runtime domains were not enabled. Dedicated target launch now waits for the attached flattened session and bootstraps those domains before returning. The fixture exercises ordinary `<input type="text">` fill (`héllo 👋`), `Input.insertText` (`λ`), printable ASCII sequential key dispatch (`a`), and live DOM/caret readback (`héllo 👋λa`, `selectionStart=10` UTF-16 code units). Test cleanup disables preserve-on-drop for this isolated session, then explicitly shuts Chrome down; a failed assertion also drops and stops the exact owned process/profile.

### Red/green evidence

The installed Chrome fixture first failed with `input fixture navigation did not create a frame` and an empty frame graph. The attached target had not received `Page.enable`/`Runtime.enable` because it was created after initial connection bootstrap. After adding target-session bootstrap to dedicated launch, that same fixture passed and reported the expected DOM value and caret position. A later fixture assertion initially indexed the CDP return envelope at the wrong level (`Null` vs the expected text); correcting the assertion to the observed `Runtime.evaluate` envelope produced the passing run. Earlier callback-only tests have no recorded pre-implementation RED run.

### Exact focused command/output evidence

Command:

```text
cargo test -p controlla-browser --locked --offline guarded_text_actions_use_revision_bound_cdp_and_race_yields -- --nocapture
```

Output:

```text
running 1 test
CDP fixture probe-to-fill dispatch interval: 2773 us
test tests::guarded_text_actions_use_revision_bound_cdp_and_race_yields ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 70 filtered out
```

Command:

```text
cargo test -p controlla-browser --locked --offline real_chrome_headless_provider_launch_and_runtime_smoke -- --ignored --nocapture
```

Output:

```text
running 1 test
test providers::tests::real_chrome_headless_provider_launch_and_runtime_smoke ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 70 filtered out
```

The installed Chrome emitted non-fatal macOS display/GPU diagnostics on stderr. They did not prevent the DOM actions, readback, caret assertion, or isolated-session shutdown from passing.

Full Rust, docs, provenance, dependency, and whitespace check output will be appended after the final verification run. No native observer or broad app/control qualification is claimed.

### Final verification evidence

Command:

```text
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked --offline -- -D warnings && cargo test --workspace --locked --offline --quiet
```

Output:

```text
Checking controlla-browser v0.1.0 (...)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.69s

running 71 tests
...test result: ok. 70 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out

running 7 tests
...test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

running 3 tests
...test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

running 0 tests
...test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

running 8 tests
...test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

running 10 tests
...test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

running 3 tests
...test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

The omitted lines are cargo's zero-test target summaries. Rust formatting and warning-free Clippy passed before the listed workspace test groups.

Command:

```text
npm run check:provenance && node scripts/check-doc-links.mjs && git diff --check && ./scripts/check-dependencies.sh
```

Output:

```text
> check:provenance
> node scripts/check-provenance.mjs

Verified 13 destination digests and 22 retained tests.
Checked local Markdown links in 15 files.
```

All four chained commands exited 0; the dependency and diff checks emitted no additional output. Focused `input::tests` also passed 5/5. The ignored Chrome fixture passed again after formatting and Clippy fixes.

## Follow-up race guards — 2026-10-06

- Fill now checks `e.value === expected_value` inside the same synchronous `Runtime.evaluate` that sets the value; mismatch returns the distinct `InputOutcome::Stale` result.
- Insert refreshes value/focus immediately before its CDP send. Sequential typing requires a collapsed caret at the UTF-16 end of the current value, then refreshes value/focus/caret before every `keyDown`, `char`, and `keyUp`, advancing the expected value and caret after each character event.
- After a keyDown is attempted, stale/error exits attempt a matching keyUp as best-effort cleanup. This sends no text and uses the same target/session authority check, so release is not guaranteed if that authority has gone stale.
- These checks narrow the gap only. CDP handling, page event handlers, and server-side effects are not atomic, and the fixture does not claim otherwise.
- Focused verification: `cargo test -p controlla-browser --locked --offline guarded_text_actions_use_revision_bound_cdp_and_race_yields -- --nocapture` passed (1 test); mock fixture timing 716 us. Existing test checks the final fill expression includes the live-value comparison, exercises the per-send guards, and verifies target-navigation race withholding. It does not inject a page-side value mutation between every individual send.
- No imported source changed; provenance digest was not changed.
