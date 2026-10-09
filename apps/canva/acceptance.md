# Canva: consistent multi-page set acceptance

Status: **pre-live brief only**. All evidence is `pending`; this file does not claim app support or live behavior. Use owned content, a test design, and explicit scoped Connect/Apps SDK authorization where applicable. Browser and SDK/Connect routes are separately qualified.

## Brief

Create a cohesive five-page **Neighborhood Heat-Ready Kit** for a community organization: (1) cover/one-line promise; (2) three heat-safety actions; (3) a simple neighborhood preparedness checklist; (4) shareable event announcement with editable date/time/location placeholders; (5) contact/next-steps page. Audience: general residents, mobile-first. Use one consistent type/color system, repeated grid and footer, readable contrast, editable text/shapes, restrained original/owned imagery, and no false local statistics. Export all pages as a five-page PDF and as individual PNGs if supported by the selected route.

## Exact setup and initial state

- Owned test Canva account and dedicated test team/workspace; record account, workspace/project IDs, role/entitlement, locale, plan tier, app/client versions and timestamp; redact secrets.
- Start from a new blank design named `CC Phase 8 Canva — Heat-Ready — <run-id>`. Record exact design ID, owner, URL/origin, initial version/revision, page count 0, and empty element inventory.
- No collaborators or concurrent edits at baseline. Capture initial UI/design state and confirm the selected workspace before writing.
- Record authorized Connect scopes and Apps SDK bridge/version when used; do not treat authorization or `sync` response as persistence. Browser-only and scoped API/SDK route results remain distinct.
- Record Chrome version, OS, foreground/background/headless mode, active tab identity, and whether the design is locked/template-derived.

## Task steps

1. Verify exact account, team/workspace, design ID, editability/unlocked state, and revision; stop if any is ambiguous.
2. Create five ordered pages with the specified content and reusable visual system; preserve editable text and shapes.
3. Independently read back design ID, page order/count, element IDs/types/text, key styles/geometry and current version.
4. Reload/reopen exact design and repeat readback; inspect every page in rendered form.
5. Export PDF and supported PNGs; record job status, checksums, page/dimension counts and independent open/render inspection. Export submission alone is not success.
6. Exercise `sync` as a mutation: record before/after versions and all changed elements; include sync conflict, expiry, and locked-page cases below.

## Separate scoring rubrics (0–4 each criterion)

**Correctness / persistence (independent evaluator):** exactly five required pages/order; required copy/placeholders present; editable objects and stable IDs; correct account/workspace/design identity; unauthorized/locked state causes no write; independent readback matches; reload persistence; sync mutation accurately reconciled; exports open and represent all pages. Any wrong project/account or false completion is automatic fail.

**Visual/design quality (blinded reviewers):** coherent system across pages; clear information hierarchy; mobile-size legibility; contrast/accessibility; alignment/spacing; appropriate, consistent imagery; useful page-to-page variation; polished event placeholders/contact details. 0 fail, 1 major gaps, 2 partial, 3 meets, 4 exceeds. Record reviewers, blinding, scores, disagreements and page references separately from correctness.

## Evidence manifest (all pending)

| Evidence | Required value / artifact | Status |
|---|---|---|
| run binding | run ID, timestamp, task hash, app/client/runtime versions | pending |
| identity and target | account/team/design IDs, role, route, scope names | pending |
| initial state | version, empty page/element inventory, baseline capture, lock/template state | pending |
| dispatch and sync | operation IDs; before/after version; changed element IDs; sync mutation record | pending |
| independent readback | observer/path, page and element inventory, text/styles, version | pending |
| persistence | reopen/reload evidence and matching readback | pending |
| exports | job IDs/status, PDF/PNG artifact IDs/checksums, page/dimension counts, render inspection | pending |
| correctness review | rubric version, criterion scores, evaluator, failures | pending |
| visual review | blinded reviewers/count, rubric scores and page references | pending |
| cleanup | test design/team artifact disposition and remaining resources | pending |

## Conflict and qualification cases

- Wrong personal account or wrong team/workspace active: stop before mutation; do not select by matching display name.
- Same design title in another team: bind exact design ID and workspace ID; deny mismatch.
- Collaborator edits an element between observation and write: detect conflict/version change, preserve edit and yield.
- One-minute session/token expiry: no stale sync/write; refresh only through authorized flow, then revalidate identity/revision.
- Locked page, unsupported element, template restriction, or lost edit permission: report unsupported/blocked without partial false completion.
- `sync` writes state while serving as read probe: compare before/after version and element inventory; classify as mutation and verify its effects.

| Mode × route | Platform/entitlement | Qualification | Evidence |
|---|---|---|---|
| foreground × browser-only | Chrome desktop; owned Canva account/team | unqualified | pending |
| strict background × browser-only | Chrome desktop; mode-compatible session | unqualified; otherwise `needs_foreground` | pending |
| dedicated headless × browser-only | headless Chrome; Canva account entitlement | unqualified | pending |
| foreground × Connect/Apps SDK | authorized scopes and supported app/bridge | unqualified | pending |
| strict background × Connect/Apps SDK | authorized scopes; no OS focus/cursor/clipboard effects | unqualified | pending |
| dedicated headless × Connect/Apps SDK | scopes plus headless inspection capability | unqualified | pending |

Each mode/route needs its own persistence, sync-as-write, conflict, export and blinded review evidence before qualification.
