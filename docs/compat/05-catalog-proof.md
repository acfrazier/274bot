# Catalog proof progress

Snapshot: 2026-09-10 22:43 UTC. The 180 fixed rows / 45 enabled cards remain unchanged.
Core passes below are partial row evidence; remaining supported options and the
integrated source review remain open. No complete catalog/fleet/platform acceptance.

| Catalog | Revision | Case | Outcome | Independent delta |
|---|---:|---|---|---|
| 100adccc | 274 | alcher | PASS | {'magic': 65} |
| 100adccc | 274 | chicken_killer | PASS | {'hitpoints': 3, 'prayer': 4, 'strength': 12} |
| 100adccc | 289 | alcher | FAIL | FAIL: catalog_boundary_live: Start baseline player "Livekper24 0" is not fresh account "livekper24_0" |
| 100adccc | 289 | alcher | PASS | {'magic': 65} |
| 8e7d965b | 289 | alcher | PASS | {'magic': 65} |
| 100adccc | 289 | chicken_killer | PASS | {'hitpoints': 3, 'prayer': 4, 'strength': 12} |
| 100adccc | 289 | thiever | PASS | {'thieving': 46} |

Exact source/client/binary/nav/catalog identities, settings, actual baseline,
inventory witnesses and process exits are in evidence/catalog-harness/live/
and the compact core-results.json. Source c933f37c changes only the actor-name
comparison in the 1947741f harness (plus a design brief); its product source is
1947741f/client 56d8027. After that correction the eight offline harness checks
passed with both frozen catalogs linked into the exact export. The missing
feature build and missing-fixture check failures are retained; neither is a pass.

The original 1947741f 289 harness attempt refused Start because the login name
had an underscore while the client displayed a space. c933f37c uses the client
screen-name formatter; it preserves exact actor identity and case handling.

Native Alcher at 1947741f completed 30 actual alchs over 27+3 withdrawals,
deposited 900000 coins, and stopped on exhausted stock on both revisions. Root
read the captured 7/29-alch progress and final bank views (170 Nature runes,
900000 coins, no chainbody stock). The native process exited 0 with no runtime
errors. Post-stop logs stayed clean for 56.68 seconds on 289 and 59.79 seconds
on 274. This is bounded Alcher restock/Stop evidence, not every item/custom/
large-batch setting; t_a384fe64 is adding those option fixtures.

The earlier BoneBurier full-loop failure and BankFletcher packet panic remain
failed cells; t_90b60f12 is implementing their required bank corrections. The
first-XP-only frontend outcomes never qualify those complete bank loops.

Runs can overlap builds or another revision fixture. Elapsed values describe
functional cells and are not comparative performance measurements.
