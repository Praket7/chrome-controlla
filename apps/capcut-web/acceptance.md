# CapCut Web: short captioned video acceptance

Status: **pre-live brief only**. All evidence is `pending`; this file does not claim app support or live behavior. Use owned, licensed test media. This scope is CapCut Web in Chrome only, not native CapCut.

## Brief

Create a 20–30 second, 1080p, 16:9 captioned community clip titled **Three Ways to Stay Cooler This Week**. Use three short owned/licensed shots (shade, water/rest, checking on a neighbor), simple cuts, clean readable captions synchronized to narration, and a quiet licensed/owned music bed under clear narration. Final captions: “Find shade during peak heat.”, “Drink water and take a cool break.”, “Check on a neighbor.” End card: “Plan ahead. Look out for each other.” No medical claims beyond the supplied script. Deliver editable timeline/project if the app exposes one and an exported playable MP4.

Owned assets manifest for each input: asset ID/name, source/rights, checksum, duration, frame rate, dimensions, audio tracks, and expected use. No external user media.

## Exact setup and initial state

- Owned test CapCut account/workspace and isolated test project; record account/project IDs, entitlement, locale, app version, URL/origin and timestamp; redact credentials.
- Create blank project `CC Phase 8 CapCut — Cooler Week — <run-id>`; record project ID, initial timeline duration 0, track/clip inventory empty, version if available, and baseline capture.
- Upload only the three manifest-listed assets and narration/music assets if separate; record uploaded IDs/checksums and confirm no preexisting media or timeline content.
- No collaborators/concurrent edits at baseline. Record Chrome/OS, codec/export options, foreground/background/headless state and active tab identity.

## Task steps

1. Confirm account, workspace/project ID, empty timeline and owned asset identities; stop on mismatch.
2. Import listed media; assemble clips in specified order, trim within source ranges, add cuts, captions and end card, and balance music below narration.
3. Independently read back project/timeline: clip IDs and order, in/out points, total duration, caption text/timing, track inventory and project revision.
4. Reload/reopen project and repeat timeline readback; capture timeline and inspect every caption interval/affected segment.
5. Export MP4; capture export settings/job outcome, artifact checksum, codec, dimensions, frame rate and duration. Independently open/play the full artifact and inspect caption timing, cuts, audio/narration intelligibility and end card. Export job completion alone is not pass.
6. Run wrong-account/project and collaborator/version conflicts below; no blind retry after uncertain export or save.

## Separate scoring rubrics (0–4 each criterion)

**Correctness / persistence (independent evaluator):** correct target/account; all listed media and no other source; clip order/trims/duration within brief; exact caption copy and timing; timeline readback; reload persistence; exported MP4 opens and plays to end with expected dimensions/duration/audio/captions. Wrong project, missing saved state, or false completion is automatic fail.

**Visual/design quality (blinded reviewers):** pacing and shot continuity; caption readability/placement/timing; coherent typography/color; balanced audio and intelligible speech; polished end card; accessibility across background contrast; fit for community audience. 0 fail, 1 major gaps, 2 partial, 3 meets, 4 exceeds. Record reviewers/blinding, per-criterion scores, disagreements and timestamped segment references independently.

## Evidence manifest (all pending)

| Evidence | Required value / artifact | Status |
|---|---|---|
| run binding | run ID, timestamp, task hash, app/client/runtime versions | pending |
| identity and target | account/workspace/project IDs, route, entitlement | pending |
| source assets | IDs, rights/source, checksums, media metadata | pending |
| initial state | empty timeline/tracks/clips, project version, baseline capture | pending |
| dispatch | operation IDs and sent/observed outcomes for import, edits, save, export | pending |
| independent readback | observer/path, clip/track IDs, source ranges, captions/times, duration, version | pending |
| persistence | reopen/reload timestamp and matching timeline readback | pending |
| export | job status, artifact ID/checksum, codec, dimensions, fps, duration, full-playback inspection | pending |
| correctness review | rubric version, criterion scores, evaluator, failures | pending |
| visual review | blinded reviewers/count, scores and timecoded references | pending |
| cleanup | project and uploaded test media disposition | pending |

## Conflict and qualification cases

- Wrong account/workspace/project or duplicate project name: stop before import/edit; exact IDs govern.
- Collaborator changes a clip/caption after validation: detect version/timeline change, preserve collaborator work and yield.
- Source asset ID resolves to different media or checksum: refuse import/use; do not rely on filename.
- Save/export returns timeout or lost response: reconcile project and artifact independently before any retry; uncertain effects stay unknown.
- Unsupported caption/timeline/export control or locked project: mark unsupported/blocked; no substitute native app route.

| Mode | Platform/entitlement | Qualification | Evidence |
|---|---|---|---|
| foreground | Chrome desktop; owned CapCut Web account | unqualified | pending |
| strict background | Chrome desktop; mode-compatible session | unqualified; otherwise `needs_foreground` | pending |
| dedicated headless | headless Chrome; account entitlement | unqualified | pending |

Record each mode independently. Qualification requires saved timeline readback, reload persistence, playable inspected export, conflict handling and blinded visual review.
