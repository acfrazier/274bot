# Catalog proof progress

Snapshot: 2026-09-10 23:18 UTC. The 180 fixed rows / 45 enabled cards remain unchanged.
Twenty Mac harness cells pass: twelve basic cells (Alcher, ChickenKiller and
Thiever across both catalogs/revisions) and eight Alcher custom/1000-batch cells.
Two diagnosed failed cells are retained below. All rows remain partial pending
remaining options and integrated source review.

| Catalog | Revision | Case | Outcome | Independent XP delta / failure |
|---|---:|---|---|---|
| 100adccc | 274 | alcher | PASS | {'magic': 65} |
| 8e7d965b | 274 | alcher | PASS | {'magic': 65} |
| 100adccc | 274 | alcher_custom | PASS | {'magic': 65} |
| 8e7d965b | 274 | alcher_custom | PASS | {'magic': 65} |
| 100adccc | 274 | alcher_large_batch | PASS | {'magic': 65} |
| 8e7d965b | 274 | alcher_large_batch | PASS | {'magic': 65} |
| 100adccc | 274 | alcher_ordered | FAIL | FAIL: catalog_boundary_live: script error: script requested stop on tick 53; isolate stopping |
| 100adccc | 274 | chicken_killer | PASS | {'hitpoints': 3, 'prayer': 4, 'strength': 12} |
| 8e7d965b | 274 | chicken_killer | PASS | {'hitpoints': 3, 'prayer': 4, 'strength': 12} |
| 100adccc | 274 | thiever | PASS | {'thieving': 46} |
| 8e7d965b | 274 | thiever | PASS | {'thieving': 46} |
| 100adccc | 289 | alcher | FAIL | FAIL: catalog_boundary_live: Start baseline player "Livekper24 0" is not fresh account "livekper24_0" |
| 100adccc | 289 | alcher | PASS | {'magic': 65} |
| 8e7d965b | 289 | alcher | PASS | {'magic': 65} |
| 100adccc | 289 | alcher_custom | PASS | {'magic': 65} |
| 8e7d965b | 289 | alcher_custom | PASS | {'magic': 65} |
| 100adccc | 289 | alcher_large_batch | PASS | {'magic': 65} |
| 8e7d965b | 289 | alcher_large_batch | PASS | {'magic': 65} |
| 100adccc | 289 | chicken_killer | PASS | {'hitpoints': 3, 'prayer': 4, 'strength': 12} |
| 8e7d965b | 289 | chicken_killer | PASS | {'hitpoints': 3, 'prayer': 4, 'strength': 12} |
| 100adccc | 289 | thiever | PASS | {'thieving': 46} |
| 8e7d965b | 289 | thiever | PASS | {'thieving': 46} |

Exact source/client/binary/nav/catalog identities, settings, baseline, inventory
witnesses and process exits are in `evidence/catalog-harness/live/` and
`evidence/catalog-harness/core-results.json`. All recorded log hashes were
reverified when producing this table. Source c933f37c corrects client display-name
identity; 41d7a1e3 adds independently reviewed option fixtures and strict witnesses.

The initial 1947741f 289 attempt refused Start on underscore versus display-space
identity. The 41d7a1e3 ordered 274 run requested Rune platebody and chainbody but
selected only chainbody: `api::content::ITEMS` has one gold row. It cast that item
and stopped; the full ordered witness correctly did not pass. t_48d33ff0 generates
server-derived JSON assets and t_f1572cf0 will consume them through serde, per
operator direction. No unchanged ordered retry is accepted. The custom checks
here use Rune chainbody; arbitrary custom-item availability remains unqualified.

Each 1000-batch cell observed 1000 noted chainbodies and 1000 Nature runes in
two inventory slots, followed by one actual cast, both counts 999, 30000 coins
and +65 Magic XP. These cells qualify the batch-size path, not 1000 sustained casts.

Native Alcher at 1947741f completed 30 actual alchs over 27+3 withdrawals,
deposited 900000 coins and stopped on exhausted stock on both revisions. Root
read progress and final bank captures. Post-stop logs stayed clean for 56.68
seconds on 289 and 59.79 seconds on 274. Native 274 Thiever completed its
180-second observation without script errors; the read capture shows three
steals/90 coins with banking off. It is not eating/restocking proof.

The earlier BoneBurier bank failure and BankFletcher packet panic remain failed
cells. Candidate 2f9099f7 completes their scoped bank paths and is in source
review; corrected full-loop LIVE is still pending.

Linux 289 basic Alcher passed at frozen c933f37c. Windows c933f37c failed before
login on a path-format comparison; 584d05bc canonicalizes both paths before
comparison. Separate platform logs are under evidence/platform-preparation/
catalog-smoke/. Neither result implies native frontend/fleet acceptance.

Functional runs may overlap builds or another revision fixture. Elapsed values
are not comparative performance measurements.

### 23:34 UTC follow-up

The first corrected BoneBurier runs at host `2f9099f7` / client `56d8027`
failed on both revisions at the 150-tick bank-arrival observation while still
walking around the river toward the nearest booth. Mac native 289 ended at
(3302,3231,0); Mac headless 274 at (3300,3229,0). Five seed burials are not a
full-loop pass. The harness now allows this single travel observation 240 ticks
to cover the script's existing 120-second travel timeout, preserving product
timeouts, other observations and the 180-second scenario deadline. Existing
BoneBurier proof-order and shared-budget tests passed. Fresh runs are required.

Linux 289 Alcher headless at `c933f37c` passed; Windows 289 Alcher headless at
`584d05bc` passed after canonicalizing both compared catalog paths. Each
observed 27 to 26 items/natures, 30,000 coins and 65 Magic XP. Native Windows
is a separate pending run. Build receipts and the original missing-NASM build
failure and Windows path-comparison live failure are retained under
`evidence/platform-preparation/catalog-smoke/`.
