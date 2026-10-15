# Stage 2 — Complete browser capabilities and human co-use

> **For agentic workers:** Use superpowers:executing-plans task by task. Checkboxes require implementation, transport wiring, independent outcome evidence and review.

**Goal:** Execute the user's real multi-window, visual, file and editing workflows through Controlla without disturbing unrelated human work.

**Architecture:** Build on Stage 1's target registry and capability contract. Browser-local semantic actions and compositor captures are primary; qualified app adapters supply app-specific postconditions. Each provider reports its actual presentation and lifecycle capabilities instead of silently switching profile or foregrounding a tab.

**Tech stack:** Existing Rust runtime, extension CDP sessions, browser DOM/AX/compositor, artifact storage, deterministic workflow engine.

**Spec:** ../specs/2026-10-14-background-window-and-visual-control-design.md; companion 2026-10-14-controlla-stage-1-reliable-contracts.md.

## Global constraints

- Keep six eager tools. Add typed operations to existing contracts.
- Same account and existing target must survive foreground/background transitions.
- “Background” cannot mean activate then restore. Capture from the page compositor, never substitute a desktop screenshot of an occluding window.
- Never silently copy the user's tab into a new profile or assume a private browser is signed in.
- Preserve identity, user edits and unknown-delivery semantics; removing these checks would make the product less reliable.
- Do not claim every Chrome page is automatable. Restricted pages, enterprise policies, authentication challenges, frozen renderers and DRM may impose real boundaries. Return precise capability evidence and supported alternatives.
- Close test-created resources. Test submissions only in Testing Google Classroom.
- Proposed thresholds are targets, not existing measured performance.

## Evidence and gaps

Live audit found working background trusted mouse/touch in an active tab within an unfocused owned window, plus working private-headless PNG and verified 34-byte download. Shared screenshot and upload contracts failed; shared download was explicitly unsupported. Exact existing Canva paired successfully, but inactive-tab pointer selection was refused. The deck stayed on page 7; no claim of reviewing all seven slides is justified.

A separate Chrome AX observer read the fixture's cross-origin child button while Controlla reported one unread frame. This proves the content existed and another observation path could reach it; it does not prove arbitrary cross-origin scripting is safe or that Controlla's frame routing is wired correctly.

Foreground-mode click succeeded in about 39 ms runtime, but the preceding foreground open reported window_focused:false. Treat actual foreground presentation as unqualified.

Not fully live-qualified this audit: inactive-tab CDP dispatch after removing the product restriction, minimized/occluded rendering freshness, cross-origin pointer transforms, drag/drop, IME, passwords, federated login, concurrent human edits, persistent app saves and complete Classroom/Canva workflows. These are explicit implementation and qualification gates below.

## Review focus

1. Capture of a stale or wrong slide despite nonempty image bytes.
2. Focus emulation causing misleading DOM hasFocus without actual window focus.
3. Pointer coordinates across nested transformed out-of-process frames.
4. File selected locally but never accepted or persisted by the app.
5. Human edits or account changes between target resolution and execution.

## S2-01 — Provider and presentation capability matrix

**Files:** capability_registry.rs, browser manager.rs, mcp/v3_full.rs, background.js; add focused presentation-policy tests.

**Interfaces:** PresentationState {window_focused, tab_active, visibility, lifecycle, viewport, activation_epoch, focus_epoch}; ExecutionPolicy {forbid_focus, forbid_activation, allow_scroll, allow_owned_window, deadline}.

- [ ] Instrument tabs/windows focus and activation events independently from page hasFocus. Observe the entire operation, not only before/after snapshots.
- [ ] Qualify foreground, inactive same-window tab, active tab in unfocused separate window, occluded window, minimized window and private headless separately.
- [ ] Test target-addressed CDP input in each mode after a capability probe. Do not infer impossibility from the current blanket activation_forbidden check.
- [ ] Evaluate focus emulation only on owned targets, record its effects and restore state. It cannot count as permission to steal the user's focus or as proof that a frozen page renders.
- [ ] For strict background requests choose a qualified route. If inactive same-window input is unavailable, expose an owned-window alternative; moving an existing user tab requires explicit allowed placement policy.
- [ ] Return unsupported before expensive actionability work when no permitted provider exists.
- [ ] Test browser behind another application, window partially covered, user switching tabs, extension detach and OS display changes.

**Acceptance:** Zero focus/activation events in strict background qualification. A restored final state cannot mask temporary activation. Cross-window work preserves account and exact tab identity.

## S2-02 — Fresh compositor screenshots and deck capture

**Files:** mcp/v2_full.rs visual_probe, runtime observe path, extension screenshot route, artifact store.

**Interface:** VisualEvidence {target, document, frame, viewport_transform, captured_at, visual_revision, dimensions, hash, artifact_handle}.

- [ ] Fix Stage 1 wire contract first, then request bounded renderer/compositor captures with fromSurface where supported.
- [ ] Detect frozen/discarded/crashed targets. Restore only permitted owned lifecycle state; never relabel stale pixels as current.
- [ ] Correlate captured slide/canvas identity with semantic state and render completion. Handle delayed fonts, image decoding, WebGL/canvas repaint and animation.
- [ ] Support viewport crop, element crop, full-slide crop and bounded full-page capture; account for DPI, zoom and offscreen geometry.
- [ ] Preserve image artifacts without forcing base64 into every model response. Return images directly when the user/model must see them.
- [ ] Test occlusion with a visibly changing counter and independent pixel/oracle evidence, not just a nonzero file size.
- [ ] Confirm captures remain correct while another app covers Chrome; separately qualify minimized and inactive tab modes.

**Acceptance:** Seven requested slides produce seven identity-bound image artifacts; identical pixels are not automatically an error if two slides are actually identical. Wrong-slide or stale-frame captures fail qualification.

## S2-03 — Complete frame and shadow-root routing

**Files:** extension debugger auto-attach/session maps, runtime shared-frame snapshot path, pointer transform helpers, capability registry.

**Interface:** FrameHandle {target_id, session_id, frame_id, loader_id, parent_id, origin, transform_revision, accessibility_state}.

- [ ] Build recursive frame/target inventory using frame and debugger events. Chrome flattened auto-attach is not recursively inherited; attach nested child sessions deliberately.
- [ ] Route reads to the correct execution context/AX tree; merge bounded results with source-frame identity.
- [ ] Expose frame handles in snapshots so callers can act without guessing IDs.
- [ ] Compose transforms through scroll, CSS transform, zoom and nested OOPIF offsets. Revalidate document and transform immediately before input.
- [ ] Handle same-process frames, OOPIFs, sandboxed opaque origins, shadow DOM and frame navigation/detach.
- [ ] Test cross-origin child button in the audit fixture, nested cross-origin frames with nonzero offsets, dynamic reparenting and same-name controls in different frames.

**Acceptance:** Child text and actions are reachable when Chrome permits attachment. Unread relevant frames prevent exhaustive claims; unread unrelated frames do not block root actions.

## S2-04 — Input primitives with explicit event semantics

**Files:** act/workflow dispatch, extension input route, widget adapters; shared test fixture.

**Interfaces:** InputPlan {target, mode, expected_state, operation, outcome, deadline}; operation variants click/double/context/hover/scroll/key/type/select/slider/touch/drag.

- [ ] Define and qualify click, double click, context click, hover, wheel, nested-container scroll, scroll-to-target, key chords, press/release and cancellation cleanup.
- [ ] Implement fast insertion, block replace, fast key typing and paced human typing with clear replace/append/selection semantics.
- [ ] Cover contenteditable, rich editors, textarea, controlled React inputs, unicode, emoji, multiline, composition/IME, undo and mid-input re-render.
- [ ] Bound human typing duration honestly. Requested human pace cannot be judged against instant-insert speed; cancellation must release keys and stop queued characters.
- [ ] Qualify native select and custom dropdowns, sliders with min/max/step, date picker and virtualized list.
- [ ] Touch tap/swipe/multi-touch must be separately advertised and tested; current touch success is not evidence for swipe.
- [ ] Drag supports HTML5 DataTransfer and pointer-driven canvases/reordering. Do not require custom data-drop-target attributes on arbitrary sites.
- [ ] Ground source and destination independently, generate required move events and verify meaningful destination state. Browser file drop is separate from element drag.
- [ ] Test no-op handlers, intercepted events, disabled state, overlay, dynamic layout and input event trust requirements.

**Acceptance:** Every advertised primitive passes through the actual public tool in each supported mode. Unsupported variants are explicit before dispatch; no silent substitution of untrusted JS click when the app needs trusted input.

## S2-05 — Files from browser to agent and back

**Files:** artifacts.rs, task_upload/task_download in v3_full.rs, native transport, extension file chooser/download handling.

**Interfaces:** Artifact {handle, mime, filename, size, sha256, source_identity, expiry}; UploadReceipt {selected, consumed, app_accepted, persisted, evidence}; DownloadReceipt {completed, bytes, sha256, source_identity}.

- [ ] Wire guarded inline-byte uploads through the extension's staged-artifact capability rather than contradictory unrestricted-only checks. Do not permit arbitrary filesystem access under an inline-byte grant.
- [ ] Support file input, chooser, iframe chooser, multiple files and app drag/drop upload using scoped staged paths/bytes.
- [ ] Install chooser/download observers before triggering action. Clean waiters on cancel, navigation and detach.
- [ ] Add shared signed-in downloads, authenticated exports and browser-generated blobs with bounded streaming/chunk transfer. Private headless is not a substitute for the user's authenticated page.
- [ ] Verify completion, byte length/hash, filename/type, interruption and overwrite policy. Accept canonical safe destinations without dropping symlink defenses.
- [ ] Observe application upload acceptance and persisted attachment independently. Reconcile unknown completion without duplicate upload.
- [ ] Include Google Drive picker and Classroom attachment flows as adapters using existing browser account state; do not infer account from URL index alone.
- [ ] Test zero-byte/large/multiple files, extension filters, server rejection, upload progress stall, page clearing input after consumption, filename collisions and checksum mismatch.

**Acceptance:** Download a fixture file, inspect/hash it, upload it and verify identical server bytes in shared and headless modes. Classroom qualifies only after the attachment appears in the intended Testing class; publication is separately verified and cleaned up where authorized.

## S2-06 — Secure authentication and sensitive typing

**Files:** secure_fill route, artifact/receipt redaction, client adapters and logging.

- [ ] Accept authorized user-provided credentials for the specified destination through secure_fill; origin/account binding is checked immediately before entry.
- [ ] Never echo values in receipts, errors, screenshots, traces, shell arguments or retained page evidence. Redaction covers failure paths and nested exceptions.
- [ ] Document honestly that values already sent in chat cannot be retroactively removed by Controlla. Offer secret handles/local secure input as optional alternatives.
- [ ] Support ordinary authorized Google sign-in and popup/account selection using observed identities, maintaining opener and destination bindings.
- [ ] Use user handoff where authentication or platform policy requires it; never present bypassing MFA/CAPTCHA as a capability improvement.
- [ ] Use dummy fixture credentials and test accounts for regression, never extract real stored passwords for a benchmark.

**Acceptance:** Secure test login succeeds with zero plaintext secret occurrence in saved evidence; account mismatch stops before disclosure.

## S2-07 — Canva and Classroom adapters grounded in generic capabilities

**Files:** existing app adapter registry and widget paths; add narrowly scoped adapter modules with versioned qualification records.

**Deck interface:** list_slides -> stable IDs; select_slide(id) -> active ID; capture_slide(id) -> image; inspect/edit -> draft/save evidence.
**Classroom interface:** resolve_class/account; open composer; draft; attach artifact; verify attachment; publish only when authorized; verify unique resulting post.

- [ ] Build Canva selection from accessible thumbnail and active-slide state. Use targeted visual resolution only if semantics cannot disambiguate.
- [ ] Capture every slide before claiming a whole-deck review. Track ordered IDs, missing slides, image sources and AI-generated image labels where known.
- [ ] Support explicit edits and verify save state; use undo/draft recovery for incomplete edits. Do not assume screenshot review fulfills rubric requirements.
- [ ] For the user's example, orchestrate PDF retrieval/text+visual extraction, rubric coverage, source research, editable outputs and final attachments as separate evidence-backed steps.
- [ ] Preserve human-writing constraints at the document-generation layer; browser transport should not rewrite the task.
- [ ] Classroom checks exact class, assignment/post destination, account and attachment acceptance. A disabled publish button triggers diagnosis of missing state, never forced enablement.
- [ ] Qualify adapters against controlled fixtures plus disposable real app drafts; keep unsupported UI changes explicit.
- [ ] Restore the user's original slide selection after read-only review when possible without overwriting intervening human activity.

**Acceptance:** A full seven-slide capture manifest and a Testing Classroom post with an audited attachment can be completed and independently verified. Real assignment submission remains outside test authorization.

## S2-08 — Seamless transitions and concurrent sessions

- [ ] Reuse one target handle across foreground/background without release/re-pair.
- [ ] Maintain separate ownership of target state, keyboard state, artifact transfers and app drafts.
- [ ] Run independent tabs across two Codex clients concurrently; serialize only conflicting mutations.
- [ ] When human input affects the same field, yield/reacquire and retain their changes. “Unaffected” cannot mean ignoring contradictory edits.
- [ ] Handle tab moves, popups, new tabs, downloads and closed targets by events. Do not bind navigation outcomes to an assumed original tab.
- [ ] Restore only state still owned by the operation; never restore an old foreground tab over a newer human choice.
- [ ] Stress reconnection and extension reload without duplicate operations or leaked leases.

**Acceptance:** Human use of another window continues without interruption. Same-target conflict is safe and understandable, not a global binding failure.

## Stage 2 qualification matrix

Run every relevant primitive against each mode below; mark unsupported with exact reason, never blank/pass by fixture substitution.

| Dimension | Required cases |
|---|---|
| Presentation | Foreground, inactive same window, separate unfocused window, occluded, minimized, headless |
| Identity | Existing tab, explicit new tab/window, duplicate URL, moved tab, account switch |
| Rendering | DOM, canvas, WebGL, animated content, virtual list, OOPIF, shadow root |
| Input | All typing modes, select/slider, keyboard, pointer, touch, scroll, drag/drop |
| Artifacts | Upload/download/chooser/export/blob, small/large/multiple, failed transfer |
| Interference | Human other-window work, same-field edit, layout drift, reload, disconnect |
| Outcome | No-op, changed state, server accepted, persisted, unknown and reconciliation |
| Cleanup | Only owned tabs/files closed, no private profile/process/waiter leak |

Use local fixtures for fault injection and independent server evidence. Real sites qualify app compatibility separately. Keep browser versions, platform, extension ID/hash and active MCP build in every report.

## Research basis

- [Chrome debugger](https://developer.chrome.com/docs/extensions/reference/api/debugger): child-session routing and nonrecursive auto-attach.
- [CDP Page](https://chromedevtools.github.io/devtools-protocol/tot/Page/): compositor screenshot capabilities; validate actual Chrome behavior.
- [Page lifecycle](https://developer.chrome.com/docs/web-platform/page-lifecycle-api): hidden/frozen/discarded states differ.
- [Playwright input](https://playwright.dev/docs/input): chooser ordering and drag event details.
- [Playwright downloads](https://playwright.dev/docs/downloads): completed artifacts require download lifecycle evidence.
- [Chrome DevTools MCP configuration](https://developer.chrome.com/docs/devtools/agents/get-started/configuration): existing-browser connection is a possible explicitly configured provider, not permission for silent profile substitution.

## Exit gate

Stage 1 is green; Stage 2's supported matrix is implemented and qualified. Any platform-limited cell has a precise tested alternative and does not satisfy a universal claim. Keep remaining unsupported requirements visible in the release summary.

