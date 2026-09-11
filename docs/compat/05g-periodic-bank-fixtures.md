# ChickenKiller periodic-bank fixture

This bounded extension keeps both frozen ChickenKiller cards and script
source unchanged. Default `chicken_killer` stays melee Off at the Lumbridge
pen. One shared scenario name:

- `chicken_killer_bank`: `bankStrategy=Loot count`, `bankEveryItems=1`,
  `lootMatch=feather`, `combatStyle=melee`. Both catalogs, both selected
  revisions.

Frozen ChickenKiller.ts is identical across catalogs
(`997caf51e5f409703f4523e74e52f9aa22d388059c36e2810358176b26f9087f`).
Banking settings `bankRules.ts` is identical
(`bd9b6d08c9cb23e437f7c18ec05e5a9a99cfcd4b3065554ca10eece28b51be90`):
labels `Off` / `Loot count` / `Time` / `Either`, `bankEveryItems` min 1.
Selected 274/289 Feather is id 314, name `Feather`, stackable, alias
`feather`. Bones 526 remain the bury keep-list default; they are not the
depositable. Melee `afterDeposit` returns without restock. No autocast.

Default Lumbridge pen `(3235,3295,0)` cannot reach a same-plane
Use-quickly booth inside the reviewed 60_000 ms walk. Castle booths are
upstairs; Draynor / Al Kharid / Varrock West are Chebyshev ≥128. The
fixture therefore anchors at Falador south chickens `(3029,3294,0)`,
immediately south of the host cow-field pin `(3029,3305,0)`. That interior
is Chebyshev 61 from Falador East vs 64 from Draynor, so nearest packed
booth stays Falador East. ChickenKiller `destination()` is null. Return
radius is the reviewed service 6. Native walk/bank timeouts are unchanged.

Prepare only before Start: empty pack, tele to the Falador interior,
acknowledge no Feather 314. Script must cause Strength XP and exact
feather loot, deposit those feathers into a fresh bank generation, return
to the original anchor within 6, close the bank, then new exact Feather
314 in the pack. Same-id `StatXpGain { min: 1 }` cannot witness further
work: the runner keeps the first XP baseline per skill id. Catalog
`ChickenKillerBankCycle` still accepts further Strength XP **or** new
feathers after the deposited baseline. Queued walk/open/deposit, XP-only,
name-only feathers, stale/closed bank, or a return outside radius 6 fail.
Default core ChickenKiller proof is untouched.

Panel and host-play keep using `scenario::get` / `names()`.

## Verification

Implementation baseline: host
`064de2cfcd7f09db1a5f30e91f269efaf6e0c7af`, client
`56d80272bcbda3eb1e22db096c1c5e21d3497de4`, plus these owned files
(fe4a7e41 chicken_killer_bank plus post-return Feather 314). Round-2
export: `/Users/acfrazier/experiments/274bot/.worktrees/t_63524882-src-r2-1222`
(git archive of 064de2cf + verified client copy from the r1 export +
owned overlay including 05g and verification.json; frozen catalog inputs
linked read-only). Isolated empty target:
`/Users/acfrazier/experiments/274bot/.worktrees/t_63524882-target-r2-1222`
(`isolated_build=true`). Shared campaign target was not used. The r1
export/target remain diagnostics only.

- `cargo test -p scenario` — 87 passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 21 passed, 1 ignored (`LIVE` cell).
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check -p scenario` — clean.

No LIVE or fixture process was launched. Root owns the catalog × revision
cells after review. Source review does not grant live acceptance.

## Root LIVE correction, 2026-09-11

Exact isolated 3be68eaf old-catalog runs on both revisions complete the bank
trip and request combat again, but fail the unchanged 180-second scenario
while waiting for a fresh Feather314. Prepared Attack/Strength were both 1;
the first loot takes many attacks and banking consumes most of the remainder.
These failures remain in catalog-harness/live and core-results.json.

The controlled banking fixture now prepares and separately acknowledges
Attack30 and Strength30 before Start, alongside the existing empty-feather
check. Start validation requires those stats. No items, loot or bank contents
are seeded as successful work; combat XP still compares against the actual
post-preparation baseline. The route, bank policy, script settings, product
bounds, 180-second scenario deadline and fresh post-return Feather314 witness
are unchanged. This isolates the banking behavior from slow novice combat;
LIVE qualification of this correction remains pending.
