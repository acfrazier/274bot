# 162: Exact egg bank approach correction

The exact HerbloreSecondaries egg preparation remains shared by both selected
revision packs. Headed-shot evidence identifies booth 2213 at
(3096,3493,0) with the native `Use-quickly` action. The prior preparation
teleported to (3094,3493,0). In both the 274 and 289 headed runs, the actor
stood there; `Interactions::open_booth_at` selected the exact booth but its
native approach target was (3095,3493,0), which the client refused as
`Unreachable`.

The bounded correction teleports to (3096,3494,0), a collision-free,
non-booth tile shown in both headed shots and Chebyshev-1 from the exact
booth. This makes the existing exact opener interact from an immediately
adjacent native stand while keeping identity, action, stale-target,
reachability, and bank API checks unchanged. The pre-Start readiness still
acknowledges exact loc 2213 and `Use-quickly`; no egg, noted egg, or Eye of
newt is seeded.
The fixture still seeds exactly 50 Lobsters in the fresh bank, closes the bank,
then starts the catalog at the egg field for real Take/deposit/return/further
production observation.

No LIVE run was performed by this task. Focused scenario tests and formatting
are the required local validation; root owns the justified headed reruns for
both revisions after review.
