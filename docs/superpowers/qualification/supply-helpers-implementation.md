# Supply helpers implementation (card t_daf02506)

## Identity

| Field | Value |
| --- | --- |
| Model | Composer 2.5 Fast |
| Session | `176b82be-fd82-474c-9eaf-6030c71060fc` |
| Checkout | `/Users/acfrazier/experiments/274bot/.worktrees/r018-supply-helpers` |
| Branch | `codex/r018-supply-helpers` |
| Base | `2c1d58e5a35678a0432cb19f421e70b0a0a565a6` |

## Summary

Implemented five sync NativeApi `HelperResult` methods (`foodCount`, `foodHealAmount`, `combatKeepNames`, `runesPerCast`, `escapeRunesFor`) plus Rust fact modules reusable for W5. v1 `foodCount` / `foodHealAmount` marshal through typed V8 globals (`globalThis.__rs2b0t_food_*`, matching buyout_plan). v2 dispatch uses `__rs2b0t_supply_v2` JSON callback (same pattern as prayer). Existing `__rs2b0t_combat_keep_names` / `__rs2b0t_runes_per_cast` JSON helpers and keep_list / combat_style_logic shims unchanged.

## v1 preservation

- `foodCount`: non-array → `0`; slot count; stack qty ignored; falsy rows skipped; `String(foodName)` / `String(name??'')` coercion at JS boundary; Rust `food_forms` when selected cache present, `[key]` fallback when absent.
- `foodHealAmount`: `fixed_food_heal`; unknown/ambiguous → `notImpl` throw (not DEFAULT 8).
- `combatKeepNames` / `runesPerCast`: existing JSON shim paths untouched.

## v2 contracts

- Strict arg validation → `invalid-args`; missing selected cache → `missing-selected-data` (including `foodCount`).
- `foodHealAmount` → `unknown-food` (no DEFAULT 8).
- `combatKeepNames`: case-sensitive mage spell match; staff not subtracted in keep list.
- `runesPerCast`: case-insensitive spell lookup; staff subtraction; `null` unknown spell; `[]` only when spell has no remaining costs.
- `escapeRunesFor`: exact `magic_spell_teleport_{id}`; seven ids verified 274/289; `unknown-id` for case/space/unknown; label `{name} teleport`.

## Pending W5

- Public v1 `escapeRunesFor` export from `hunting/supply.js` with frozen unknown-id empty payload adapter.
- No partial hunting/supply module registered in this group.

## Verification

```text
BOT_NAV_BUILD=skip CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/.worktrees/release-0.1.8/target \
  cargo test -p script --offline --locked -j4 \
  --test supply_helpers --test combat_keep_names --test combat_start_helpers \
  --test native_api_v2 --test host_js
```

All listed tests green (see receipt log). Offline pure-function proof only; no LIVE/gameplay claim.

## Changed paths

- `crates/script/src/food_policy.rs`, `escape_runes.rs`, `supply_v2.rs`, `load/supply_v8.rs`
- `crates/script/src/load/bindings.rs`, `load/mod.rs`, `lib.rs`, `keep_list.rs`
- `crates/script/src/shim/food.js`, `host_js.rs`, `host-js/index.d.ts`
- `crates/script/examples/supply_helpers_v2.ts`
- `crates/script/tests/supply_helpers.rs`, `combat_start_helpers.rs`, `host_js.rs`
- `docs/api/js-api-v2.md`

Commit: `c37a21ad89c1cc4498aa572088ca4858f32a002f`.
