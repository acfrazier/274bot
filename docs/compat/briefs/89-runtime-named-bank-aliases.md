# Publish selected-world named bank aliases before catalog evaluation

Use grok46 defaults after count-dialog t_ee0d3dbd completes same-card review.
Special t_68de6f48 will wait for this card. Read design04n, but apply the root
corrections below. Use fail-closed-dispatch. No foreign edits, LIVE, fixture,
ledger, timeout, ranking or packet changes.

MuleCrafter both old-catalog isolated675 runs fail after Start on unknown
Falador East because BANK_LOCATIONS is empty. Implement this missing host
mapping. Publish only the five existing shim RUNES.bank names (Falador East,
Varrock East, Edgeville, Draynor, Al Kharid), backed by the selected native
NavWorld's actual bankbooth placements and walkable stand tiles. Preserve
live nearestBank/nearest_booth, unnamed packed Bank booth rows, navpack format,
all closest-bank/open-booth behavior, and separately owned cook_stands.
Do not copy the foreign 18-row table, requires/access/npcAccess, quest gates,
foreign approach/ranking/router or optional feature implementations.

Root corrections to04n are requirements:
- Test-only checks against an input-audit file DO NOT permit publishing static
  aliases at runtime. Validate against the actually bound NavWorld before
  isolate module evaluation. Absent world, missing relevant booths, wrong
  plane, invalid/blocked stand or incompatible identity => omit that alias.
- The proposed Edgeville3094,3493 and Draynor3093,3243 stands are distance2 from
  the listed booth tiles, contrary to the report's adjacency condition. Derive
  or choose a native walkable adjacent stand within that named bank cluster;
  verify selected274/289 packed data. Do not silently omit supported aliases
  or weaken adjacency just to preserve the draft coordinates. Falador East
  3013,3355 and Varrock East3253,3420 must also pass real-world validation.
- This is small immutable per-profile metadata. Build/share the few resolved
  rows with the native resource lifecycle and post once before modules eval;
  do not deep-copy world/banks every read/tick or mutate a global across slots.
  A new profile/isolate gets its own correct facts after Stop/reset/rebind.
- An exception disappearing is not full Mule acceptance. Root later requires
  original full withdraw/enter/craft/exit/deposit/restock/return/further core.

Own new api/named_banks.rs + api/lib.rs module export, the minimal native nav
validation/resolution seam (nav/world.rs and a unique nav test if needed),
script shim bank_locations.js/mod.rs, load.rs/slot.rs and their minimal native
host-play construction wiring needed to pass resolved facts. Inspect existing
selected-game-data and world ownership first; prefer a typed immutable facts
argument and compatible existing constructors. Do not make api depend on nav,
introduce a new service or change generated game-data schema. A needed caller
update is owned composition, preserve every unrelated behavior in shared files.
Own NEW script/tests/named_bank_locations.rs and native tests that exercise
real invalid/missing-world and cross-profile cases. Do not edit others' tests
except unavoidable constructor composition, reported explicitly. Own04q-native-
named-bank-aliases.md and unique evidence/runtime-named-banks/. No UI changes.

Prove actual isolate import-time BANK_LOCATIONS.find/Tile methods and missing
name behavior, absent-world omission, blocked/wrong-plane/missing booth
rejection, independent worlds/reset, and unchanged nearestBank. Verify all five
aliases on both pinned real navpacks using a bounded native check, not just a
handwritten array. Frozen Mule bankTile call shape should see valid Falador.
Use exact host/client export plus owned overlay and separate empty target;
check disk before builds and leave cleanup to root. Focused affected tests and
strict affected Clippy, no broad tests mirroring a table. Commit owned paths,
request SAME-card reviewer with exact identities/evidence then STOP.
