# ChatGPT Desktop local plugin

The packaged local plugin starts Controlla's six-tool V3 MCP over stdio. Package tests verify startup and discovery; a live ChatGPT Desktop acceptance run remains separate.

```text
ChatGPT Desktop → plugin/launch.cjs → bin/controlla mcp → native Chrome bridge → shared tabs
```

The six tools are `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. The plugin launcher passes `mcp` to the package-relative launcher. No network MCP endpoint is opened.

## Install

Build the package with `scripts/package-build.sh`, then install `packages/chrome-controlla/dist` into a local prefix. Select its `node_modules/chrome-controlla/plugin` directory in ChatGPT Desktop. The native host must point to the installed `bin/controlla-core`, and the unpacked Chrome extension must be reloaded after updates. The package is built for the host OS and CPU; do not copy native binaries across platforms.

## Live acceptance

After loading the plugin, confirm all six tools are discoverable. Call `browser` action `describe` to read the active interaction policy. For shared Chrome, discover and pair a tab, poll `accept_shared`, then use `snapshot` and `act` on its numeric `chrome_tab_id`. A local fixture can verify fill, readback, and session release without changing an external app.

Guarded mode requires selected-tab authority and fresh references. With `CONTROLLA_UNRESTRICTED=1`, the local process relaxes tab authority and interaction guards; `dispatched` acknowledges input only. Inspect the page outcome before claiming a post or save. Reload ChatGPT Desktop after changing the MCP environment or installed package.

CI package startup does not establish a live ChatGPT Desktop connection, real-site compatibility, or competitive speed. Record those separately.
