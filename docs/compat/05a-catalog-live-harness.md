# Frozen catalog live harness

## Scope

`crates/host-play/tests/catalog_boundary_live.rs` is the headless compatibility proof for one frozen rs2b0t catalog card at a time. It uses `SharedClientTemplate`, `run_with_template`, the real loopback `Client` callback, `ScriptStartHandle::start_load`, the real `ScenarioRunner`, actual Guardian hold state, and `host::publish_snapshot`/`Pump` observations. It creates no alternate runtime or gameplay-action path.

The live test is ignored by default and requires `LIVE=1`. One invocation owns one ephemeral account, settings store, JS cache, and Play slot. The slot is stopped before the cell returns. No live launch was performed while adding the harness.

## Inputs and command

Required environment:

- `CATALOG_REVISION`: `274` or `289`
- `CATALOG_ROOT`: frozen catalog root
- `CATALOG_COMMIT`: exactly `100adccc037d9f6898080e1cad58fcfc43364775` or `8e7d965be2071d6ec65c3265e12af797082d720a`
- `CATALOG_SCENARIO`: `bone_burier`, `chicken_killer`, `thiever`, `alcher`, or `bank_fletcher`
- `CATALOG_NAV_PACK`: revision-compatible nav pack

Optional environment is `CATALOG_ENGINE_DIR`, `CATALOG_NAV_FLAGS`, and object-valued `CATALOG_SETTINGS_JSON`. The bound profile is loopback-only and uses the revision's existing local port defaults.

```sh
LIVE=1 \
CATALOG_REVISION=274 \
CATALOG_ROOT=.superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775 \
CATALOG_COMMIT=100adccc037d9f6898080e1cad58fcfc43364775 \
CATALOG_SCENARIO=bone_burier \
CATALOG_NAV_PACK=/absolute/path/to/nav.pack \
cargo test -p host-play --test catalog_boundary_live --features memory-profile \
  catalog_boundary_live -- --ignored --exact --nocapture
```

Missing or malformed gates, catalog identity mismatch, card load/compile failure, sibling failure, unavailable revision operation, scenario timeout/failure, script start/runtime error, baseline mismatch, or absent core delta fails the process. Once `LIVE=1` is set, none of these conditions becomes a successful skip.

Schema defaults, scenario injections, and `CATALOG_SETTINGS_JSON` are merged in that order in memory. Explicit keys must exist in the selected card schema. The five core criteria remain mandatory; explicit settings that contradict a core criterion fail rather than weaken the proof.

## Identity ledger

The harness reads `docs/compat/support-matrix.json` from the build checkout and requires one exact catalog and card row. It verifies the registry source and selected card bytes before registration and verifies the registered card source hash afterward. The identity phase records registry/card source hash, merged settings, account name, revision/cache/nav identity including nav SHA-256, compiled JS SHA-256, compiled sibling hashes, and the disabled scenario-only `engine_speed_ms` override. The account password is never printed.

Both catalogs use `src/bot/scripts/index.ts`, SHA-256 `b614aedb3d5adfdfbac3760f082273f3cf5c33fffe4ced0f2b01490ae02d1bc2`.

| Catalog commit | Card | Source path | SHA-256 |
|---|---|---|---|
| `100adccc037d9f6898080e1cad58fcfc43364775` | BoneBurier | `src/bot/scripts/BoneBurier/BoneBurier.ts` | `3ef1a82250d39258972bf5aef243c42c289d32634f8fcee31185d9a06aacf1e7` |
| same | ChickenKiller | `src/bot/scripts/ChickenKiller/ChickenKiller.ts` | `997caf51e5f409703f4523e74e52f9aa22d388059c36e2810358176b26f9087f` |
| same | Thiever | `src/bot/scripts/ThievingBot/ThievingBot.ts` | `5cff25a75db3166a3bf3ab6ca1a8ba37ed77e0e9d48292178f79767316b66ad9` |
| same | Alcher | `src/bot/scripts/Alcher/Alcher.ts` | `1a09baa1117544a2bb80da145829e69c9ced029ad5af533616afa8e7a36f2032` |
| same | BankFletcher | `src/bot/scripts/BankFletcher/BankFletcher.ts` | `194eb82dd3613cf2fea77a5cb04fba1dadbacad78efa6e0eee16c1f4a1c3f3ee` |
| `8e7d965be2071d6ec65c3265e12af797082d720a` | BoneBurier | `src/bot/scripts/BoneBurier/BoneBurier.ts` | `3ef1a82250d39258972bf5aef243c42c289d32634f8fcee31185d9a06aacf1e7` |
| same | ChickenKiller | `src/bot/scripts/ChickenKiller/ChickenKiller.ts` | `997caf51e5f409703f4523e74e52f9aa22d388059c36e2810358176b26f9087f` |
| same | Thiever | `src/bot/scripts/ThievingBot/ThievingBot.ts` | `5cff25a75db3166a3bf3ab6ca1a8ba37ed77e0e9d48292178f79767316b66ad9` |
| same | Alcher | `src/bot/scripts/Alcher/Alcher.ts` | `1a09baa1117544a2bb80da145829e69c9ced029ad5af533616afa8e7a36f2032` |
| same | BankFletcher | `src/bot/scripts/BankFletcher/BankFletcher.ts` | `48a9f13b774931cd2ea5298cf625c480f071713d459ade293c1caccfb383aa2d` |

## Start baselines and core deltas

The harness builds Play with zero accounts, installs the start handle, then spawns one minted local account. It fires the staged production source exactly once when the scenario reaches `StartScript`. That boundary captures a fresh published snapshot and requires attached/in-game scene 2, the minted local player, and case preparation:

- BoneBurier: Lumbridge and five Bones; then Bones decrease, Prayer XP increases, and post-start bury chat appears.
- ChickenKiller: Lumbridge chicken pen; then Strength XP increases, Bones are acquired and later consumed, Prayer XP increases, and post-start bury chat appears.
- Thiever: Ardougne guard stand, ten Lobsters, Thieving/Hitpoints 50; then Thieving XP and Coins increase.
- Alcher: Varrock West bank and Magic 55; then Rune chainbody and Nature rune stocks are acquired and consumed, Magic XP increases, and Coins increase.
- BankFletcher: Varrock West bank, Knife, twenty-seven Willow logs, Fletching 35; then logs decrease, Willow shortbows increase, and Fletching XP increases.

Every comparison uses retained snapshots observed after Start. Seeded state cannot satisfy a core outcome. These cells do not claim complete banking/restock/options coverage or all catalog cards accepted.

## Execution matrix

`NOT RUN` is deliberate: this task was scoped not to launch a live client. Root should run each cell against its intended local server/nav pack. Revision 289 currently fails closed at the unavailable bot-operation gate and is not reported supported until that gate is legitimately available and these cells pass.

| Catalog | Revision | Scenario | Required outcome | Status |
|---|---:|---|---|---|
| `100adccc037d9f6898080e1cad58fcfc43364775` | 274 | bone_burier | bones + Prayer delta | NOT RUN |
| same | 274 | chicken_killer | combat + loot/bury delta | NOT RUN |
| same | 274 | thiever | theft XP + inventory delta | NOT RUN |
| same | 274 | alcher | Magic + fodder/rune + Coins delta | NOT RUN |
| same | 274 | bank_fletcher | Fletching + logs/product delta | NOT RUN |
| same | 289 | bone_burier | bones + Prayer delta | NOT RUN; unavailable operation must fail |
| same | 289 | chicken_killer | combat + loot/bury delta | NOT RUN; unavailable operation must fail |
| same | 289 | thiever | theft XP + inventory delta | NOT RUN; unavailable operation must fail |
| same | 289 | alcher | Magic + fodder/rune + Coins delta | NOT RUN; unavailable operation must fail |
| same | 289 | bank_fletcher | Fletching + logs/product delta | NOT RUN; unavailable operation must fail |
| `8e7d965be2071d6ec65c3265e12af797082d720a` | 274 | bone_burier | bones + Prayer delta | NOT RUN |
| same | 274 | chicken_killer | combat + loot/bury delta | NOT RUN |
| same | 274 | thiever | theft XP + inventory delta | NOT RUN |
| same | 274 | alcher | Magic + fodder/rune + Coins delta | NOT RUN |
| same | 274 | bank_fletcher | Fletching + logs/product delta | NOT RUN |
| same | 289 | bone_burier | bones + Prayer delta | NOT RUN; unavailable operation must fail |
| same | 289 | chicken_killer | combat + loot/bury delta | NOT RUN; unavailable operation must fail |
| same | 289 | thiever | theft XP + inventory delta | NOT RUN; unavailable operation must fail |
| same | 289 | alcher | Magic + fodder/rune + Coins delta | NOT RUN; unavailable operation must fail |
| same | 289 | bank_fletcher | Fletching + logs/product delta | NOT RUN; unavailable operation must fail |

Stdout emits structured `identity`, `baseline-after-preparation`, `start`, and `pass` JSON records followed by one `PASS` line. Preserve full per-cell output under `docs/compat/evidence/catalog-harness/live/`. The implementation checks in `docs/compat/evidence/catalog-harness/verification.json` are not live compatibility evidence.
