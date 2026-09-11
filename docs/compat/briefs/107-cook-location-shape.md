# Map native cooking stands into the consumed location shape

Use grok46 profile defaults on codex/rs2b0t-multirevision. Own only
crates/script/src/shim/cook_locations_data.js, cook_locations.js, a unique
crates/script/tests/cook_location_shape.rs, docs/compat/04ac-cook-location-shape.md
and evidence/cook-location-shape/. No shared mod.rs, API, runtime, fixture,
frontend, client, navigation, foreign source, STATE or ledger edits. No LIVE.

Root exact912289 old-catalog cook_bot and cook_bot_lobster both stopped during
onStart (ticks45/39). Raw records are evidence/catalog-harness/live/
r289-cook-bot[-lobster]-100adccc-91289e9b.*. Root source inspection found a
concrete mapping mismatch: cook_locations_data currently returns bank:Tile,
range:Tile; frozen CookBot consumes where.bank.tile and where.surface.stand,
approach, locName, arriveRadius. Missing surface deliberately calls stop.
Do not claim the stop reason was logged; it was inferred from source.

Read both frozen inputs' CookBot, CookLocations and cookLocations data shapes,
current native api::content::COOK_STANDS and existing named-bank mapping.
Keep native Catherby bank/range coordinates authoritative. Produce the shape
actually consumed using those posted facts and existing host bank mapping.
Do not clone the foreign derived surface database, router, bank selection or
access policy. Missing unsupported facts must stay explicit/null; no arbitrary
nearest-location fallback or invented surfaces. Custom and Auto behavior must
be described honestly; this bounded card need not implement new Auto policy.
If a required native fact is missing, report the exact fact rather than adding
new native coordinates or guessing an approach/loc tile. Do not label a surface
verified by a source-only test or reuse a stand as a fabricated loc observation.

Test a real consumer traversal of the posted Catherby bank/surface shape and
missing/unknown facts, not just serialized equality. Preserve host navigation
arrival/radius and all timeouts; existing range target remains the host stand.
Exact source export plus owned overlay, reuse your exclusively owned compiler
cache for implementation/review. Check free disk before compile; no duplicate
full build for nominal independence. Root cleans compiler outputs after review.
Relevant existing script checks and strict Clippy, scoped commit, SAME-card
reviewer with exact source identities and limits, then STOP. Root owns fresh
catalog LIVE and any subsequent missing operation reached after startup.
