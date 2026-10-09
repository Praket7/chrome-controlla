# Local MCP client setup

Guide version: `clients-2026-10-06-v1`. Server package: `controlla-runtime` `0.1.0`. Client setup formats checked against upstream documentation/source on 2026-10-06. OpenCode 1.18.5 completed one read-only guide call using a generated v1 config and local Qwen 2.5 7B; this is a narrow tool-call check, not full client acceptance. On 2026-10-08, a Freebuff CLI 0.2.19 launch with a temporary HOME stopped at login; its bundled runtime reported 0.2.22, and no MCP discovery occurred. A later Freebuff Desktop 0.0.134 run used a temporary project config and isolated state directory. Desktop status was `connected_writable` with write authorization; one model turn returned that no `chrome-controlla` MCP tools were loaded. Result: 0/2 requested actions (`guide`, `session`); its agent only listed/read project files. No browser tool was called. Freebuff CLI authentication remains unverified. See `verification-matrix.md` for bounded evidence and remaining client gates.

MCP runs over local stdio by default. From a checkout, build the executable with `cargo build --locked --release -p controlla-runtime`; its path is `target/release/controlla` (`target/release/controlla.exe` on Windows). Configure the MCP client to run that absolute path with `mcp` as its argument. The optional HTTP server is authenticated and loopback-only. Each concurrently running MCP process needs a unique absolute `CONTROLLA_STATE_DIR`. Shared-tab control uses Chrome's native messaging bridge; it does not use a remote debugging port.

## One-time Chrome bridge setup

1. Build the runtime from the repository root: `cargo build --locked --release -p controlla-runtime`. On macOS/Linux, set `CONTROLLA_BIN="$(pwd)/target/release/controlla"`; on Windows, use the absolute path to `target/release/controlla.exe`.
2. Use Chrome 106 or later. Open `chrome://extensions`, enable Developer mode, and load the checkout's `extensions/chrome-controlla` directory with **Load unpacked**. Approve the `webNavigation` permission when Chrome prompts.
3. Copy the 32-character extension ID shown on its card in `chrome://extensions`, then register the native host using the same built executable that the MCP client will run: `"$CONTROLLA_BIN" install-bridge YOUR_EXTENSION_ID` (substitute the Windows path as needed). This writes the native-host manifest with that executable path and allows only that exact extension origin. The ID is specific to this installation; never reuse an ID from another machine or a documentation example.
4. Reload the extension on `chrome://extensions` so its service worker reconnects to the newly registered host. Configure the local MCP client to run the same executable with argument `mcp`, then restart/reload that client. Keep Chrome and the MCP process running.

For a generic stdio client, the entry is:

```json
{
  "mcpServers": {
    "chrome-controlla": {
      "command": "/ABSOLUTE/PATH/TO/controlla",
      "args": ["mcp"],
      "env": { "CONTROLLA_STATE_DIR": "/ABSOLUTE/PATH/TO/controlla-state" }
    }
  }
}
```

Use the client-specific format below when its config schema differs. Keep the state directory absolute and unique for each running process. A successful MCP tool listing proves only that the client started the server; it does not prove the Chrome extension/native host connection.

In MCP, call `session` with `action: "discover_shared_tabs"`. Select one or more IDs from `snapshot.tabs` and pass those `target_ids` plus `snapshot.host_id` to `session` with `action: "pair_shared"`; this creates **one session for all selected tabs** over the native bridge. Then call `session` with `action: "accept_shared"` and its returned `session_id`. If you omit `host_id`, pairing uses the manual popup fallback. Discovery includes at most 100 eligible tabs; when `snapshot.truncated` is `true`, later tabs are omitted. Listing does not attach to tabs.

The popup's endpoint/token form is used when `host_id` is omitted, including when the native host is unavailable or busy. Start `pair_shared` without `host_id`, select the same exact IDs in the extension popup, and enter the returned loopback endpoint and one-session token there; then call `accept_shared`. Treat the token as sensitive and do not put it into client configuration or documentation.

Pairing trusts the MCP client that supplies the tab IDs: use it only through a client you trust. Chrome displays its debugger indicator for attached tabs; separate browser-side per-tab approval beyond that indicator has not been established.

Live qualification is narrow: on 2026-10-09, the enabled local MCP entry and loaded extension paired one agent-created local fixture tab over the native route. Guarded fill, typing, fixture slide clicks, mock in-memory post, navigation invalidation/re-pair, bounded readback, release, and tab cleanup succeeded. The file-input attempt was blocked and no file was selected. These checks do not establish remote app changes or persistence. Classroom writes, app acceptance/persistence, and broad client compatibility remain unqualified; see `verification-matrix.md` for details.

Generate the dated config shape for a client with `node integrations/generate-config.mjs <client> /absolute/path/to/controlla /absolute/path/to/state-root`. Supported IDs are `freebuff`, `opencode-v1`, `opencode-v2`, and `claude`; OpenCode versions have different nesting. Each output gives that client its own state subdirectory. These versioned templates are configuration examples, not proof that an installed client accepts them. ChatGPT is intentionally omitted because this preview has neither local-stdio support there nor an authenticated remote endpoint.

For a local package build on the current machine, run `scripts/package-build.sh`, then install the staged host package into a private prefix with `npm install --prefix "$HOME/.local/share/chrome-controlla" "$PWD/packages/chrome-controlla/dist"`. On macOS/Linux, generate a Freebuff config using the installed package with `node integrations/generate-config.mjs freebuff "$HOME/.local/share/chrome-controlla/node_modules/chrome-controlla/bin/controlla-core" "$HOME/.local/share/chrome-controlla/state"`. On Windows, use the installed `controlla-core.exe` path and an absolute state path. This is a local, host-bound package workflow; it does not imply a registry release or cross-platform installer.

The `guide` MCP tool returns this document for `topic: "clients"` and `server_version: "0.1.0"`. It rejects other server versions so clients do not silently receive mismatched instructions.

## Freebuff / Codebuff CLI MCP config

The current Codebuff source reads `.agents/mcp.json` (project, parent, then home) with an `mcpServers` object. Add this entry to the file, preserving existing entries:

```json
{
  "mcpServers": {
    "chrome-controlla": {
      "command": "/ABS/PATH/TO/controlla",
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
      "command": ["/ABS/PATH/TO/controlla", "mcp"],
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
        "command": ["/ABS/PATH/TO/controlla", "mcp"],
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
claude mcp add --env 'CONTROLLA_STATE_DIR=/ABS/PATH/TO/controlla-state/claude' --transport stdio --scope user chrome-controlla -- '/ABS/PATH/TO/controlla' mcp
claude mcp get chrome-controlla
```

The second command checks the configured server status. Start Claude Code and review/approve it if prompted. Confirm the `session`, `observe`, `extract`, and `guide` tools are available. This is setup guidance, not a live Claude Code acceptance result.

## ChatGPT

ChatGPT does not connect directly to a local stdio process. This branch has no authenticated remote MCP endpoint or outbound paired bridge, so it cannot be configured for ChatGPT. Do not expose a local CDP or stdio server to make this work. ChatGPT web requires a separately implemented and authenticated remote route.

## Check after setup

1. Ask the client to call `guide` with `topic: "clients"` and `server_version: "0.1.0"`.
2. Call `session` with `action: "discover"`; then follow its returned next step for a configured provider.
3. Use `session` to select explicit target IDs before calling `observe` or `extract`.
4. If setup fails, run `controlla doctor` locally and preserve its report. Do not infer browser reachability from tool listing.

The CLI config schemas were checked against [Freebuff MCP config loading](https://github.com/CodebuffAI/freebuff/blob/main/sdk/src/agents/load-mcp-config.ts), [OpenCode v1 MCP docs](https://thdxr.dev.opencode.ai/docs/mcp-servers/), [OpenCode v2 MCP docs](https://opencode.ai/v2/docs/mcp-servers/), and [Claude Code MCP docs](https://code.claude.com/docs/en/mcp) on 2026-10-06. [ChatGPT's MCP setup documentation](https://help.openai.com/en/articles/12584461-developer-mode-and-mcp-apps-in-chatgpt) says ChatGPT connects to remote MCP servers, not local stdio servers. Run `npm run check:clients` to verify generated config shapes and per-client state isolation; this local check is not client acceptance.
