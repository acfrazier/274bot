# Isolated Superheater qualification

Actual Grok 4.5 review 1212 approved fixture source 6d750e65. Root built it in
a new empty target with client 56d8027 and ran all ten intended cells. All eight
Bronze/Steel catalog/revision cells pass with observed staff equip, exact ores,
nature runes, bars, both XP types, deposit/restock and further production.
`harvest_isolated.py` independently validates the saved witnesses and hashes;
`isolated-results-6d750e65.json` preserves the ten results.

Both newer-catalog alternative Fire battlestaff cells fail at wield. Their
baselines have Magic 43 and Attack 1. The selected 274 and 289 content both
require Magic 30 and Attack 30 (`scripts/levelrequire/scripts/tier30.rs2:78`);
the bounded source/hash receipts are in `fire-battlestaff-wield-requirement.json`.
This is a preparation defect. The correction must seed and acknowledge Attack
30 before Start, then repeat only those two failed cells on the corrected
isolated build. Keep these failures. Do not change gameplay requirements or
foreign script behavior, and do not dim the card for a fixture error.

Root is waiting for the active PeriodicBank fixture owner to release the shared
scenario/harness files before making the small seed/baseline correction. The
first eight results are bounded headless evidence; other supported settings,
native frontend and integrated acceptance remain separate. Diagnostic overlap
precludes a performance comparison.
