# Observe two independent live slots through shared Play

Use grok46 defaults on codex/rs2b0t-multirevision. Read applicable instructions,
docs/execution.md, fail-closed-dispatch and current STATE.md. This prepares the
controlled same-revision N=2 gate from plan step 8. Do not spawn other agents.
Root owns LIVE launches and frontend acceptance.

Add one standalone ignored host-play LIVE integration test plus uniquely owned
fixture support. Use production Play, selected immutable local profile/template,
ScriptStartHandle and actual frozen catalog source. Do not copy a foreign bot or
make a test-authored loop stand in for catalog acceptance. Existing catalog proof
has qualified Alcher core and custom-item selection, so two Alchers with distinct
supported settings/items are a suitable bounded case. Inspect the frozen source
and existing scenario/harness preparation to select the two branches; avoid
pending runtime capabilities. Reuse existing preparation helpers where public,
or keep any necessary test setup in a unique support module. No shared scenario
or catalog harness edits and no runtime/frontend/client edits.

Require two fresh private actors with distinct account identity and independent
baseline, settings and output witnesses on one same-revision Play/template. Seed
only before each Start, after actual ingame scene2; use exact selected-data
items/stats and acknowledged setup. Both real scripts must cause the expected
item/coin/XP deltas, with no cross-slot result/settings contamination. Then pause
one slot through the production control while the other continues its own work;
resume the paused slot and observe further progress; stop that slot and observe
the other continue. Account for already in-flight native work by a bounded drain
before stability assertions: do not redefine host lifecycle or require an
instantaneous rollback. Verify script state and dispatch/progress observations,
not labels alone. End both actors cleanly and capture final states/errors.

Private vault, loadouts, settings and cache namespace; never operator stores.
Selected local-274/local-289 environment with explicit nav/cache provenance.
Fixture must run either revision; root will launch each. Missing required catalog
must fail closed under LIVE=1. FAIL plus exit 1 for missing operations, wrong
identity, idle/non-progress, phase timeout or isolation failure. Preserve existing
product timeouts. Record per-phase baselines, raw observations and exact frozen
catalog/script hashes with short, bounded deadlines. Native UI and N32 remain
separate gates; this component proof does not claim them.

Own only crates/host-play/tests/two_slot_isolation_live.rs, a unique
crates/host-play/tests/support/two_slot_isolation module if needed,
docs/compat/06a-two-slot-isolation-proof.md and evidence/two-slot-isolation/.
Use meaningful witness tests that reject mixed identities, queued-only success
and no-progress controls. Verify on an exact committed source export plus owned
overlay in a NEW EMPTY Cargo target; affected test and strict Clippy are enough.
Record host/client/source/overlay identity. Commit only owned paths, request
SAME-card profile reviewer with evidence, then STOP. Report a missing production
seam to root rather than extending scope or silently weakening the proof.
