# Combat evidence serialization repair

The exact b18 combat batch stopped after three completed cells: revision274
MossGiant PASS and both revisions ChaosDruid FAIL. The append-only ledger now
contains419 receipts. Root terminated only the batch wrappers, let the active
cells finish, and reaped all processes. The aborted batch metadata records this
boundary; unrun cells have no result.

Both ChaosDruid failures exposed the same harness defect: a populated
`CombatCoreCycle.previous_ground` uses tuple map keys that serde_json cannot
encode as object keys. Revision274 panicked in the qualified-result path;
revision289 panicked in accumulated failure diagnostics. Reaching the274
qualifier does not turn its failed process into a pass, and the289 underlying
failure still requires a fresh diagnostic result.

Root commit `cffd62c59` serializes that map as deterministic named rows containing
item id, x, z, level and count. The observer map, update rules, qualifications,
scenario preparation, settings and clocks remain unchanged. New regressions
populate the map via observations and exercise both actual receipt paths.
Both reproduced the JSON panic before correction (exit101); all11 selected
combat-related tests pass after correction. Logs and exact source identity are
in `evidence/combat-ground-serialization/`. Shared mage work was preserved through
an isolated-index commit. This support-tool fix does not require a separate
routine review; the campaign whole-branch review remains required.

Fresh LIVE qualification follows the corrected exact-source binary build.
The previous failures and the one core PASS remain in
`evidence/catalog-harness/qualification-combatb18.json`; they do not qualify
all supported options or the full combat batch. No gameplay workaround or
performance claim follows from this evidence repair.
