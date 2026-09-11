# Brief133: audit native sequencing observation transfer

Campaign codex/rs2b0t-multirevision. Read AGENTS.md and docs/execution.md once.
Bounded read-only source/evidence audit; own report09b and unique
 evidence/native-sequencing-observation-cost. No implementation, LIVE launches,
world/client/runtime edits, broad benchmark campaign, STATE/matrix/ledger.

Native123 reviewed d142c0ee0/aef now runs in exact21446414 export at
.superpowers/review-exports/catalog-root-21446414. Root controlled BoneBurier
old catalog274/289 logs in catalog-harness/live/*bone*21446414.log show
80/84 slow ticks often50–60ms during WalkNearestBank, a60s open timeout then
retry, actual withdrawal and further bury. First old catalog receipts completed;
newer may still run. Do not infer a matched performance result from concurrent
workers/builds. Product functional acceptance is separately harvested.

Source bank.js bankOpenObservation() passes entire s.locs and s.banks into
__rs2b0t_bank_open on every delayUntil poll; Rust parses locs into a new Vec
with owned strings/actions even during world-nearest traversal, which only
needs nearest_booth, here and bank flags. Inspect cake_stall equivalent too.
Repo says do not deep-copy the world on every read; native ownership correction
must preserve memory/observation efficiency as well as policy ownership.

Determine exact transfer/copy path and phase-specific facts actually required.
Distinguish proven copy work from unmeasured latency attribution. Recommend the
smallest Rust-owned snapshot/borrow/view seam that avoids serializing unchanged
bulk lists back through JS per poll. Do not move target selection/filter policy
back into JS. No giant world clone/cache. Maintain per-isolate/session clearing,
selected identity, native60s/5s/2400ms/3s bounds and Pause/hold freeze semantics.
Check whether the full lists are needed at begin only or need fresh native
identity validation later; stale retained views are not acceptable.

Also inspect nearest walk reach radius vs booth adjacency for the observed
60s timeout/retry without guessing from process timing. The prior JS120s wait
was deliberately removed in123 to restore the documented native60s owner;
do not inflate it as a fix. If evidence lacks route timing/arrival details,
name one bounded future diagnostic that will resolve it.

Deliver source-bound diagnosis and a concrete scoped implementation suggestion,
not a blanket performance claim or foreign JS controller. No new tasks.
