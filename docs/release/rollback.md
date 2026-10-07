# Local rollback

1. Stop the MCP process by restarting Codex or stopping the client that owns it.
2. Restore the previous host-matched `controlla` binary from the prior release candidate. Never copy a binary built for another OS or CPU architecture.
3. Reload the previously installed unpacked extension version from its saved folder in `chrome://extensions`.
4. Start the MCP client and check `controlla --version` and the extension version before pairing a test tab.
5. Keep the state directory intact. Do not delete journals, artifacts, browser profiles, credentials, or user data as part of rollback.
6. If a job was in flight during restart, inspect its durable status and reconcile any `unknown` delivery before starting a replacement operation.

This procedure has not been exercised across distinct release binaries or on Windows/Linux yet.
