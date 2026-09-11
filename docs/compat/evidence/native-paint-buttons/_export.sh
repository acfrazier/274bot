#!/bin/sh
set -eu
SRC="/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision"
EXPORT="$SRC/.superpowers/review-exports/native-paint-buttons-t_9b6fbcdf"
rm -rf "$EXPORT"
mkdir -p "$EXPORT"
git -C "$SRC" archive HEAD | tar -x -C "$EXPORT"
rm -rf "$EXPORT/vendor/fr-client-rust"
mkdir -p "$EXPORT/vendor/fr-client-rust"
git -C "$SRC/vendor/fr-client-rust" archive HEAD | tar -x -C "$EXPORT/vendor/fr-client-rust"
for f in \
  crates/script/src/shim/paint.js \
  crates/script/src/shim/mod.rs \
  crates/script/schema/isolate.fbs \
  crates/script/src/isolate_fb.rs \
  crates/script/src/load.rs \
  crates/script/src/slot.rs \
  crates/script/tests/paint_buttons.rs \
  crates/script/tests/load_isolate.rs \
  crates/host-play/src/lib.rs \
  crates/panel/src/paint.rs \
  crates/panel/src/session.rs \
  crates/panel/src/app.rs \
  crates/tui/src/chat.rs \
  crates/tui/src/app.rs \
  crates/tui/src/bin.rs \
  docs/compat/04af-native-paint-buttons.md \
  docs/compat/evidence/native-paint-buttons/README.md
do
  mkdir -p "$EXPORT/$(dirname "$f")"
  cp "$SRC/$f" "$EXPORT/$f"
done
printf '%s\n' "$EXPORT"
