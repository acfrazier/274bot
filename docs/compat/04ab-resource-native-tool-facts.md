# Native gather-tool facts and already-prepared Traversal.preload

Task `t_afe0a5a1` implements brief 105 on `codex/rs2b0t-multirevision` after
cake97 review. Isolated checks used exact Git regular blobs of host
`d854321b8012b391866dc619d74f0b60ac380a15` plus the owned overlay. Client
gitlink `9d090ed04957e4efc254f073cda97bc5510ca72b`. 3738 exclusive new files;
no archive extraction, overwrite or deletion. New empty target
`.superpowers/review-exports/resource-native-tool-facts-t_afe0a5a1-target`.
Not LIVE.

## Bridge

Host-owned gather-tool facts live in `api::gather_tools` and are posted on
`__rs2b0t_host.content.gather_tools` through the existing `content_json` seam
after cake's `baker_stall` key. Selected 274/289 generated rows supply
id/name/`wear_position: 3` only. Mining `levelrequire` and Attack opheld2
come from selected content (identical on both revisions for these objects).
Axes have no woodcutting use gate. Bronze wield is the tutorial path, not
Attack-gated. Native black axe **1361** is in the best-first woodcut list.

JS `Tools.js` is thin callback glue over those posted rows. `available` is
exclusive: no inventory fallback. Unknown `canWieldTool` names return false.
`exactTool` / req helpers and the restock/toolkit stubs stay as they were.
`declared_surface.js` is unused and still throws.

`Traversal.preload` is a void no-op. Native `NavWorld` already binds at
template/Play construction. Preload does not start a worker, wait for
readiness, queue a walk, or claim a path. Unbound nav remains fail-closed on
the existing walk arm.

## Proof (isolated empty target)

Export `.superpowers/review-exports/resource-native-tool-facts-t_afe0a5a1` with
`CARGO_TARGET_DIR=.../resource-native-tool-facts-t_afe0a5a1-target`
(`isolated_build=true`). Raw logs: `docs/compat/evidence/resource-native-tool-facts/`.

- Posted AXES are best-first with real ids, including black 1361. Steel-or-better
  index includes mith/addy/rune/black and excludes bronze/iron.
- `bestPickaxe(30, steel)` → `"Steel pickaxe"`; `(5, steel)` → `null`;
  `(1, bronze+steel)` → `"Bronze pickaxe"`. Mining 30 does not consult rune/adamant.
- `bestAxe(1, rune+steel)` → `"Rune axe"` (no WC gate). Bank-only callback does
  not return a held-only name.
- Posted steel pick in inv with `available === false` still returns `null`.
- `canWieldTool("Steel pickaxe"|"Steel axe", 1)` false; `(5)` true. Bronze at
  Attack 0 is true. `"Dragon pickaxe"` is false.
- Deleted `gather_tools` facts: best* return `null`, canWield false.
- `Traversal.preload()` returns undefined, does not throw, queues nothing.

`cargo test --locked --offline -p api gather_tools -- --test-threads=1`: 4 passed.
`cargo test --locked --offline -p script --test resource_tool_facts -- --test-threads=1`:
4 passed. Clippy `-D warnings` on `api --lib` and
`script --test resource_tool_facts` passed.
`rustfmt --edition 2021 --check` on the new Rust files passed.

## Limits

Root owns LIVE Gnome/Coal cells, Seers haul/bank/return, Make-X fletch, and
cache cleanup. This slice unblocks mine/truck/chop first owners only. Ordinary
full Coal haul remains required later. Ancillary death/boat/Bob/knife stay
stubs. No foreign Navigator/Tools table, route, timeout, or runtime change.
`toolRestockPlan` / `bankHasBetterGatherTool` remain `not impl`.
