# Controlled two-slot lifecycle isolation proof

## Scope

`crates/host-play/tests/two_slot_isolation_live.rs` is a standalone ignored LIVE
integration cell for same-revision N=2 isolation through production `Play`.
Two minted private actors share one local-274/local-289 template and start the
same frozen `Alcher` catalog card with distinct supported settings: core
`items=["rune_chainbody"]` versus custom `items=["custom"]` /
`customItem="adamant_scimitar"`. Start uses `ScriptStartHandle::start_load`.
Pause, Resume and Stop go through `Play::{script_pause,script_resume,script_stop}`.
Witnesses require script state plus item/coin/XP progress, not labels alone.

This is component proof preparation, not catalog-card acceptance, not native UI,
and not N32. Root owns actual LIVE launches and frontend acceptance.

No shared scenario, API, script runtime, client, panel, or TUI files were
edited. Catalog harness `catalog_boundary_live.rs` was not edited.

## Contract

The live cell requires `LIVE=1`. Missing `LIVE` skips. Once `LIVE=1` is set,
failure prints `FAIL:` and exits 1. Seed and Start happen only after
`ingame && scene_state == 2`. Off-world relog observations are processed
under production hold; gameplay sends are not issued under hold.

Required environment:

- `TWO_SLOT_REVISION`: `274` or `289`
- `TWO_SLOT_NAV_PACK`: revision-compatible nav pack
- `TWO_SLOT_CATALOG_ROOT`: frozen catalog root
- `TWO_SLOT_CATALOG_COMMIT`: exactly
  `100adccc037d9f6898080e1cad58fcfc43364775` or
  `8e7d965be2071d6ec65c3265e12af797082d720a`

Optional: `TWO_SLOT_ENGINE_DIR`, `TWO_SLOT_NAV_FLAGS`. Ports, cache and nav
identity come from the selected `local-274` / `local-289` loopback profile
(43594/80 or 44594/1080). Vault, JS home and settings use private temporary
storage. Operator `~/.274bot` is not read or written.

Missing required catalog identity fails closed under `LIVE=1`. Product
timeouts are preserved: 180 s preparation, 90 s progress windows, 12 s
in-flight drain before pause-stability assertions.

```sh
LIVE=1 \
TWO_SLOT_REVISION=274 \
TWO_SLOT_CATALOG_ROOT=.superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775 \
TWO_SLOT_CATALOG_COMMIT=100adccc037d9f6898080e1cad58fcfc43364775 \
TWO_SLOT_NAV_PACK=/absolute/path/to/nav.pack \
cargo test -p host-play --test two_slot_isolation_live \
  two_slot_isolation_live -- --ignored --exact --nocapture
```

Both actors must show Magic XP, coin and own-fodder deltas with no peer
item contamination. Pause on slot A must leave A `Paused` and stable after
drain while B continues; Resume must show further A progress; Stop A must
leave A `Idle` while B continues.

## Verification

Export base: host `08235c6460241787e31cbf9eb09f97442bc9f5b0` (host-play lib
identical to `637ee0e3c275a03ee86e78d6c4253b6bfc31c7e0`), client
`56d80272bcbda3eb1e22db096c1c5e21d3497de4`, plus this owned overlay.
Frozen export:
`/Users/acfrazier/experiments/274bot/.worktrees/t_4eb51061-src-20260911041819`
(committed host checkout-index plus client checkout-index plus owned overlay).
Isolated empty target:
`/Users/acfrazier/experiments/274bot/.worktrees/t_4eb51061-target-20260911041819`
(`isolated_build=true`). Shared campaign target was not used.

- `rustfmt --check -- crates/host-play/tests/two_slot_isolation_live.rs crates/host-play/tests/support/two_slot_isolation.rs` — pass.
- `cargo test -p host-play --test two_slot_isolation_live -- --skip two_slot_isolation_live`
  — 6 passed (mixed identities, queued-only success, no-progress while paused,
  paused slot still progressing, cross-slot contamination, ordered
  pause/resume/stop isolation).
- `cargo clippy -p host-play --test two_slot_isolation_live --no-deps -- -D warnings`
  — pass.
- `cargo test -p host-play --test two_slot_isolation_live two_slot_isolation_live`
  — 0 passed, 1 ignored.

No LIVE client or engine was launched. These checks are fixture/build
evidence, not gameplay acceptance and not catalog/frontend acceptance.
