#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

node extensions/chrome-controlla/test-background.cjs
# This fixture covers OOPIF AX/click routing, frame-bound chooser events and file setting,
# same-key tab-open retries, and inactive-tab/window presentation telemetry.
cargo test -p controlla-runtime --test guard_diagnostics --locked --offline -- --ignored
cargo test -p controlla-runtime --lib --locked --offline
cargo test -p controlla-runtime --bin controlla-v2 --locked --offline
cargo test -p controlla-runtime --bin controlla-v2 v3_rich_editor_block_fast_human_and_ime_are_verified --locked --offline -- --ignored
CONTROLLA_UNRESTRICTED=1 cargo test -p controlla-runtime --bin controlla-v2 v3_rich_editor_block_fast_human_and_ime_are_verified --locked --offline -- --ignored
cargo test -p controlla-runtime --bin controlla-v2 v3_hidden_file_upload_waits_for_delayed_application_acceptance --locked --offline -- --ignored
cargo test -p controlla-runtime --bin controlla-v2 v3_headless_launch_list_snapshot_fill_verify_and_release --locked --offline -- --ignored
cargo test -p controlla-runtime --bin controlla-v2 shared_extension_marked_control_defaults_to_verified_trusted_fast_keys --locked --offline -- --ignored
# The unrestricted fixture uses synthetic tab IDs. Keep the native host's real
# per-user tab inventory out of the fixture so it cannot pair against user Chrome.
fixture_home="$(mktemp -d)"
fixture_cargo_home="${CARGO_HOME:-$HOME/.cargo}"
fixture_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
trap 'python3 -c '\''import shutil,sys; shutil.rmtree(sys.argv[1],ignore_errors=True)'\'' "$fixture_home"' EXIT
HOME="$fixture_home" CARGO_HOME="$fixture_cargo_home" RUSTUP_HOME="$fixture_rustup_home" CONTROLLA_UNRESTRICTED=1 \
  cargo test -p controlla-runtime --bin controlla-v2 shared_extension_marked_control_defaults_to_verified_trusted_fast_keys --locked --offline -- --ignored
cargo test -p controlla-runtime --test v2_stdio compact_v3_advertises_task_route_and_guarded_tools --locked --offline
cargo test -p controlla-browser --locked --offline
cargo test -p controlla-runtime --test espn_latest_story --locked --offline
# Exercise semantic identity across document/frame changes and the deterministic late-DOM story path.
cargo test -p controlla-runtime --test v3_core --locked --offline
cargo test -p controlla-runtime --test reference_reacquire --locked --offline
cargo test -p controlla-runtime --test v3_tasks --locked --offline
