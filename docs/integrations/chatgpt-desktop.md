# ChatGPT Desktop local plugin

Status: the repository packages a local Agent Plugins 1.0 plugin backed by Controlla's compact MCP v2 stdio server. CI verifies the packaged files, package-local `controlla-v2-core` startup, MCP initialization, and tool discovery. A live ChatGPT Desktop acceptance run is a separate manual qualification gate and must not be inferred from CI alone.

## Architecture

```text
ChatGPT Desktop
  -> Chrome Controlla local plugin
  -> node ${PLUGIN_ROOT}/launch.cjs
  -> package-local bin/controlla-v2-core
  -> Controlla native messaging bridge
  -> explicitly paired Chrome tabs
```

The plugin does not expose CDP or MCP on a network port. MCP stays on local stdio. Chrome authority still comes from Controlla's explicit shared-tab pairing flow, and the compact v2 server preserves stale-reference, document-revision, and unknown-delivery protections.

## Build and install the host package

From the repository root:

```sh
scripts/package-build.sh
npm install --prefix "$HOME/.local/share/chrome-controlla" "$PWD/packages/chrome-controlla/dist"
```

The installed plugin directory is:

```text
$HOME/.local/share/chrome-controlla/node_modules/chrome-controlla/plugin
```

On Windows, use an equivalent private absolute prefix. The staged npm package is host-bound to the OS and CPU that built it, so do not copy a native package between platforms.

## One-time Chrome bridge setup

1. Open `chrome://extensions`, enable Developer mode, and load `extensions/chrome-controlla` with **Load unpacked**.
2. Copy the 32-character extension ID shown by Chrome.
3. Run the installed legacy launcher with `install-bridge` because that command owns native-host registration. For example on macOS/Linux:

   ```sh
   "$HOME/.local/share/chrome-controlla/node_modules/.bin/controlla" install-bridge YOUR_EXTENSION_ID
   ```

4. Reload the extension.
5. Run `controlla doctor` from the same installation and fix any reported bridge or browser problems before using the plugin.

The extension ID is installation-specific. Never paste a documentation placeholder into the native-host manifest.

## Add the plugin to ChatGPT Desktop

Use ChatGPT Desktop's local plugin installation flow and select the installed `chrome-controlla/plugin` directory above. The plugin's portable `mcp.json` starts `node ${PLUGIN_ROOT}/launch.cjs`; that launcher resolves the v2 native binary relative to the installed package rather than relying on a global executable path.

The local process exists only on that desktop machine. Installing or saving the plugin does not make the process available to ChatGPT web or mobile.

## Acceptance flow

After ChatGPT Desktop loads the plugin, verify the following in order:

1. Confirm the compact tools are discoverable, including `browser_session`, `browser_snapshot`, `browser_find`, `browser_act`, `browser_extract`, `browser_workflow`, `browser_probe`, `browser_verify`, and `browser_skill`.
2. Call `browser_session` to discover shared Chrome tabs. Discovery is inventory, not mutation authority.
3. Explicitly select the intended tab IDs and pair only those tabs. Do not silently grant every visible tab.
4. Use `browser_snapshot` or `browser_find` to obtain fresh semantic `@cN` references.
5. Perform a guarded action with `browser_act` or a bounded `browser_workflow`. Supply expected values where the tool supports them.
6. Use the returned fresh state or call `browser_verify` before reporting that a mutation succeeded.
7. Release the session when the task is complete.

A useful first live acceptance task is a local form fixture: discover one fixture tab, pair it, snapshot it, find a text field, fill it without submitting, verify the value, then release the tab. That exercises discovery, explicit authority, semantic references, guarded mutation, and independent fresh-state verification without creating an external side effect.

## Safety rules

- Never treat tab discovery as permission to mutate every tab.
- Never reuse an `@cN` reference after navigation, document replacement, target drift, or another invalidating state change. Take a fresh snapshot.
- If a mutation returns `unknown`, inspect current state before deciding what to do. Never automatically retry an unknown browser outcome.
- Prefer semantic snapshots and deltas. Use `browser_probe` only when semantic state is insufficient; full-page vision is not the default path.
- Keep MCP protocol output on stdout and diagnostics on stderr. The package launcher follows this contract.
- Do not bypass Controlla's selected-tab authority, stale-target checks, or verification by exposing a raw remote-debugging endpoint.

## What CI proves

The repository package check installs the packed npm archive into a clean temporary prefix, launches the packaged plugin entrypoint through the maintained MCP client, initializes the v2 MCP server, and confirms representative compact tools such as `browser_session`, `browser_snapshot`, and `browser_verify` are discoverable on Ubuntu, macOS, and Windows CI.

That establishes package integrity and local MCP startup. It does **not** by itself establish a live ChatGPT Desktop UI connection, human approval UX, or production website compatibility. Those remain explicit acceptance gates rather than inferred claims.
