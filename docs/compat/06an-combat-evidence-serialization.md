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

## Fresh corrected-source diagnostic boundary (16:51 UTC)

Exact `cffd62c598ceab2ea80b9bcc29e0eaf58f4dd885` / unchanged client was
built from4966 verified files, with no overlays; binary/source receipts are
in catalog-headed. Both fresh Chaos cells returned bounded-timeout FAIL with
complete JSON diagnostics, no serializer panic. Combat facts and XP progressed,
but selected herb/law/nature loot was never acquired. Compact diagnostics are
in combat-ground-serialization/fresh-chaos-diagnostics.json. The source hunt
policy permits aggression at the wilderness fixture and the script only loots
outside combat; that is a possible explanation, not a proven script defect.
No unchanged Chaos rerun is planned.

At this boundary289 has finished,274 continues, and the batch has14PASS/9FAIL.
Moss/Hill/Auto core pass all four cells;289Coal both pass. RockCrab's required
post-Start wake is absent from an already-awake baseline; GreenDragon Repeat
wear rejects before Start; FireGiant encounters missing reader.npcBox during
paint. Fixture139 and native144 own those corrections. ArdyFighter289 makes
Attack/Thieving progress but never reaches selected Strength style;139 tests
its supported one-food target while preserving real theft and combat witnesses.
All failures remain, and incomplete batches/options are not final acceptance.
