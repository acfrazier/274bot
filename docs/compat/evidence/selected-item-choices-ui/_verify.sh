#!/bin/sh
set -eu
ROOT="/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision"
EXPORT="$ROOT/.check-t_f8ca5ed7-r1289-src"
TARGET="$ROOT/target-t_f8ca5ed7-r1289-iso"
EVIDENCE="$ROOT/docs/compat/evidence/selected-item-choices-ui"
HOST_HEAD="$(git -C "$ROOT" rev-parse HEAD)"
CLIENT_HEAD="$(git -C "$ROOT/vendor/fr-client-rust" rev-parse HEAD)"

rm -rf "$EXPORT"
mkdir -p "$EXPORT"
git -C "$ROOT" archive HEAD | tar -C "$EXPORT" -xf -
mkdir -p "$EXPORT/vendor/fr-client-rust"
git -C "$ROOT/vendor/fr-client-rust" archive HEAD | tar -C "$EXPORT/vendor/fr-client-rust" -xf -

OVERLAY="
crates/script/src/rs2b0t_registry.rs
crates/script/src/loadouts_store.rs
crates/script/src/lib.rs
crates/script/src/settings_store.rs
crates/script/tests/rs2b0t_registry.rs
crates/script/tests/loadouts_bag.rs
crates/script/tests/settings_bag.rs
crates/tui/src/script_params.rs
crates/tui/src/app.rs
crates/tui/src/bin.rs
crates/panel/src/app.rs
"
for f in $OVERLAY; do
  mkdir -p "$EXPORT/$(dirname "$f")"
  cp "$ROOT/$f" "$EXPORT/$f"
done

rm -rf "$TARGET"
mkdir -p "$TARGET"
cd "$EXPORT"
export CARGO_TARGET_DIR="$TARGET"

{
  echo "task t_f8ca5ed7"
  echo "retry_label r1289-after-disk-full"
  echo "date $(date -u '+%Y-%m-%d %H:%M UTC')"
  echo "branch $(git -C "$ROOT" branch --show-current)"
  echo "export_host_commit $HOST_HEAD"
  echo "client_commit $CLIENT_HEAD"
  echo "isolated_export $EXPORT"
  echo "isolated_target $TARGET"
  echo "isolated_build true"
  echo "target_empty_before_compile true"
  echo "no_campaign_target"
  echo "no_LIVE"
  echo
} > "$EVIDENCE/isolated-checks.txt"

run() {
  echo "\$ $*"
  "$@"
}

run cargo test -p script --locked --test rs2b0t_registry
run cargo test -p script --locked --lib resolve_item_options
run cargo test -p script --locked --lib settings_store
run cargo test -p script --locked --lib resolve_setting_options_lists_loadout
run cargo test -p script --locked --test loadouts_bag loadout_combo
run cargo clippy -p script --locked --no-deps --lib --tests -- -D warnings
run cargo test -p tui --locked --lib script_params
run cargo clippy -p tui --locked --no-deps --lib --tests -- -D warnings
run cargo test -p panel --locked --lib loadout_combo
run cargo clippy -p panel --locked --no-deps --lib --tests -- -D warnings
run cargo clippy -p tui --locked --no-deps --bins -- -W clippy::type_complexity
