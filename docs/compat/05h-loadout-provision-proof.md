# Observed loadout provisioning proof

## Scope

`crates/host-play/tests/loadout_provision_live.rs` is a standalone ignored LIVE
integration cell for the existing loadout component contract. It uses production
`Play`, `ScriptStartHandle::start_load`, the selected immutable local
profile/template, and `host::publish_snapshot` observations. The script is
test-authored CompatClass JS in
`crates/host-play/tests/fixtures/loadout_provision/provision.js`. It is not a
catalog card, not a frontend claim, and not a queued-success substitute.

No shared scenario, API, script runtime, client, panel, or TUI files were
edited. Native editor and reviewed loadout source `89767527` remain the
qualified store/accessor work. Root owns actual LIVE launches.

## Contract

The live cell requires `LIVE=1`. Missing `LIVE` skips. Once `LIVE=1` is set,
failure prints `FAIL:` and exits 1. Start happens only after
`ingame && scene_state == 2`.

Required environment:

- `LOADOUT_REVISION`: `274` or `289`
- `LOADOUT_NAV_PACK`: revision-compatible nav pack

Optional: `LOADOUT_ENGINE_DIR`, `LOADOUT_NAV_FLAGS`. Ports, cache and nav
identity come from the selected `local-274` / `local-289` loopback profile
(43594/80 or 44594/1080). Vault, JS home and loadouts use private temporary
storage. Operator `~/.274bot/loadouts.json` is not read or written.

```sh
LIVE=1 \
LOADOUT_REVISION=274 \
LOADOUT_NAV_PACK=/absolute/path/to/nav.pack \
cargo test -p host-play --test loadout_provision_live \
  loadout_provision_live -- --ignored --exact --nocapture
```

Preparation before Start is only selected-data valid Attack 40, `givebank`
Rune scimitar / 40 Lobsters, Varrock West stand `(3185,3440,0)`, and an
acknowledged open-bank snapshot. Baseline must lack those items in inventory
and equipment. After Start the production script waits for `Bank.ready()`,
withdraws quantity 17 and the weapon through `Bank.withdrawX`, then
`Equipment.equip`. The witness requires a later fresh bank generation,
exact inventory 17, bank decrement, equipped weapon id 1333 in wearpos 3
(`righthand`), and preserved supplies. Empty/stale bank, queued request, or
seeded equipment cannot pass.

## Verification

Export base: host `c19e3170efe62bd38a39ea64f7ce62bef63780c8`, client
`56d80272bcbda3eb1e22db096c1c5e21d3497de4`, plus this owned overlay.
Frozen export:
`/Users/acfrazier/experiments/274bot/.worktrees/t_3d333b56-src-20260910234748`
(git archive of that host + client archive + owned overlay). Isolated empty
target:
`/Users/acfrazier/experiments/274bot/.worktrees/t_3d333b56-target-20260910234748`
(`isolated_build=true`). Shared campaign target was not used.

- `cargo fmt --check -- crates/host-play/tests/loadout_provision_live.rs` — pass.
- `cargo test -p host-play --test loadout_provision_live -- --skip loadout_provision_live`
  — 5 passed (seeded equipment, stale/empty bank, queued request, ordered
  withdraw/decrement/righthand, missing decrement).
- `cargo clippy -p host-play --test loadout_provision_live --no-deps -- -D warnings`
  — pass.
- `cargo test -p host-play --test loadout_provision_live loadout_provision_live`
  — 0 passed, 1 ignored.

No LIVE client or engine was launched. These checks are fixture/build
evidence, not gameplay acceptance and not catalog/frontend acceptance.

## Root isolated LIVE follow-up

Actual Grok 4.5 review 1218 approved 064de2cf, with independent empty-target
checks. Root froze that exact host/client source (2,172 files) and built in a
new empty target. Both revisions failed before Start at WaitLoggedOut: the
fixture returned on production hold, which is true while disconnected, and
therefore never observed the off-world transition. Both failure receipts and
logs remain under evidence/loadout-provision/r{274,289}-064de2cf/. No provision
or card/frontend acceptance is claimed.

The bounded fixture correction lets WaitLoggedOut/WaitRelog observe during
hold; those states issue no game actions. Other preparation and gameplay gates
are unchanged. Root will repeat the two cells on a new isolated candidate.
