# Blockers and external dependencies

## Phase 0

No Phase 0 blocker. The local Rust installation was discovered at `~/.cargo/bin`; it was not initially on the login shell PATH. Build evidence below invokes Cargo by its absolute path.

## Later qualification

Client live qualification, paid app entitlements, test accounts, and platform-specific interference observation have not been attempted. These are unverified future gates, not Phase 0 passes or confirmed blockers. Record an exact missing prerequisite only when its phase is reached.

## Phase 1 boundary

- No daemon, bridge, or browser dispatch handler is shipped. Doctor can probe a configured local PID and loopback authenticated health service; only mock-service fixtures are available here, so no live service/transport result is claimed. Windows process liveness remains explicitly unknown.
- The target-bound npm archive/install check is local macOS arm64 evidence. CI is configured to rebuild and install the host-bound archive on each runner, but those updated macOS/Linux/Windows jobs have not yet run; no release/publication is implied. Every OS/CPU release cell needs its own host build.
- Registry provider availability is fixture-backed. Catalog and dispatch share registry-owned revisions and policy reevaluation, but these fixtures do not establish a running direct CDP connection or extension bridge.
