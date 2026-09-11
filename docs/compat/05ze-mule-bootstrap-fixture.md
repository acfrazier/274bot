# MuleCrafter bootstrap fixture correction

This bounded correction changes only the paired MuleCrafter fixture, its host-owned witness, and focused tests. It does not modify the runtime, client, frozen JavaScript, scenario layer, compatibility ledger, navigation, or LIVE clocks. The implementation is commit `ae2d4752d3d8d9a0691e78831b2bbae05b847a65` on `codex/rs2b0t-multirevision`, based on `13b9ed49f6ac9544e2368a4f73f49c57beedafd7`.

The brief requested the `05zc` report slot, but `05zc-combat-coal-qualification-repairs.md` already occupied that name in the shared campaign checkout. This report uses the next free suffix, `05ze`, without replacing the existing report.

## Verified bootstrap precondition

The frozen MuleCrafter Crafter flow cannot initiate the normal exchange from the former talisman-only, zero-essence baseline. Its crafter path must first produce runes; the mule bookkeeping and trade path then exchange the mule's unnoted essence for those crafted runes. Starting with no raw essence and no produced runes therefore left both sides waiting on a state the scripts could not create.

The paired fixture now seeds the Crafter with exactly Air talisman 1438 x1 and one raw unnoted Rune essence 1436 load of `MULE_TRADE_CAP` x27 at the Air ruins before Start. The baseline contract requires the exact 27 unnoted essence, no noted essence 1437, no produced Air rune 556, and zero Runecraft XP. The Mule remains a distinct account and counterpart with its own first 27-essence pack and acknowledged 200-essence Falador East bank seed. The preparation receipt records counterpart identity, raw seed count, produced-rune count, bank seed, and bank acknowledgement truthfully.

This branch is specific to `SlotKind::MuleCrafter`. The shared NatureCrafter Air fixture retains its prior master/runner preparation and semantics.

## Ordered exchange and further-work proof

The raw Crafter seed and its first craft are bootstrap only. They do not qualify a partner exchange. Each Mule slot now advances an ordered host witness through:

1. an offer interface naming the configured minted counterpart;
2. a confirm interface naming that same counterpart;
3. the immediately following closed-trade observation at the Air ruins with both expected inventory deltas.

For Crafter, one completed exchange requires unnoted essence to enter while crafted Air runes leave. For Mule, it requires unnoted essence to leave while crafted Air runes enter. Only that sequence increments the partner-transfer count and conservation totals. A missing partner publication, wrong partner, missing confirm, later unrelated inventory change, one-sided transfer, empty exchange, or stale open trade cannot qualify.

The first supported claim additionally requires conserved essence and Air-rune quantities across both slots plus a fresh Crafter craft and Runecraft XP after the completed partner exchange. Seeded products or the bootstrap craft alone remain insufficient.

The full-cycle claim preserves and strengthens the existing order: after the first completed exchange, the Mule must deposit received Air runes at an open, loaded Falador East bank; withdraw a fresh unnoted essence load; return to the Air ruins; complete a second fresh offer/confirm/transfer with the named counterpart; and cause a second fresh Crafter craft and XP gain. A second transfer before bank return, or raw inventory deltas without a second counterpart handshake, is rejected.

## Ownership and limits

Native trade capability gaps remain owned by task `t_47426de7`; this fixture does not port foreign negotiation, add auto-accept work, or reinterpret missing host operations. The campaign root owns the actual r274/r289 two-catalog paired LIVE proof after that capability review and this task review are accepted.

`SCRIPT_GOLD_DEADLINE_SECS` remains 180 and `SCRIPT_GOLD_WATCH_TICKS` remains 150. No LIVE process was launched for this task.

## Verification

The exact committed host source and client gitlink were exported to `.superpowers/review-exports/t_c6765b1f-r2` and checked with the exclusive `target-t_c6765b1f-exact-r2` cache. The paired catalog test binary passed 49 tests with its three LIVE tests ignored. Strict Clippy with `-D warnings`, rustfmt check, source hashing, frozen catalog identity checks, and unchanged clock assertions passed.

Raw output and the structured receipt are under `docs/compat/evidence/mule-bootstrap-fixture/`. The retained r1 log records the initial strict-Clippy enum naming failure that was corrected before the successful r2 export.
