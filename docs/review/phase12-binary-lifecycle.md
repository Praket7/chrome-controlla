# Phase 12 distinct-binary package lifecycle — 2026-10-07

This local package-manager fixture uses two separately built source revisions. It does not start an MCP server, connect a browser, test an external client, or publish an artifact.

| Version | Source revision | Installed binary SHA-256 |
|---|---|---|
| `0.1.0` (rollback baseline) | `2f4fb06` | `340c476f825ef10bff2362df5a40b9564879fa4eada563197000ed40cf8ecaef` |
| `0.1.1` (upgrade candidate) | `b0a7c7c1634a993523c69577289d20ac2f97ff93` code; packaged from clean tree `5e2914fd68db642c5a1774b5de08e68f4f86cbe0` | `dda6690a9ea5c48dbac76df8952de4f7d5f4682a0d726c9cbfd2a6b1d8dc4dd1` |

Command: `CONTROLLA_PREVIOUS_BINARY=/tmp/chrome-controlla-phase12-previous CONTROLLA_CURRENT_BINARY="$PWD/packages/chrome-controlla/dist/bin/controlla-core" node scripts/check-package-lifecycle.mjs`.

Result: pass on `darwin/arm64`. The harness packed both versions, installed `0.1.0`, upgraded to `0.1.1`, rolled back to `0.1.0`, checked each installed executable against its expected full digest, checked launcher help, uninstalled, and verified an unrelated user-data sentinel survived. The two executable digests differ.

This closes only the local distinct-binary lifecycle check. Cross-platform consumer installs, published registry install, MCP client/server compatibility, signing, and live browser acceptance remain open.
