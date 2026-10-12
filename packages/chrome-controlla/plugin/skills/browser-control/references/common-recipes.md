# Canonical Controlla recipes

Load this reference only when the compact examples in `SKILL.md` are insufficient. These are execution recipes, not a reason to inspect repository source.

## Read or navigate to known content

1. Open the most specific known/qualified destination with `browser`.
2. Take one targeted `snapshot` if a decision is still required.
3. Use one `workflow` for deterministic follow-up actions.
4. Verify only if the mutation receipt does not already establish the requested result.

Avoid homepage detours and duplicate snapshots.

## Newest/latest item

Preferred shape:

`direct destination -> targeted candidate observation -> complete candidate set if required -> choose by publication evidence -> click -> verify`

Use `find_all` and `choose` with `key:"datetime"` when exact publication metadata is available. Do not infer newest from DOM order. If the page exposes only relative ages, report that evidence and do not claim an exact timestamp.

## Known multi-field form

Take one targeted snapshot to obtain fresh controls, then use `fill_form` for up to 16 fields that do not change one another's meaning. Keep a field separate only when filling it changes the next decision.

## Search result navigation

If the destination URL is already user-provided, observed, or returned by qualified route memory, open it directly. Do not open a homepage merely to navigate through menus to the same destination.

## Existing tab task

Reuse the active Controlla context/session and tab. Do not rediscover all tabs or create a new session unless the existing context is invalid.

## Performance anti-patterns

Treat these as bugs on ordinary tasks:

- MCP/server discovery when six tools are already exposed
- `browser describe` before a normal call
- reading `MEMORY.md`, `SKILL.md`, `mcp.rs`, `v3_full.rs`, `observe.rs`, or other source to infer routine arguments
- broad snapshot followed immediately by another broad snapshot
- homepage navigation when a verified specific route exists
- returning deterministic intermediate data to the model only so it can issue the next obvious browser call
- verifying twice when the action receipt already proves the declared postcondition
