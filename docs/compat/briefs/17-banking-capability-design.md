# Banking capability design review

Use configured `grok46` defaults. This is a bounded step-6 ownership/fidelity
review before implementation, independent of root's pending step-4 live proof.
Host branch `codex/rs2b0t-multirevision`; inspect accepted operation source at
`b9cacc6e`, current ownership around it, `banking-capability-audit.md`, step 6
of the active plan, and only the relevant frozen Bank/Banking API and card call
sites in the two `.superpowers/inputs/rs2b0t-<full commit>` trees. Read applicable
AGENTS/execution and fail-closed-dispatch. No implementation or broad test run.

Produce a concrete minimal Rust ownership/observation plan for the first
banking capability family:

1. Bank readiness and snapshot generation must distinguish a fresh empty bank
   from unavailable or retained rows. Modal visibility, a nonempty list,
   unrelated inventory traffic and all-family invalidation are insufficient.
   Consider the generic client seam actually required for per-container packet
   publication (including full/partial/stop-transmit and ordering around open,
   close and reconnect), with snapshot/reset ownership in the Rust host. Keep
   shared interface/world ownership and avoid per-frame deep copies. Existing
   client session/grant/explicit-invalidation facts do not identify which
   inventory container was updated.
2. Withdraw-X must wait for the actual count dialog, dispatch its answer once,
   and settle on the resulting observed inventory/bank change. Handle delayed
   dialogs, stale rows, refused operations, timeout, reconnect and Pause/Stop.
   Propose the smallest host-owned bounded pending operation arrangement that
   can reuse current script dispatch/nav state, without introducing a foreign
   planner in JS. Preserve existing runtime/login/frame timeouts and explain
   exact existing bank wait bounds. A queued request is not success.
3. Required `Banking.open({stand, boothName, boothOp})` and `Bank.openBooth`
   arguments must reach Rust navigation and named interaction dispatch.
   Honor the requested access when supported and report concrete missing
   options; don't silently ignore options or copy the foreign Banking router.
   Review BankFletcher's changed cut/string mode and Superheater's fire staff
   callers only to establish this family's contract and later proof inputs.

Thin shims may marshal arguments and await posted Rust observations/results.
If optional FlatBuffer fields or request variants are needed, preserve existing
field ids/semantics and describe compatibility. Avoid designing an unnecessary
universal job framework. Common-loot matching, PeriodicBank/DeathRecovery,
loadouts and other production/combat tasks are subsequent bounded families;
identify dependencies but do not expand this first implementation into them.

Deliver `docs/compat/04-banking-design.md` with source-backed exact contracts,
minimal owned files/seams, what remains unsupported, and a bounded regression
and live proof that could disprove the approach. These include empty/stale bank,
late dialog, reset, wrong access, both revisions, script call -> IPC -> Rust
operation -> posted result. Explicitly distinguish source approval from later
live qualification. Complete this design card with findings/recommendation;
do not edit product files, fixtures, STATE or the matrix, and do not commit,
stash, reset, restore, checkout or alter another worker's WIP/index.

Operator instruction: scripts load/start irrespective of revision. Suitability
belongs to the user; host/client operations refuse unsupported capabilities.
Never introduce a revision/card allowlist as part of this banking work.
