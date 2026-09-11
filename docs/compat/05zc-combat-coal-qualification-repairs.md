# Combat and Coal Trucks qualification repairs

Status: implementation reviewed; root LIVE requalification pending

Task: `t_76f5e046`

Implementation commit: `6dd6f9102b1111fdcadf551ccfd3a24658d81661`

Client commit: `aef3952d1cd7bb3b93d39c497f0f476b68021c59`

## Scope

This batch repairs the shared combat observer, the combat scenario preconditions exposed by the task-136 live run, and the Coal Trucks full-pack fixture. It does not add missing host capability, change the frozen JavaScript sources, or qualify adjacent policy branches.

The inspected `AutoFighter.ts`, `CoalTrucks.ts` / `CoalTrucksLogic.ts`, `HillGiant.ts`, and `GreenDragon.ts` / `GreenDragonLogic.ts` files have identical SHA-256 values in the `100adccc037d9f6898080e1cad58fcfc43364775` and `8e7d965be2071d6ec65c3265e12af797082d720a` source snapshots. The repair therefore applies one revision-neutral fixture/oracle contract to r274 and r289.

## Findings and repairs

### Combat observation

The old observer treated any target NPC's generic `in_combat` state as local engagement evidence, made zero-health observations sticky without first proving a live selected spawn, and allowed a previously acquired global loot flag to turn an unrelated disappearance into a defeat. Those shortcuts could both false-qualify and deadlock the qualification cycle.

The replacement observer:

- accepts only local selected-target evidence (`local_target_npc` with local combat, or the NPC explicitly targeting the local player);
- counts one engagement per proven life and does not count disappearance/reappearance as another life;
- requires a known-positive selected life followed by a zero-health transition, or disappearance accompanied by a matching ground drop that is fresh relative to both Start and the preceding observation;
- clears the defeated generation only when the same slot returns with positive known health;
- requires a later selected work frame after the verified defeat before qualification;
- retains only per-NPC-slot state and the preceding bounded ground-loot observation.

Focused regression tests cover other-actor combat, stale inventory loot, unknown initial zero health, unsupported disappearance/reappearance, stale versus fresh matching drops, respawn generation reset, and the post-defeat work requirement.

### Combat preconditions

- `HillGiant` now receives Brass key `983` before Start. The host baseline requires it. The key-fetch/entrance branch remains outside this inside-pit qualification cell.
- `GreenDragon` receives Anti-dragon shield `1540`, wears it with the native `Interactions::wear` fixture action on the safe tile, acknowledges removal from inventory, and only then teleports into the hostile field. The host baseline now requires the shield in equipment rather than accepting it merely held. The frozen script's GearEquip branch is not credited.
- `AutoFighter` keeps Adamant scimitar `1331` as a held loadout item. Source inspection showed `ReequipGear` restores only gear captured as equipped at script start; it does not establish a requirement to equip a newly supplied weapon. The qualification requires real selected combat, style XP, defeat, and further work, so no fixture-side weapon equip is needed.

### Coal Trucks

The empty-pack fixture did not exercise the script's inventory-full route. The repaired fixture starts with one Steel pickaxe `1269`, exactly 26 unstackable Knife items `946`, and no Coal `453`, leaving one inventory slot. One mined coal fills the pack, the script must deposit that one coal into the truck rather than a bank, and the cycle must then mine another coal. The host baseline and cycle reject missing, extra, or changed ballast and require the exact mining/deposit/further-mining sequence.

The truck remains the only credited deposit surface; banking is still explicitly rejected by the existing cycle.

## Prior live evidence

The task-136 live runs were diagnostic inputs, not post-repair proof. Their exit-code matrix at host `21446414d21b50088aabf100df4392c3674b89fa` was:

| Case | r274 | r289 |
| --- | ---: | ---: |
| Auto Fighter | 1 | 1 |
| Chaos Druid | 1 | 0 |
| Moss Giant | 0 | 1 |
| Coal Trucks | 1 | 1 |

The run summaries and logs remain under `docs/compat/evidence/catalog-harness/live/` with suffix `100adccc-21446414`.

No LIVE was run by the implementer, as brief136 assigns fresh qualification to root after review. Deterministic tests verify the observer and fixture contracts; they do not prove gameplay. Root will requalify the affected cells before accepting those cycles.

## Verification

Exact committed export: `.superpowers/review-exports/t_76f5e046-r1`

- `cargo test -p scenario`: PASS, 104 tests.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`: PASS, 46 tests; the single `LIVE=1` entrypoint was ignored.
- `cargo clippy -p scenario -p host-play --features host-play/memory-profile --tests -- -D warnings`: PASS.
- `rustfmt --edition 2021 --check crates/scenario/src/lib.rs crates/host-play/tests/catalog_boundary_live.rs`: PASS.

Receipts:

- `docs/compat/evidence/combat-coal-qualification/isolated-checks.log`
- `docs/compat/evidence/combat-coal-qualification/verification.json`

Owned-file SHA-256 values in the export:

- `crates/scenario/src/lib.rs`: `32df2c82653a33bbd54e670569d2276db86b5f6000f27e21c2a9c64a481a94fb`
- `crates/host-play/tests/catalog_boundary_live.rs`: `52f12f8df416a85dcb8d3b34369a5a2b970302de9d4824a21beec6e528644def`

## Qualification boundary

This batch repairs the qualification logic for the named combat-core cycles and Coal Trucks truck-deposit cycle. Actual gameplay remains unqualified until fresh root LIVE evidence passes. It does not claim combat banking, death recovery, Hill Giant key retrieval, Green Dragon escape/banking, or any JavaScript runtime/policy recreation. No client or frozen-source changes were required.

Root verified Grok4.5 / xai-oauth review1372 (12 API calls), approval of code6dd6f9102 and report3818aa662. The initial report incorrectly described LIVE as optional and fixture tests as cycle proof; root corrected those claims above to match the brief and review metadata. Original reviewed report remains in Git history.
