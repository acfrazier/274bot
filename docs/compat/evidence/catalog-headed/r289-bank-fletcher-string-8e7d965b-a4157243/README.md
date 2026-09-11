# Isolated native BankFletcher stringing

Reviewed host a4157243/client 56d8027; isolated native binary built from the
hash-verified export. Revision 289, newer frozen catalog 8e7d965b. Root read the
terminal capture: ingame scene 2 at Varrock bank, stringing overlay reports seven
bows and one bank trip, with finished and unstrung items visible in inventory.
The paired headless cell for this same source/catalog/revision verifies exact
ID-based initial crafting, deposit, restock and further output. Native runtime
finished with exit 0 and no unhandled runtime errors. After finite fixture stock
was exhausted, the script requested its normal stop; no all-options or lifecycle
acceptance is inferred from this run.

Native loadout inspection after the stringing terminal proof exposed layout
clipping. The later separate UI-only 50f2be8a run verifies the correction; those
isolated editor actions are not script provisioning proof. Actual binary, log,
timeline, image and sidecar hashes are in native-proof.json and the parent receipt.
