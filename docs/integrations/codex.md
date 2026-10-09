# Codex integration

Controlla v3 is a local stdio MCP server. Codex CLI and the Codex IDE extension share MCP configuration, so one registration is enough for both.

Generate a shell-safe registration command:

```sh
node integrations/codex-config.mjs /ABSOLUTE/PATH/TO/controlla /ABSOLUTE/PATH/TO/controlla-state
```

The generated command follows the current Codex CLI stdio contract:

```sh
codex mcp add chrome-controlla --env 'CONTROLLA_STATE_DIR=/ABSOLUTE/PATH/TO/controlla-state/codex' -- '/ABSOLUTE/PATH/TO/controlla' mcp
codex mcp get chrome-controlla --json
```

Codex accepts repeated `--env KEY=VALUE` options before `--`, then treats every argument after `--` as the stdio server command. The dedicated `codex` state directory prevents accidental ownership collisions with Freebuff, OpenCode, Claude Code, and ChatGPT Desktop processes.

After registration, use `codex mcp list --json` or `codex mcp get chrome-controlla --json` to verify startup. Tool discovery alone is not browser qualification. A real acceptance run must also establish the Chrome bridge, explicitly pair the intended tabs, exercise foreground/background/headless behavior, perform guarded typing and a verified mutation, and prove that user/tab drift fails closed.

The v3 default surface intentionally stays at six tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. Prefer `workflow` for multi-step automation so Codex can finish routine research, assignments, form filling, and repetitive work with very few model/tool round trips.

Source contract checked against the current `openai/codex` `mcp add` parser and MCP conformance tests on 2026-10-09. Do not hand-edit unrelated Codex MCP entries when the CLI command is sufficient.
