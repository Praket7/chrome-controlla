# Local MCP client setup

Guide version: `clients-2026-10-06-v1`. Server package: `controlla-runtime` `0.1.0`. Client setup formats checked against upstream documentation/source on 2026-10-06. OpenCode 1.18.5 completed one read-only guide call using a generated v1 config and local Qwen 2.5 7B; this is a narrow tool-call check, not full client acceptance. On 2026-10-08, a Freebuff CLI 0.2.19 launch with a temporary HOME stopped at login; its bundled runtime reported 0.2.22, and no MCP discovery occurred. A later Freebuff Desktop 0.0.134 run used a temporary project config and isolated state directory. Desktop status was `connected_writable` with write authorization; one model turn returned that no `chrome-controlla` MCP tools were loaded. Result: 0/2 requested actions (`guide`, `session`); its agent only listed/read project files. No browser tool was called. Freebuff CLI authentication remains unverified. See `verification-matrix.md` for bounded evidence and remaining client gates.

The current agent interface is the V3 six-tool MCP: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. For a source checkout, build both binaries with `cargo build --locked --release -p controlla-runtime --bins`: `target/release/controlla` registers the native host, and `target/release/controlla-v2 mcp` runs the V3 MCP. Do not configure the source `controlla mcp` command when you want the V3 tools; that executable serves the legacy tool contract. The packaged launcher `bin/controlla mcp` routes to V3 and retries a worker that exits during startup up to two times without replaying a client mutation. The MCP publishes `stdio-runtime.json` with its process generation, extension build identity, heartbeat, bridge host ID, and tab count; `controlla doctor --json` reads those fields and probes the live bridge. The optional HTTP server is authenticated and loopback-only. Each concurrently running MCP process needs a unique absolute `CONTROLLA_STATE_DIR`. Shared-tab control uses Chrome's native messaging bridge; it does not use a remote debugging port. See [`quickstart.md`](quickstart.md) for the shortest setup and agent workflow.

Runtime contract notes: `act` accepts `value` for `fill`, `secure_fill`, `type`, `select`, and `slider`; `type`, `select`, and `slider` require it. `verify(text_present)` can read bounded root text, a unique CSS-selected region, a fresh `@cN` semantic reference, or a fresh loader-bound `@f:` frame. Incomplete or truncated evidence cannot prove absence. Existing-session operations infer their mode when omitted, and reject an explicit mode that contradicts the session. Background pointer input to an inactive user-owned tab returns `not_dispatched` without activating it.

## One-time Chrome bridge setup

For a first install, follow [`quickstart.md`](quickstart.md). Install once per Chrome profile; all local MCP clients can use the same registered bridge. Give each concurrently running MCP process its own `CONTROLLA_STATE_DIR`. Each session owns its selected tabs independently: releasing one session leaves other sessions attached, while a tab already leased by another session is rejected with a clear error. Multiple simultaneous sessions require the current extension and native host protocol; an older extension continues to support one session and tells you to reload before enabling concurrency.

1. Build the runtime from the repository root: `cargo build --locked --release -p controlla-runtime`. On macOS/Linux, set `CONTROLLA_BIN="$(pwd)/target/release/controlla"`; on Windows, use the absolute path to `target/release/controlla.exe`.
2. Use Chrome 106 or later. Open `chrome://extensions`, enable Developer mode, and load the checkout's `extensions/chrome-controlla` directory with **Load unpacked**. Approve the `webNavigation` permission when Chrome prompts.
3. Copy the 32-character extension ID shown on its card in `chrome://extensions`, then register the native host using the same built executable that the MCP client will run: `"$CONTROLLA_BIN" install-bridge YOUR_EXTENSION_ID` (substitute the Windows path as needed). This writes the native-host manifest with that executable path and allows only that exact extension origin. The ID is specific to this installation; never reuse an ID from another machine or a documentation example.
4. Reload the extension on `chrome://extensions` so its service worker reconnects to the newly registered host. Configure the MCP client with the checkout's `target/release/controlla-v2` executable and argument `mcp`, then restart/reload that client. The bridge registration above still uses `target/release/controlla`. Keep Chrome and the MCP process running.

For a generic stdio client, the entry is:

```json
{
  "mcpServers": {
    "chrome-controlla": {
      "command": "/ABSOLUTE/PATH/TO/controlla-v2",
      "args": ["mcp"],
      "env": { "CONTROLLA_STATE_DIR": "/ABSOLUTE/PATH/TO/controlla-state" }
    }
  }
}
```

Use the client-specific format below when its config schema differs. Keep the state directory absolute and unique for each running process. A successful MCP tool listing proves only that the client started the server; it does not prove the Chrome extension/native host connection.

### Legacy V2 `session` workflow

The following `session` protocol is for legacy V2 clients only. The current V3 MCP exposes `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`; do not use this section for V3 tasks.

In MCP, call `session` with `action: "discover_shared_tabs"`. Select one or more IDs from `snapshot.tabs` and pass those `target_ids` plus `snapshot.host_id` to `session` with `action: "pair_shared"`; this creates **one session for all selected tabs** over the native bridge. Then call `session` with `action: "accept_shared"` and its returned `session_id`. If you omit `host_id`, pairing uses the manual popup fallback. Discovery includes at most 100 eligible tabs; when `snapshot.truncated` is `true`, later tabs are omitted. Listing does not attach to tabs.

The popup's endpoint/token form is used when `host_id` is omitted, including when the native host is unavailable or busy. Start `pair_shared` without `host_id`, select the same exact IDs in the extension popup, and enter the returned loopback endpoint and one-session token there; then call `accept_shared`. Treat the token as sensitive and do not put it into client configuration or documentation.

Pairing trusts the MCP client that supplies the tab IDs: use it only through a client you trust. Chrome displays its debugger indicator for attached tabs; separate browser-side per-tab approval beyond that indicator has not been established.

After `accept_shared`, use `shared_tab` for direct navigation: `action:"open"` plus `url` creates an active tab and adds it to the same session; `action:"navigate"` also requires the exact `chrome_tab_id`. URLs are bounded HTTP(S) without credentials. The result includes the tab ID, final URL, and fresh document ID. If navigation fails, rediscover and pair the tab again before using it.

Live qualification is narrow: on 2026-10-09, the enabled local MCP entry and loaded extension paired one agent-created local fixture tab over the native route. Guarded fill, typing, fixture slide clicks, mock in-memory post, navigation invalidation/re-pair, bounded readback, release, and tab cleanup succeeded. The file-input attempt was blocked and no file was selected. These checks do not establish remote app changes or persistence. Classroom writes, app acceptance/persistence, and broad client compatibility remain unqualified; see `verification-matrix.md` for details.

Generate the dated config shape for a client with `node integrations/generate-config.mjs <client> /absolute/path/to/controlla-v2 /absolute/path/to/state-root`. Supported IDs are `freebuff`, `opencode-v1`, `opencode-v2`, and `claude`; OpenCode versions have different nesting. Each output gives that client its own state subdirectory. These versioned templates are configuration examples, not proof that an installed client accepts them. ChatGPT Desktop uses the packaged Chrome Controlla local plugin rather than this raw-client config generator; see [`integrations/chatgpt-desktop.md`](integrations/chatgpt-desktop.md).

For a local package build on the current machine, run `scripts/package-build.sh`, then install the staged host package into a private prefix with `npm install --prefix "$HOME/.local/share/chrome-controlla" "$PWD/packages/chrome-controlla/dist"`. The package launcher routes `controlla mcp` to the six-tool v3 MCP while retaining the legacy CLI for setup commands. Configure the MCP command as the installed `bin/controlla` with argument `mcp` and an absolute state path. The same installed package exposes the ChatGPT Desktop plugin at `<prefix>/node_modules/chrome-controlla/plugin`. To enable the temporary permissive local build, set `CONTROLLA_UNRESTRICTED=1` in that MCP server's environment and reload both the unpacked Chrome extension and MCP client; `browser` action `describe` must then report `interaction_policy:"local_unrestricted"`. This is a local, host-bound package workflow; it does not imply a registry release or cross-platform installer.

The `guide` MCP tool returns this document for `topic: "clients"` and `server_version: "0.1.0"`. It rejects other server versions so clients do not silently receive mismatched instructions.

## Freebuff / Codebuff CLI MCP config

The current Codebuff source reads `.agents/mcp.json` (project, parent, then home) with an `mcpServers` object. Add this entry to the file, preserving existing entries:

```json
{
  "mcpServers": {
    "chrome-controlla": {
      "command": "/ABS/PATH/TO/controlla-v2",
      "args": ["mcp"],
      "env": {
        "CONTROLLA_STATE_DIR": "/ABS/PATH/TO/controlla-state/freebuff"
      }
    }
  }
}
```

Restart Freebuff and check that the server tools appear. Tool discovery verifies only that the client loaded the local server; it does not qualify browser access or client behavior.

## OpenCode v1

The v1 config puts named servers directly under `mcp` and uses `enabled`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "chrome-controlla": {
      "type": "local",
      "command": ["/ABS/PATH/TO/controlla-v2", "mcp"],
      "enabled": true,
      "environment": {
        "CONTROLLA_STATE_DIR": "/ABS/PATH/TO/controlla-state/opencode-v1"
      }
    }
  }
}
```

## OpenCode v2

The v2 config nests named servers under `mcp.servers`; servers connect unless `disabled` is true:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "chrome-controlla": {
        "type": "local",
        "command": ["/ABS/PATH/TO/controlla-v2", "mcp"],
        "environment": {
          "CONTROLLA_STATE_DIR": "/ABS/PATH/TO/controlla-state/opencode-v2"
        }
      }
    }
  }
}
```

Use the schema matching the installed OpenCode major version; v1 and v2 config nesting and enablement fields differ. Verify with `opencode mcp list`, then confirm tools load in the client.

## Claude Code

Register a user-scope local stdio process:

```sh
claude mcp add --env 'CONTROLLA_STATE_DIR=/ABS/PATH/TO/controlla-state/claude' --transport stdio --scope user chrome-controlla -- '/ABS/PATH/TO/controlla-v2' mcp
claude mcp get chrome-controlla
```

The second command checks the configured server status. Start Claude Code and review/approve it if prompted. Confirm the V3 `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify` tools are available. This is setup guidance, not a live Claude Code acceptance result.

## ChatGPT Desktop

ChatGPT Desktop uses the packaged Chrome Controlla local plugin rather than a raw client config generated by `integrations/generate-config.mjs`. Build and stage the npm package, install it into a private prefix, then use the local plugin directory at `<prefix>/node_modules/chrome-controlla/plugin`. The plugin launches the package-local compact MCP v2 server over stdio; it does not expose Chrome or CDP on a network port.

Follow [`integrations/chatgpt-desktop.md`](integrations/chatgpt-desktop.md) for the package path, native-bridge setup, explicit tab pairing, compact tools, verification workflow, and acceptance gates. Local plugin availability is machine-local; saving or using the plugin does not make the local process available on web/mobile.

CI verifies the packaged plugin files, package-local v2 binary, MCP startup, and tool discovery. That is not a substitute for a live ChatGPT Desktop acceptance run, which remains separately qualified.

## Check after setup

1. Confirm the V3 `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify` tools are listed.
2. Call `browser` `describe` and `discover_shared_tabs`; pair only the exact tab IDs selected from the fresh discovery result when the active policy requires pairing.
3. Use a fresh snapshot reference before acting, prefer one bounded workflow for deterministic multi-step tasks, and verify the result before reporting success.
4. If setup fails, run `controlla doctor` locally and preserve its report. Do not infer browser reachability from tool listing.

The CLI config schemas were checked against [Freebuff MCP config loading](https://github.com/CodebuffAI/freebuff/blob/main/sdk/src/agents/load-mcp-config.ts), [OpenCode v1 MCP docs](https://thdxr.dev.opencode.ai/docs/mcp-servers/), [OpenCode v2 MCP docs](https://opencode.ai/v2/docs/mcp-servers/), and [Claude Code MCP docs](https://code.claude.com/docs/en/mcp) on 2026-10-06. ChatGPT Desktop's package-local path is documented separately in [`integrations/chatgpt-desktop.md`](integrations/chatgpt-desktop.md) and is validated here only at the plugin/package/startup level until a live Desktop acceptance run is recorded. Run `npm run check:clients` to verify generated legacy config shapes, per-client state isolation, and the local plugin package contract; these checks are not themselves client acceptance.

## Maintained standalone task client

Use registered Controlla tools first, including a scoped Hotload search when available. This fallback is for hosts that do not expose those tools. It is a task helper, not a replacement MCP server entry.

After staging/installing the local package, run `controlla-client`. From a checkout:

```sh
node packages/chrome-controlla/bin/controlla-client.cjs --server "$PWD/target/release/controlla" --server-arg mcp
```

Keep stdin open for the task (use a persistent terminal/PTY when the shell tool otherwise closes stdin). Send one JSON object per line. Startup performs MCP initialization and lists schemas once. Query a selected full schema before calling that tool:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"name":"shared_snapshot"}}
```

Tool calls use `method:"tools/call"`, with `params.name` and `params.arguments` exactly as that schema declares. IDs remain strings where declared. Local validation rejects misspelled fields and wrong types before dispatch. Compact replies prefer structured content; stderr is separate. Schema changes require restarting the helper. Timeouts are never retried automatically; a tool-call timeout is an unknown outcome.

Close stdin after cleanup to end the child process. Interrupts and failed startup also close it, with bounded termination escalation. Use `--state-dir /absolute/path` only for an intentionally separate client. If ownership is busy, use or stop the existing owner; do not evade the conflict with random state directories. Restarting this helper loses in-memory sessions, so reconnect deliberately.
