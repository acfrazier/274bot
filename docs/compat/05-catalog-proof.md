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


### 23:55 UTC full-loop and platform follow-up

At frozen bc499cbc/client 56d8027, Mac headless 274 and native 289 BoneBurier
both completed real banking, withdrew 28 bones, closed the bank, and buried
another bone. The native run continued to 33 total burials/one bank trip. All
receipt log hashes were reverified. The other catalog/revision combinations
and supported settings still need qualification.

BankFletcher native 289 at 2f9099f7 made and deposited 27 willow shortbows
without a panic, then stopped on empty bank stock. Its first-craft scenario
PASS does not qualify restocking. Root source 6c6bb2d5 seeds 54 bank logs before
Start, requires first-pack deposit and same-generation withdrawal, then requires
crafting a withdrawn log with XP progress. The scenario requires at least 900
XP, beyond the maximum 899 integer XP from the 27 carried logs. Nine harness
witness regressions and strict Clippy pass; fresh full-loop LIVE is pending.

The same source enables existing internal terminal captures for BoneBurier,
ChickenKiller, Alcher/variants and BankFletcher. Previous empty screenshot
directories reflected unarmed milestones. Subsequent native proofs will read
the application's complete frame and snapshot sidecar.

Windows native 289 Alcher at 584d05bc completed its scenario, continued through
30 casts, deposited 900000 coins and stopped cleanly (198.079-second outer
run, zero runtime errors). Its log hash is verified and both root-owned
scheduled tasks were removed. The binary predates startup source 814e5293, so
the Windows startup stall has not yet been qualified as fixed. The retained OS
captures are DPI-cropped and do not establish whole-frame visual acceptance.


### 2026-09-11 00:25 UTC complete core bank-cycle follow-up

BoneBurier at bc499cbc and BankFletcher at 6c6bb2d5 now each pass the headless
core bank cycle on both revisions and both frozen catalogs (eight new qualified
cells). Their exact binary, source, catalog, nav, baseline and fresh-bank
observations are retained in core-results.json and the raw receipt/log pairs.
The earlier first-action BankFletcher result remains PARTIAL and the failed
BoneBurier travel attempt remains FAIL. Every completed log hash was reverified.

| Card | 274 / 100adccc | 274 / 8e7d965b | 289 / 100adccc | 289 / 8e7d965b |
|---|---|---|---|---|
| BoneBurier | PASS | PASS | PASS | PASS |
| BankFletcher | PASS | PASS | PASS | PASS |

BoneBurier witnesses initial burials, fresh bank stock, withdrawal of 28 bones,
bank closure and another burial. BankFletcher witnesses 27 initial products,
deposit of those products, same-generation withdrawal of 27 banked logs,
bank closure and crafting from the new pack (+932 fletching XP overall).
The eight support-matrix rows move to PARTIAL; unexercised settings remain open.

Native 289 BankFletcher at 6c6bb2d5 also completed the full scenario, then the
41cfc85e diagnostic build reproduced it with a read internal full-window PNG
and actual scene-2 sidecar. Earlier armed but unpresented captures are retained
as capture diagnostics. The owned app bundle contained the exact frozen binary.
Native Windows 6c6bb2d5 Alcher completed 30 casts and banking and supplied an
internal full-window PNG. Startup samples, responsiveness probe limits, copied-
resource refusal and exact capture paths are in 06f-panel-startup-native-proof.md.
These results do not qualify every option or the remaining frontend/fleet gates.

### 2026-09-11 01:10 UTC ordered Alcher follow-up

Selected-data source 7852b5a0 passed actual Grok 4.5 corrective review 1175
(report 8c6689c1). Root built frozen host 1a2f2dcf/client 56d8027 and verified
all 1,452 exported files plus both executable hashes. All four ordered Alcher
cells now pass: both revisions and both frozen catalog versions. Each observed
Rune platebody first, exhaustion and restocking of Rune chainbody, then the
second actual cast, for +130 Magic XP. The independent witness includes
ordered_first_exhausted=true and both targets' acquisition/consumption.
All receipt/log hashes were reverified; raw artifacts and full witnesses are
retained. core-results.json now records 32 PASS, three FAIL and one PARTIAL;
these are historical cells, not 36 completed matrix rows.

The old 41d7a1e3 ordered failure remains in the ledger. These new results close
that selected-metadata ordering failure. Earlier custom cells still used only
Rune chainbody; brief 51 scopes a separate selected-data custom-item witness
for Adamant scimitar by alias and display name. No arbitrary custom-item LIVE
acceptance is claimed yet. All Alcher matrix rows remain PARTIAL pending the
remaining option and integrated/frontend gates.

### 2026-09-11 01:46 UTC generated custom-item qualification

All eight custom alias/display-name Alcher cells pass at frozen d4f1809e/client
56d8027: Adamant scimitar on both revisions and both immutable catalogs. Root
verified receipt/log hashes, empty Start inventory, fresh loaded bank generation
with exact noted item 1332 and rune 561, then both consumed, 1536 coins and
+65 Magic XP. Each retained 401-431 post-Start observations and a full core
witness. Same-card Grok 4.5 review 1182 approved the source and seed helper.
These qualify these two selected-data options, not every arbitrary item.

BankFletcher stringing at 30193c3b failed before Start because the seed helper
opened banker dialogue. Corrected d4f1809e opened the seed bank, then failed
after the first bow: the shim discarded valid IDs/slots and reselected a
same-name finished bow. Both 274 headless and 289 native failures are retained;
t_90e850ab repairs that bridge identity loss before fresh LIVE retries.
The ledger now contains 40 PASS, five FAIL and one PARTIAL historical headless
cells. Native receipts remain separate; no stringing success is claimed.

The operator clarified catalog ownership: a demonstrated foreign-script bug
gets a dimmed card and a concrete reason, preserving its original inventory
row and evidence. Host behavior is not changed to accommodate it. Ancillary
features such as clue solving remain stubs for separate future native host
projects; that existing deferred dependency is not a foreign-script defect.
Current BankFletcher evidence identifies our bridge bug, so it remains enabled.
