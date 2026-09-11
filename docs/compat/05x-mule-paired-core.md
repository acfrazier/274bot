# MuleCrafter Air pair on the existing harness

`crates/host-play/tests/paired_catalog_live.rs` and
`crates/host-play/tests/support/paired_catalog.rs` remain a standalone
ignored LIVE pair harness. This card adds `PairCase::Mule` with actual
frozen Crafter+Mule actors. Air and Duel are preserved. No runtime, API,
scenario, `catalog_boundary_live`, engine, imported catalog, ledger, or
STATE edits.

Design 05w sections 4B/6 are source guidance. The frozen MuleCrafter
flow was read on both catalogs before encoding witnesses. Root owns
acceptance and any LIVE replay after Paint 113.

## Frozen flow (not weakened)

MuleCrafter.ts
`bf745db4c0a3df22406b49c8a8b716b10e0594302853d80ca6f38862d05dc1f9` and
MuleCrafterLogic.ts
`d9cc408c1857a02332e3338e1ae1e956dea51c7c191d4a66af81be6e5108251a`
are identical on `100adccc` and `8e7d965b`. Settings: `rune='Air rune'`
(singular), `mode` Crafter/Mule, required mule partner, `bankFill=true`.
TRADE_CAP 27. Ruins `(2983,3288,0)`, Falador East `(3013,3355,0)`.

Mule carries unnoted 1436 to the ruins, `Trade.request`s the crafter,
`offerAll('Rune essence', id===1436)`, two-stage accept. Crafter
`CrafterTradeAtRuins` accepts when `Trade.active()` and
`classifyMuleState` sees essence, offers non-talisman items (the Air
runes) only then, crafts after receiving essence, and
`CrafterGoBank`s after `hasAllMulesTraded`. Mule `MuleGoBank` deposits
received Air runes at Falador East, then withdraws a new 1436 load.

The mule, not the crafter, is the role that deposits received 556 and
restocks essence for a further exchange. Full cycle keeps that
requirement. Crafter packing their own crafted runes to the bank is not
a mule deposit.

## Design error (reported, not used to weaken the core)

Design 4B seeds the crafter at the ruins with Air talisman 1438 and no
essence, mule firstload 27 after Falador East `givebank` 200. That first
trade is essence-only: the crafter has no runes to offer. After craft,
`hasAllMulesTraded` is true, so `CrafterGoBank` walks the crafter to
Falador East with the newly crafted 556. The mule remains at the ruins
with 0 essence and 0 runes, so `MuleGoBank` does not validate. The
crafter's bank was not seeded, so `bankFill` withdraw fails.

That path cannot produce mule-received-556, mule deposit, or a second
mule transfer. It is a design/prep mismatch with the frozen simultaneous
rune-for-essence trade, which needs the crafter to already hold runes
when the mule offers essence. This card does not seed crafter essence or
Air 556, does not count the crafter's own deposit as mule deposit, and
does not reclassify first-exchange as full. Partial
`FirstExchangeCraft` can still hold (mule 1436 out, crafter 556 + RC XP
from script). Full `MuleBankReturnSecondCycle` stays the named witness.
If gold expires after partial, print partial.

## Preparation and Start

Same shared Start barrier (05s). Same 180s prep, 180s gold, 150 watch
ticks. Harness may teleport/seed/ack bank before Start only. No
post-Start loot, coins, teleports, Trade clicks, or player aliases.

Crafter: `mode=Crafter`, `rune='Air rune'`, `partner`=mule IGN,
`bankFill=true`, talisman 1438, no essence, at ruins. Mule: `mode=Mule`,
same rune, `partner`=crafter IGN. Prep mule like Air runner: Falador
East booth ack `givebank blankrune 200` open+loaded+close/generation,
then firstload 27 unnoted at ruins. Do not seed 556, RC XP, or noted
1437. Air talisman only on the crafter. Empty mule partner throws in
script `onStart` and is a fixture miss, not a LIVE cell. Both-Crafter is
a fixture miss.

## Witnesses

Partial `FirstExchangeCraft`: both offer+confirm with the minted
counterpart; 1436 left the mule; crafter 556 ≥ 1 and RC XP ≥ 1 from
script; conservation of transferred 1436 (crafter in == mule out, or
crafter already converted to 556). Seeded runes fail. Stale open trade
after claimed craft fails.

Full `MuleBankReturnSecondCycle`: mule held script 556, deposited it at
a loaded Falador East bank, withdrew a **new** 1436 load (first 27 is
seed), returned to ruins, second transfer, further crafter craft
(`craft_events >= 2`). First-transfer printed as full is rejected.

## Verification

Exact Git regular blobs of host `96ff5ff12f33470f731b03ba13ac0a4ca7837ec6`
plus client `9d090ed04957e4efc254f073cda97bc5510ca72b` plus owned overlay:
`.superpowers/review-exports/mule-paired-core-t_aba60704`. Isolated empty
target `.superpowers/review-exports/mule-paired-core-t_aba60704-target`
(`isolated_build=true`). Shared campaign target was not used. One exclusive
target reused for the reviewer.

- `rustfmt --check --edition 2021 -- crates/host-play/tests/paired_catalog_live.rs crates/host-play/tests/support/paired_catalog.rs` — pass.
- `cargo test -p host-play --test paired_catalog_live` — 37 passed, 3 ignored
  (existing Air/Duel identity/conservation/full-vs-partial plus Mule wrong-partner,
  one-sided, seed-only, missing conservation, stale trade, empty partner, both-Crafter,
  no mule bank deposit, no restock, no second cycle, first-exchange accept,
  prepared-current, frozen hashes).
- `cargo clippy -p host-play --test paired_catalog_live --no-deps -- -D warnings` — pass.
- `cargo test -p host-play --test paired_catalog_live paired_catalog_mule_live -- --exact` — 0 passed, 1 ignored.

No LIVE client or engine was launched. These checks are fixture/build
evidence, not gameplay acceptance. Root owns both-revision/catalog LIVE
after Paint review. Solo Mule Air PASS does not qualify this pair.
