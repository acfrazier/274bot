# Selected-revision game metadata ownership

The checked-in `crates/api/data/game-data/{274,289}.json` files are the runtime
source for generated item, fixed-consumption and pickpocket facts. The API crate
deserializes the selected revision through one process-static `Arc`, validates
schema/revision plus the bound profile's cache identity, and never consults a
server checkout at runtime. A 274 profile cannot read the 289 static and an
unknown alias or name stays absent.

`SharedClientTemplate` owns the selected `Arc` beside the cache and navigation
world. `Play`, `ScriptStartHandle`, and each started isolate borrow that immutable
selection; they do not copy it into per-tick snapshots or per-bot world state.
Legacy/offline Play paths carry no selected game data. At isolate startup the
script shim publishes one JSON content object before catalog module evaluation.
The JS module necessarily owns that one startup copy; no game-data table is
serialized on snapshots or ticks.

Generated rows provide every aliased selected-cache item to `ITEM_DB`, preserving
alias, id, display name and cost. Same display names remain separate rows and
aliases are never synthesized by underscore normalization. Only display names
whose generated facts all qualify as the same `fixed_hp_heal` become food-heal
lookups; conflicting or conditional same-name variants remain unavailable.
Curated pickpocket stands and leashes stay host policy, while
`required_thieving` is joined from generated NPC display-name
facts and omitted if no selected fact matches.

Regression coverage exercises both generated revision loaders and the real
frozen AlcherLogic module: all enabled fodder survives, rune platebody sorts
before rune chainbody, custom aliases and display names resolve, same-name
Dragonhide rows remain distinct, unknown content fails closed, and a 289-only
item never appears in revision 274. Selected food and pickpocket checks cover
Lobster/Bread/Anchovies (12/4/3), Guard level 40, and unknown lookups. The nav
audit uses the same selected generated food facts instead of a handwritten
table.
