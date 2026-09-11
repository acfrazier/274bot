# Resource-start preparation

Status: static implementation and focused core verification complete; live execution intentionally not run by this task.

## Provenance

- Host branch/head at verification: `codex/rs2b0t-multirevision` / `ead4d5f5d57f4018b8857880c3daf8c2ddd1990b`.
- Client submodule: `52c37f9ce50d1f184656d5b4469c007ec8a5791a`.
- Frozen source pair checked by the core harness: `100adccc037d9f6898080e1cad58fcfc43364775` and `8e7d965be2071d6ec65c3265e12af797082d720a`.
- Root's one-shot gear send-kind fix `df2ba846a` remains an ancestor. Its GreenDragon shield and AutoFighter mage-staff steps remain `StepKind::Perform`; the scenario runner was not changed.
- `npm exec --yes tsx tools/game-data/verify.ts` passed its source-pin, dirty-input, content/input hash, generated-output, and cache-identity checks for revisions 274 and 289. The checked output digests are `6ca4c04acd7f12f635b99c044009d24d0af09d1d23afb709e8f0e6ca5a634232` (274) and `f7c78bf2ac32b29c618cc5690c555495abd29f101d1e1eba7e382dc4b8f0422b` (289).

## Preparation changes

- HerbloreSecondaries now waits for exact bank booth 2213 at `(3096,3493,0)` with `Use-quickly` before attempting the seed-bank interaction. The existing bounded repeat watch retries only `SceneUnavailable`, `OffScene`, and `StaleTarget`; terminal refusal reasons are retained in the diagnostic. It still acknowledges exact Lobster 379 bank stock and does not seed eggs.
- Gnome chop/short/long modes start at `(2433,3409,0)`, adjacent to the loaded south-bank Magic tree 1306 at `(2432,3410,0)` with `Chop down`. Preparation and the core baseline both require that exact loc/action plus WC 75, Rune axe 1359, 26 Knives 946, mode-specific Fletching, and zero products. The loc anchor is corroborated by `docs/compat/06ab-resource-diagnostic-ownership.md` and the cross-revision cache summary in `docs/compat/05n-resource-world-fixtures.md`.
- ArdyCakes starts with exactly 22 nonproduct Knives, leaving six product slots. The core witness requires the ballast to be banked with the first product deposit before return and further stealing.
- WildyAgility seeds Hitpoints 40; the core baseline requires both base and effective Hitpoints at least 40.
- RockCrab starts at the source reset tile `(2712,3688,0)` and must observe dormant `Rocks` in the supported field before Start. The post-Start witness still requires native activation into `Rock Crab` and the existing combat cycle.
- GreenDragon and AutoFighter mage now acknowledge the exact item in worn equipment, not merely disappearance from inventory, before hostile-field teleport.
- ArdyFighter injects `foodTarget=1` while preserving the existing bank strategy.
- CoalTrucks now requires base/effective Mining 60, Rune pickaxe 1275, 26 Knives, native combat level 55, and zero coal/noted coal before Start. The one-free-slot truck cycle is otherwise unchanged.

## Verification

- `cargo test -p scenario --lib`: PASS, 107/107.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`: PASS, 49/49, one live test ignored as designed.
- `npm exec --yes tsx tools/game-data/verify.ts`: PASS for both pinned revisions.
- Focused negative coverage rejects missing Gnome tree readiness, wrong Ardy ballast/deposit, low Wildy base or effective Hitpoints, low Coal base or effective Mining, Steel-pickaxe substitution, and awake-only RockCrab baselines.

No `LIVE=1` command was run. This is preparation/static evidence for a later fresh run; it does not claim repaired gameplay outcomes.
