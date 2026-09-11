# Design the missing host-owned named-bank metadata mapping

Use grok46 defaults and fail-closed-dispatch. Bounded read-only design; own only
04n-native-named-bank-metadata.md and unique evidence/named-bank-metadata/.
No runtime, fixture, ledger, STATE or foreign-source edits, no LIVE, no subagents.

Both isolated675 MuleCrafter Air blank-partner cells fail after Start with
`bankTile: unknown bank 'Falador East'`. Raw receipts/logs are under
catalog-harness/live/r{274,289}-mule-crafter-100adccc-6750713b.*. The actual
MuleCrafterLogic.bankTile imports BANK_LOCATIONS and finds by name; our
crates/script/src/shim/bank_locations.js exports an empty array. This is our
missing mapping, not a foreign defect. Root retains FAILs and no dim verdict.

Determine the smallest native-owned source of named bank facts required by
supported catalog callers, especially Rune/Mule and cookLocations. Do not copy
the foreign BankLocations table or its quest/skill/global-setting, distance
ranking, access fallback, dangerous-bank preference or routing policy. Location
names needed by foreign arguments may be compatibility aliases to verified
host data. Existing world.banks() derives booth coordinates, display names and
access from selected content and posts immutable/reused bank facts; its labels
are normally Bank booth, not Falador East. A booth tile and a reachable stand
tile are different facts: preserve that distinction and existing walk semantics.

Inspect selected274/289 content, packed navigation and current game-data
architecture. Propose a compact native metadata source with content/profile
identity, backed by actual booth placements and valid stand/collision checks.
Account for list import before first snapshot, profile reset/lifecycle and
existing shared snapshot ownership without per-read deep copies. Explain how
BANK_LOCATIONS array callers see accurate facts and refuse unavailable entries.
Do not offer a static made-up Falador row, clone an entire foreign bank table,
change native closest-bank policy, or treat a live nearest booth as the named
whole-world list. Do not claim all bank helpers supported from this one mapping.

Produce a bounded file ownership plan, meaningful tests and one concrete LIVE
proof root can run. Current host-play/runtime shared files belong to hostile
observation t_eeea20f0 followed by special/teleport/shop/Make-X/fire; Bank.js is
under named-approach review t_4948af53. Prefer independently owned native module
plus minimal gated integration, but do not code in this read-only task. State
which capabilities remain unsupported and why. Reference exact source hashes
and source evidence for names/coordinates; no broad framework or new policy.
