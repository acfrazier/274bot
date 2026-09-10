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

- BoneBurier: five carried Bones and 28 banked before Start; first-batch burial/depletion, fresh bank open and stock transfer, bank close and another inventory/Prayer burial delta.
- ChickenKiller: Lumbridge chicken pen; then Strength XP increases, Bones are acquired and later consumed, Prayer XP increases, and post-start bury chat appears.
- Thiever: Ardougne guard stand, ten Lobsters, Thieving/Hitpoints 50; then Thieving XP and Coins increase.
- Alcher: Varrock West bank and Magic 55; then Rune chainbody and Nature rune stocks are acquired and consumed, Magic XP increases, and Coins increase.
- BankFletcher: Varrock West bank, Knife, twenty-seven Willow logs, Fletching 35; then logs decrease, Willow shortbows increase, and Fletching XP increases.

Every comparison uses retained snapshots observed after Start. Seeded state cannot satisfy a core outcome. These cells do not claim complete banking/restock/options coverage or all catalog cards accepted.

## Execution matrix

`NOT RUN` is deliberate: this task was scoped not to launch a live client. Root should run each cell against its intended local server/nav pack. Revision 289 operations were enabled at 87084cbc after the scoped host/world proofs. Each catalog cell still requires its own observed functional result.

| Catalog | Revision | Scenario | Required outcome | Status |
|---|---:|---|---|---|
| `100adccc037d9f6898080e1cad58fcfc43364775` | 274 | bone_burier | deplete + bank restock + second burial | NOT RUN |
| same | 274 | chicken_killer | combat + loot/bury delta | NOT RUN |
| same | 274 | thiever | theft XP + inventory delta | NOT RUN |
| same | 274 | alcher | Magic + fodder/rune + Coins delta | NOT RUN |
| same | 274 | bank_fletcher | Fletching + logs/product delta | NOT RUN |
| same | 289 | bone_burier | deplete + bank restock + second burial | NOT RUN |
| same | 289 | chicken_killer | combat + loot/bury delta | NOT RUN |
| same | 289 | thiever | theft XP + inventory delta | NOT RUN |
| same | 289 | alcher | Magic + fodder/rune + Coins delta | NOT RUN |
| same | 289 | bank_fletcher | Fletching + logs/product delta | NOT RUN |
| `8e7d965be2071d6ec65c3265e12af797082d720a` | 274 | bone_burier | deplete + bank restock + second burial | NOT RUN |
| same | 274 | chicken_killer | combat + loot/bury delta | NOT RUN |
| same | 274 | thiever | theft XP + inventory delta | NOT RUN |
| same | 274 | alcher | Magic + fodder/rune + Coins delta | NOT RUN |
| same | 274 | bank_fletcher | Fletching + logs/product delta | NOT RUN |
| same | 289 | bone_burier | deplete + bank restock + second burial | NOT RUN |
| same | 289 | chicken_killer | combat + loot/bury delta | NOT RUN |
| same | 289 | thiever | theft XP + inventory delta | NOT RUN |
| same | 289 | alcher | Magic + fodder/rune + Coins delta | NOT RUN |
| same | 289 | bank_fletcher | Fletching + logs/product delta | NOT RUN |

Stdout emits structured `identity`, `baseline-after-preparation`, `start`, and `pass` JSON records followed by one `PASS` line. Preserve full per-cell output under `docs/compat/evidence/catalog-harness/live/`. The implementation checks in `docs/compat/evidence/catalog-harness/verification.json` are not live compatibility evidence.


## Headed BoneBurier finding and corrected proof (21:43 UTC)

The operator requested headed runs. Root launched the actual panel through its
small catalog_watch entry, which uses the existing IsolatedEnv store scope
and production run_panel. Frozen host e7915812/client 56d8027, catalog 100adccc,
Mac local 289; exact binary/source hashes and commands are under
`evidence/catalog-headed/`.

The process ran from 21:24:16 to 21:27:55 UTC and exited 0. Its original
scenario printed PASS after one burial. The actual window subsequently showed
five bones buried, Prayer XP +22, inventory zero, Trips 0 and repeated
`could not open a bank` at (3220,3220,0). Root captured and read
`r289-bone-burier-100adccc-e7915812.png`. This is a failed complete BoneBurier
loop, preserved with its original log/exit result. It qualifies no ledger row.

Frozen Banking.ts includes a nearest-bank travel fallback. The host shim only
tries a scene booth, and Bank.withdraw queues without returning the boolean
its caller checks. Brief 31 addresses those planned capabilities in Rust.
The user asked root to explain the source evidence before changing scope;
nearest-bank behavior remains in the plan.

Root strengthened the shared scenario: seed five carried and 28 banked Bones
before the relog/Start, observe first-batch XP and depletion, fresh open bank
stock, 28 withdrawn with bank stock zero, bank close, and additional burial XP.
The independent catalog witness also requires the ordered fresh bank transfer
and post-withdrawal inventory/Prayer change. Existing 180-second deadline and
watch bounds remain. Scenario tests: 79 pass. Independent catalog tests: eight
pass, including rejection of first-burial-only, depleted-only, stale-bank,
missing-transfer, no-consumption and no-new-XP observations. Exact source
manifest/checks and the corrected field-access compile failure are retained.
The corrected live loop has not run yet.


The first headed ChickenKiller diagnostic used the same e7915812 binary from
21:42:09 to 21:45:48 UTC. The first capture shows a style announcement error;
the second shows three kills, three burials and 15 feathers. Root read both
captures. The core loop continued, but the original XP-only PASS is not an
error-free result. `Game.combatStyleResolution` returned mode/label while
`describeCombatStyle` required requested. Root adds the requested/effective
fields for the existing exact-match path and validates the two actual loaded
API calls together. This mapping defect predates the optimization merge:
the exact mismatched source exists at 217f150f's parent; source receipt is
`combat-style-prior-source.json`. It was introduced in 0ca3742f on September 3.
This correction does not claim completion of all weapon/style fallback options.
The source-contract check passes after correcting its initial empty-snapshot
fixture. A fresh headed run is next.
