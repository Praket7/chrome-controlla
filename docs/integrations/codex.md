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

## Avoiding state-lock failures across Codex tasks

Controlla permits one MCP server process to own a state directory at a time. Codex may start a separate stdio process for each task; two Controlla-enabled tasks using the same `CONTROLLA_STATE_DIR` therefore conflict. The second process exits during MCP initialization with `state directory is already owned by another Chrome Controlla process`, and Codex may show the server as unavailable or omit its tools. `codex mcp list` only confirms registration; it does not confirm that this task's server initialized.

**Prevention:** keep only one active Codex task using this Controlla registration at a time. Finish or close the first task before starting another Controlla task. Ordinary work in other Codex tasks is fine if they do not start this same Controlla server. Do not reload the Chrome extension to fix this lock conflict; the extension is not the lock owner.

**Recovery:** close the duplicate or stale Codex task so its MCP process exits, then start one fresh task and check that the six Controlla tools are available. If the lock error remains, copy the configured state path from `codex mcp get chrome-controlla --json` and inspect its `server.lock` owner with `lsof "/ABSOLUTE/STATE/PATH/server.lock"`. Identify the owning task/process and close that task gracefully. Do not delete `server.lock` or kill a process until you have confirmed it belongs to a stale Controlla task: the OS releases the lock when its owner exits, and removing the file does not safely release an open lock. If the error still reproduces with only one task, verify the configured binary and `CONTROLLA_STATE_DIR`, then restart Codex to create a fresh MCP process. Extension reload is needed only when the extension itself changed.

Separate state directories are only for intentionally isolated concurrent Controlla instances. They do not share journals or coordination state, so they are not a workaround for two tasks that need the same browser session.

After registration, use `codex mcp list --json` or `codex mcp get chrome-controlla --json` to verify startup. Tool discovery alone is not browser qualification. A real acceptance run must also establish the Chrome bridge, explicitly pair the intended tabs, exercise foreground/background/headless behavior, perform guarded typing and a verified mutation, and prove that user/tab drift fails closed.

The v3 default surface intentionally stays at six tools: `browser`, `snapshot`, `act`, `workflow`, `extract`, and `verify`. Prefer `workflow` for multi-step automation so Codex can finish routine research, assignments, form filling, and repetitive work with very few model/tool round trips.

Source contract checked against the current `openai/codex` `mcp add` parser and MCP conformance tests on 2026-10-09. Do not hand-edit unrelated Codex MCP entries when the CLI command is sufficient.
