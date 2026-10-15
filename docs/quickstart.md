# Chrome Controlla: installation and agent guide

This guide installs the current V3 six-tool MCP from a local checkout and connects it to one Chrome profile. The bridge is registered once per macOS user; each MCP process gets its own state directory.

## Install on macOS

Run these steps from the Controlla checkout you intend to use. Do not load a parent `extensions` directory or another Controlla checkout.

1. Build the CLI/native host and V3 MCP:

   ```sh
   cargo build --locked --release -p controlla-runtime --bins
   ```

2. In the Chrome profile you want to control, open `chrome://extensions`, enable **Developer mode**, choose **Load unpacked**, and select `<checkout>/extensions/chrome-controlla`. This is the folder that directly contains `manifest.json`. Copy the extension ID shown on that extension's card.

3. Register that exact ID with the CLI built from this same checkout:

   ```sh
   "$(pwd)/target/release/controlla" install-bridge YOUR_EXTENSION_ID
   ```

   Check Chrome's native-host manifest at `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/chrome_controlla_bridge.json`. Its `allowed_origins` must equal `chrome-extension://YOUR_EXTENSION_ID/`, and its `path` must point to this checkout's `target/release/controlla`. Re-run `install-bridge` whenever the extension ID or checkout changes. This exact-ID match is required for Chrome to start the native host.

4. Reload the unpacked extension in `chrome://extensions` so it reconnects to the registered host.

5. Generate the Codex MCP command using the **V3** executable, then run the printed commands:

   ```sh
   node integrations/codex-config.mjs "$(pwd)/target/release/controlla-v2" "$HOME/.chrome-controlla"
   ```

   Restart Codex. The registered MCP must expose exactly these V3 tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. The `controlla` executable is used for `install-bridge`; `controlla-v2` serves this V3 tool surface.

## Instructions for AI agents

Use the [browser-control skill](../packages/chrome-controlla/plugin/skills/browser-control/SKILL.md) as the full tool contract. If the client cannot load skills, follow these rules:

1. Use the six V3 schemas already supplied by the MCP. Do not call `describe`, list schemas, or perform discovery for an ordinary task. After a reload, use the newly supplied schema directly; diagnose only after an actual tool error. If the available tools are legacy names such as `session` or `observe`, configure the V3 executable above.
2. For an exact existing tab, use its already-bound `session_id` and `chrome_tab_id`; do not call `open`. `browser action=open` always creates a new tab. Use `mode:"background"` with `target_policy:"create_background_window"` only when a separate unfocused automation window is explicitly requested. For each new target, supply a unique `idempotency_key` and reuse it only to recover that same open.
3. Take one `snapshot` scoped to the decision. Use a single bounded `workflow` for known multi-step tasks instead of repeating snapshot/find/action calls. Choose a target from fresh evidence and keep its returned reference; do not infer a target from tab order or page position.
4. Use `extract` for bounded structured page data and `verify` for an independent postcondition. Report success only when the returned evidence proves it. After an `unknown` or `dispatched` mutation, inspect or verify the result; never replay it blindly.
5. For “newest” or “latest,” gather the complete relevant candidate set and rank by observed published time. If dates are missing or coverage is incomplete, report that uncertainty instead of guessing.
6. Release the Controlla session when finished. Release does not close the user's tabs. For concurrent MCP processes, assign each process a different absolute `CONTROLLA_STATE_DIR`; do not make a second process reuse a directory that another process owns.

### Efficient task pattern

- **New page:** one `browser` foreground `open` call, with a bounded `workflow` in the same call when the next steps are already clear.
- **Existing page:** in guarded mode, discover tabs, select exact decimal tab IDs, pair once, accept once, then snapshot and run one workflow. In local-unrestricted mode, follow `describe`'s bootstrap route.
- **Simple known change:** use one `act` with a fresh snapshot reference, then check its verified receipt. Do not take another full snapshot when the receipt already proves the requested result.
- **Upload:** register only the requested file in the active session, use an observed file input/trigger and the required acceptance predicate, and continue only after `application_accepted:true`. That proves app acceptance, not persistence; verify persistence separately when the task requires it.

`act` accepts `value` for `fill`, `secure_fill`, `type`, `select`, and `slider`; the last three require a value. `verify(text_present)` supports root text, a unique CSS selector, a fresh `@cN` reference, or a fresh loader-bound `@f:` frame reference. Omit `mode` when continuing or releasing a known browser session; Controlla uses the session's mode and rejects an explicit conflict. Background pointer actions on inactive user tabs are refused without activation.

## Troubleshooting

- **“Native tab inventory is stale”:** check that the extension is enabled, the host manifest's `allowed_origins` exactly matches the ID Chrome displays, and its executable path exists. Reload the extension after correcting registration. A successful MCP startup alone does not prove the Chrome bridge is connected.
- **No tools or the wrong tool names:** confirm Codex launches this checkout's `target/release/controlla-v2 mcp`, then restart Codex and inspect the live tool list.
- **A second MCP process cannot start:** give it a different absolute `CONTROLLA_STATE_DIR`.
- **Pairing rejects a tab:** rediscover and pass the fresh snapshot's `host_id` and only the exact selected tab IDs. Do not retry with an older snapshot.

For Windows, use the corresponding `.exe` binaries and the native-host registration path documented in [`clients.md`](clients.md). The agent tool contract is the same; the installed extension ID must still match the native-host manifest.
