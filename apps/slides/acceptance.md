# Google Slides: 10-slide editable deck acceptance

Status: **pre-live brief only**. All evidence is `pending`; this file does not claim app support or live behavior. Use an owned test account and owned source content. Browser-only and authorized API-assisted runs are separate results.

## Brief

Create a 10-slide, editable presentation titled **Urban Heat: A Neighborhood Field Guide** for a city community meeting. Audience: residents with no technical background. Goal: explain the urban heat-island effect and give three practical, locally actionable responses. Use exactly 10 slides: (1) title and one-sentence promise; (2) what urban heat is; (3) why neighborhoods differ; (4) one clearly labeled illustrative data chart; (5) health impacts; (6) who faces greatest exposure; (7) shade and trees; (8) cool roofs and buildings; (9) a three-step neighborhood action plan; (10) sources and a concise call to action. Do not invent measured local data: label illustrative values and include credible source names/URLs on slide 10. Deliver editable native text and shapes, one editable chart or chart-like grouped shapes, consistent visual system, and a PDF export.

## Exact setup and initial state

- Fresh owned test Google account, no shared personal profile; test tenant and billing/entitlement recorded without secrets.
- Create a new blank Slides test document named `CC Phase 8 Slides — Urban Heat — <run-id>` in a dedicated test Drive folder. Record document ID, owner, initial revision/version, URL/origin and timestamp.
- No collaborators, comments, existing objects, or concurrent edits at baseline. Capture initial slide count (0), object inventory (empty), and screenshot/render.
- Grant only the documented OAuth scopes needed for the selected route. Record consent scope names, app/client version, route (`browser_only` or `api_assisted`), Chrome version, OS, locale, and session mode. No credentials or tokens in evidence.
- Record browser foreground state and active tab identity. Background/headless runs start with Chrome unfocused and no prior app tab unless the mode contract says otherwise.

## Task steps

1. Confirm account, Drive folder, document ID, edit access, and starting revision before mutation; stop on mismatch.
2. Build the ten specified slides with editable titles/body/shapes/chart elements; use stable object IDs and revision-aware writes for API-assisted route.
3. Read back slide order, count, text, object types/IDs, key geometry/style fields, and revision through an independent read path.
4. Reload the document; repeat structural readback and capture rendered evidence for all ten slides.
5. Export PDF; record export job/result and artifact checksum, page count, and independent file-open/render inspection. Submission of an export job alone is not pass evidence.
6. Run account/project conflict variants below. Record stop/yield behavior before any write.

## Separate scoring rubrics (0–4 each criterion)

**Correctness / persistence (independent evaluator):** exact slide count/order; required content and sources; editability/object structure; correct target identity/account; no unapproved mutation; readback matches requested state; reload persistence and revision continuity; PDF opens with ten pages and matches saved deck. Score each criterion 0 fail, 1 major gaps, 2 partial, 3 meets, 4 exceeds; any wrong account/document or false completion is an automatic fail regardless of average.

**Visual/design quality (blinded reviewers, without correctness score):** hierarchy/readability at presentation size; consistent typography/color/grid; chart clarity and honest labeling; visual variety without inconsistency; whitespace/alignment; audience fit and accessibility/contrast; source/call-to-action legibility. Same 0–4 anchors; report reviewer count, blinded status, per-criterion scores, disagreements, and representative slide references. Correctness cannot increase design score.

## Evidence manifest (all pending)

| Evidence | Required value / artifact | Status |
|---|---|---|
| run binding | run ID, timestamp, task hash, app/client/runtime versions | pending |
| identity and target | principal/account ID hash, Drive folder ID, document ID, owner, route, scope names | pending |
| initial state | revision, zero-slide/object inventory, baseline capture | pending |
| dispatch | operation IDs, requested object IDs, sent/observed status, policy decision | pending |
| independent readback | observer identity/path, slide/object inventory, text/geometry, revision | pending |
| persistence | reload timestamp, post-reload readback, revision and comparison | pending |
| export | export format, job ID, completion state, artifact path/ID, checksum, page count, independent render review | pending |
| correctness review | rubric version, criterion scores, evaluator, failures | pending |
| visual review | blinded reviewers/count, rubric scores, slide references | pending |
| cleanup | owned test doc disposition and remaining resources | pending |

## Conflict and qualification cases

- Wrong Google account active, wrong Drive folder, or document ID changed: deny before mutation; capture observed identity and denial.
- Collaborator changes a field/slide after plan validation but before dispatch: detect revision conflict, preserve collaborator edit, yield/replan; no silent overwrite.
- Same document opened in two tabs/sessions: serialize or reject stale revision; verify no lost update.
- OAuth revoked/insufficient scope or ownership changed: fail closed and report required scope/permission without retrying writes.
- Duplicate title with different document ID: bind by exact ID, never title alone.

| Mode × route | Platform/entitlement | Qualification | Evidence |
|---|---|---|---|
| foreground × browser-only | Chrome desktop; owned Google account | unqualified | pending |
| strict background × browser-only | Chrome desktop; account/session permitted in mode | unqualified; unsupported action must return `needs_foreground` | pending |
| dedicated headless × browser-only | Chrome headless; account entitlement | unqualified | pending |
| foreground × API-assisted | authorized Slides API OAuth scopes + Chrome inspection | unqualified | pending |
| strict background × API-assisted | authorized API scopes; no focus/cursor/clipboard use | unqualified | pending |
| dedicated headless × API-assisted | authorized API scopes + headless Chrome inspection | unqualified | pending |

No row becomes qualified until its own saved-output, readback, reload, export, conflict, and review evidence is attached.
