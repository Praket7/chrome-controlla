# Local MCP client setup

Guide version: `clients-2026-10-06-v1`. Server package: `controlla-runtime` `0.1.0`. Client setup formats checked against upstream documentation/source on 2026-10-06. OpenCode 1.18.5 completed one read-only guide call using a generated v1 config and local Qwen 2.5 7B; this is a narrow tool-call check, not full client acceptance. See `verification-matrix.md` for the bounded evidence and remaining client gates.

This preview runs MCP over local stdio only. Build the executable, then replace `/ABS/PATH/TO/controlla` below with its absolute path (normally `target/release/controlla`); the MCP command is `controlla mcp`. Do not point a client at an HTTP endpoint: this branch has no remote listener or authentication. Each concurrently running client process needs a unique absolute `CONTROLLA_STATE_DIR`; the server locks this directory before journal recovery. To enable permissioned Chrome auto-connect, set `COMPTROL_CHROME_AUTO_CONNECT=1` in the client configuration and enable Chrome Remote Debugging at `chrome://inspect/#remote-debugging`. Chrome may ask for consent. Alternatively, configure an explicitly approved loopback CDP WebSocket using `COMPTROL_ALLOW_DIRECT_CDP=1` and `COMPTROL_CDP_ENDPOINT`; never expose that endpoint outside loopback.

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
        "COMPTROL_CHROME_AUTO_CONNECT": "1",
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
        "COMPTROL_CHROME_AUTO_CONNECT": "1",
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
          "COMPTROL_CHROME_AUTO_CONNECT": "1",
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
claude mcp add --env 'COMPTROL_CHROME_AUTO_CONNECT=1' --env 'CONTROLLA_STATE_DIR=/ABS/PATH/TO/controlla-state/claude' --transport stdio --scope user chrome-controlla -- '/ABS/PATH/TO/controlla' mcp
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
