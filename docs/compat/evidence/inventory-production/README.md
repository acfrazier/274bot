# Isolated inventory-production qualification

Host a4157243817d8606f6c22a5ca2583f106d98bb85 and client
56d80272bcbda3eb1e22db096c1c5e21d3497de4 pass 24 headless cells on macOS:
274/289 x both frozen catalogs x Bronze/Iron darts, default/named herbs,
and default/named gems. All receipts bind a hash-verified immutable source
export to a binary built in a target that started empty. Actual same-card
source reviews are retained; earlier shared-target runtime results remain
historical diagnostics.

Root independently verified all 24 log hashes, valid in-game/scene-2 baselines
and terminal observations, exact output IDs, positive skill XP and the cycle
witnesses. Darts produce 100 of the selected tier (806 Bronze / 807 Iron),
with 180 / 380 Fletching XP respectively. Herbs clean a complete pack of guam,
deposit the result, withdraw fresh stock and clean further exact 249, with
72 Herblore XP. Gems cut a complete pack of sapphires, deposit, withdraw and
cut further 1607 while preserving chisel 1755, with 1400 Crafting XP.
Named filters show no wrong-material witness. Summary: isolated-results-a4157243.json.

Six additional BankFletcher stringing cells on the same build pass. The two
old-catalog cut-and-string invocations were root preflight errors: that catalog
has no such setting, no actor started, and the exit-1 evidence remains.
These 24 passes qualify named gameplay branches, not every material/setting or
frontend/lifecycle combination. Diagnostic runs overlapped; no timing or memory
comparison is claimed.
