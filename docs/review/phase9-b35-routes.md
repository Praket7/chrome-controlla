# Phase 9 B35 route evaluation

## Method and environment

Fresh blind evaluation against the current `docs/MASTER_GUIDE.md` (version `master-2026-10-06-v3`) and rebuilt packaged MCP executable `packages/chrome-controlla/dist/bin/controlla-core` (server version `0.1.0`). The server was launched with a fresh temporary `CONTROLLA_STATE_DIR`; I sent JSONL `initialize` using protocol version `2025-11-25`, then `notifications/initialized` and `tools/list`. The initialize response listed the registered tools, including `session`, `observe`, `shared_observe`, `shared_input`, and `file_select`. The catalog schema for `file_select.locator` is a typed `FileSelectLocator` union of explicit locator objects, each with `additionalProperties:false`.

No browser or app connection was made. No source or other documentation was inspected. The exact original scenario prompts were not retained; the descriptions below are the five scenarios evaluated, not verbatim prompts.

## Results

All five outcomes remain unchanged from the prior run: **PASS**. The v3 guide and rebuilt package retain the route, target, ownership, and unsupported-capability guidance used in this assessment.

### 1. Local stdio configuration and discovery — PASS

**Scenario:** Configure the local stdio server and discover its available tools.

**Evidence:** `MASTER_GUIDE.md` §2 shows a client config with an absolute `controlla` command path, `mcp` argument, `CONTROLLA_STATE_DIR`, and `COMPTROL_CHROME_AUTO_CONNECT`. It says to use `session` action `discover`. In the rebuilt packaged MCP `tools/list` result, `session` is present and its schema requires `action` as a string.

**Gap:** The `action` schema is an unconstrained string; the operator must follow the guide's supported-action list.

### 2. Direct CDP versus selected extension mode — PASS

**Scenario:** Choose between explicitly configured Direct CDP and pairing already-open extension tabs.

**Evidence:** `MASTER_GUIDE.md` §§2–3 describe Direct CDP as an explicitly configured loopback endpoint or permissioned auto-connect, and the extension route as pairing selected decimal tab IDs. The guide says Direct CDP does not enable the shared-extension provider and warns against silently switching routes or tabs. The rebuilt packaged catalog exposes `session`, `shared_observe`, and `shared_input` as separate tools.

**Gap:** No live browser connection or route behavior was tested.

### 3. List and observe exactly one target — PASS

**Scenario:** Select one explicit target, list it, then make a bounded observation of that target.

**Evidence:** `MASTER_GUIDE.md` §2 gives the Direct CDP sequence: `targets` with `provider:"explicit_cdp"`, `connect` with exact IDs returned by `targets`, then `list_targets` with the returned session ID. §4 requires the complete `target_ref` returned by `list_targets`. In rebuilt packaged `tools/list`, `observe` requires `session_id`, `target_ref`, and a bounded `spec`.

**Gap:** The tool catalog was inspected, but no browser target was connected or observed.

### 4. Preserve and clean up a borrowed tab — PASS

**Scenario:** Avoid closing a user-owned or borrowed tab and determine the supported cleanup action.

**Evidence:** `MASTER_GUIDE.md` §4 says there is no tab-close or generic session-release tool; `release_shared` detaches this server's debugger attachments and does not close the Chrome tab. It says stopping the MCP process ends Direct CDP while Chrome tabs/windows remain open, and explicitly instructs never to close borrowed tabs and to preserve tabs the user takes over or wants to keep.

**Gap:** The guide does not show a step-by-step Direct CDP cleanup example; it identifies stopping the MCP process as the way to end Direct CDP.

### 5. Recover from unavailable capability — PASS

**Scenario:** Handle an unsupported operation or unavailable route without silently switching modes or bypassing a denial.

**Evidence:** `MASTER_GUIDE.md` §7 says unsupported input modes should be reported instead of guessed. §11 says not to use raw CDP to bypass an origin or operation denial. §14's `unsupported` recovery row says to state the exact operation/mode limitation and offer a supported scoped route. §2 also says not to switch modes silently.

**Gap:** No live denial or unavailable-capability response was induced; this is a guide-based recovery evaluation.

## Limits

These outcomes assess whether the guide and rebuilt packaged tool catalog give a novice enough direction to proceed safely or refuse. They do not establish client installation, browser behavior, app behavior, real-client acceptance, or live cleanup/recovery behavior. Tool schemas were inspected via the rebuilt packaged local MCP `tools/list`; no tool calls to a browser or app were made.
