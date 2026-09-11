# Correct runecrafting observation order and retain failure witnesses

Use grok46 defaults after BOTH Ardy fixture t_464d9c4c and Air audit t_8a8e6ef3
complete. Root owns accepted scope below; resource fixture t_aa061a28 will wait
for this card's same-card review. Own Rune-only scenario changes and tests in
crates/scenario/src/lib.rs, test-only additions in runner.rs if needed, and a
bounded catalog harness failure witness emission in
crates/host-play/tests/catalog_boundary_live.rs. Own05p-rune-observation-order.md
and unique evidence/rune-observation-order/. No runtime, API, shim, client,
foreign, ledger or unrelated fixture edits. No LIVE.

Root675 native289/newer Earth failed step20 stat_xp_gain(20)>=1 after first
craft and bank deposit/restock, now holding27essence at the second altar.
Raw catalog-headed/r289-rune-crafter-earth-8e7d965b-6750713b-gpu.* retained.
Root read CUA first temple, exact27Earth deposit and Trips2/27essence, then FAIL.
Frozen Scenario order is ruins arrival -> crafted item -> XP watch. Actual
ScenarioRunner::begin_step captures XP only on first StatXpGain watch; an item
and XP arriving in the same snapshot makes the baseline late. The headless
Air failure also had two crafts before its pack-empty watch and is under05o
audit. Read that audit; do not assume the same cause explains every failure.

Implement the smallest Rune-only ordering correction: arm the existing XP
watch after arrival at ruins and BEFORE observing crafted product, so its
baseline precedes the craft. Preserve exact withdrawn essence, rune IDs,
conversion, portal exit, fresh bank deposit, pack-empty, restock, closed bank,
return and further-craft requirements. Keep existing Runner baseline semantics,
timeouts, tick budgets and script behavior. Do not make this a generic runner
rewrite, accept seeded XP/products, skip bank phases or reorder product actions.

Add a meaningful regression with the actual ScenarioRunner and an observation
where rune output and XP change together; demonstrate the original late watch
miss and corrected sequence. This may use a reduced scenario built from the
actual Rune watch sequence with controlled client observations. Verify no
post-Start progress/seed-only input still cannot qualify and full CoreWitness
still requires ordered bank return and further XP. Do not only assert the
literal step order or mirror source values.

For future failed catalog runs, emit the already accumulated CoreWitness plus
current observation on the normal failure path before returning nonzero.
Use existing serialization and raw receipt conventions; do not add a new
tracing system, mutate earlier failures, or derive invented snapshots from
chat. This diagnostic must not change success, deadline or lifecycle policy.
If the audit finds an additional genuine bank/route bug, keep it explicit and
separate; do not silently repair production behavior under this fixture card.

Exact Git-blob host/client export with only owned overlay and a new empty target.
Run affected scenario/catalog tests, meaningful regression and strict affected
Clippy/fmt. Commit only owned changes, request SAME-card reviewer with source
identity and evidence, then STOP. Root repeats failed cells on a fresh reviewed
isolated binary; existing failed receipts remain immutable. No PASS claim here.
