# Seed-bank correction and native loading observation

The first option runs at frozen host 30193c3b/client 56d8027 failed before
Start, at step 9: fresh_bank_item_id(60)>=28. Both 274 headless and 289 native
opened banker dialogue. Raw process/receipt/log artifacts remain under
catalog-harness/live and catalog-headed with that exact source suffix.
Headless exit 1, 150.889 s; native exit 1, 142.245 s. These elapsed values are
functional diagnostics, not performance measurements. Concurrent native and
headless work and agent checks exclude a startup performance comparison.

Selected content bank_booth.loc declares op1=Use, op2=Use-quickly; its rs2
OP_LOC1 handler talks to a banker and OP_LOC2 opens the bank. The seed helper
used generic op_loc (OP_LOC1). Root correction d4f1809e calls the existing
Interactions::open_booth_at with the observed snapshot and exact adjacent
booth tile/id; that resolves Use-quickly with normal stale-identity checks.
It does not change API packet dispatch, runtime banking or stock predicates.
Format-only follow-up cd393879 collapses one call to rustfmt's line shape.

Exact d4f1809e scenario suite: 83 passed. Harness suite: 13 passed, 1 ignored
LIVE. Strict scenario/host-play Clippy passes. The initial exported harness
suite lacked its immutable catalog input symlink; that fixture setup failure
is retained, then the corrected rerun passes. Initial rustfmt failure and
format-only correction are also retained. See checks.json and raw logs.
Same-card custom-Alcher review t_1a24382e/run1182 independently approved
5f86dc87, d4f1809e and cd393879 with actual Grok 4.5. reviews.json records
model/provider/session evidence. Root reruns remain separate LIVE evidence.

The native 30193c3b run also proves the loading UI is visible while hashing.
Root read both internal whole-window PNGs (2240x1160):
- 01:18:42 manual capture: Checking navigation files, 35%, 91,226,112 of
  260,571,161 bytes, seven filled orange cells, no slot; all text fits the rail.
  Its default empty sidecar is UI evidence only.
- 01:20:44 automatic terminal capture: real scene 2, exact seeded input IDs
  60/1777 and banker dialogue; the startup progress banner is cleared. This
  documents the failed fixture, not production script progress.

Both files are in catalog-headed/r289-bank-fletcher-string-100adccc-30193c3b/
shots. The process exited 1 and the owned proof window closed. Root did not
close or modify the operator's older Startup Proof window.

The operator then questioned four repeated navigation bars and clarified the
release contract: precompute the hash during build/packaging and ship the
navpack. Brief 53 now covers a bundled manifest and no normal-release startup
rehashing, with runtime validation retained for external/development packs.
The captured repeated-bar behavior is therefore an accepted implementation
observation and a known UX follow-up, not the final release loading design.

## Corrected fixture result and runtime ownership

The d4f1809e retries opened the seed bank successfully, then failed step 14
requiring XP from both initial stringing pairs. Headless 274 exited 1 at 120.610 s;
native 289 exited 1 at 117.508 s. Both made one strung item 849, leaving input 60
and string 1777 while reporting Nothing interesting happens. The native retry
has no captured internal PNG; its log/receipt establishes this failure only.
Both catalog versions choose inventory items by ID; the bridge reduced those
handles to names. Selected 274/289 data uses the same display name for input 60
and output 849. Source fix t_90e850ab preserves explicit source/target IDs and
slots through the bridge. This is a host/shim defect, not evidence of a broken
foreign script. No stringing PASS is claimed, and these failures are retained.

All eight generated custom-Alcher cells at frozen d4f1809e/client 56d8027 pass:
Adamant scimitar selected by alias and display name, both revisions/catalogs.
Each starts with an empty inventory, observes exact noted item 1332 and Nature
rune 561 in a fresh loaded bank generation, then their consumption, 1536 coins
and 65 Magic XP after Start. Full independent witnesses and verified log hashes
are in catalog-harness/core-results.json. These are bounded option proofs.
