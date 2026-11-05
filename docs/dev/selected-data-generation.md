# Generate the selected game data

> **Audience:** contributors who regenerate the game data that ships in the
> `api` crate. You need local checkouts of the 274 and 289 engines and content.
> This text moved here from CONTRIBUTING.md; only relative links changed.

Generate game data and navigation before you build 274bot. Use your own local
engine and content checkouts. The builder does not download inputs or select a
published reference.

Set each source root to the matching local checkout:

```bash
export GAME_DATA_274_ENGINE=/absolute/path/to/274-engine
export GAME_DATA_274_CONTENT=/absolute/path/to/274-content
export GAME_DATA_289_ENGINE=/absolute/path/to/289-engine
export GAME_DATA_289_CONTENT=/absolute/path/to/289-content
export RS2B0T=/absolute/path/to/rs2b0t
```

Run the canonical cross-platform builder from the repository:

```bash
python tools/local-data/build.py
```

The builder runs generator stage 1, builds and runs the isolated `nav-bake`
tool for both revisions, exports the real schema-3 crossings, then runs
generator stage 2 with that projection. Stage 1 writes real game data but no
prerequisite family. Stage 2 overwrites the intermediate outputs and writes
real 289 prerequisite facts.

Build a product only after the builder succeeds. For example:

```bash
cargo build --release -p panel --bin panel-play
```

To rebake navigation after a content change, run `nav-pack` directly. This
command uses the already-generated game data and writes the local 289 pack and
its default sidecars:

```bash
cargo run --locked -p nav-bake --bin nav-pack -- \
  --revision 289 \
  --content "$GAME_DATA_289_CONTENT" \
  --cache "$GAME_DATA_289_ENGINE/data/pack/client" \
  --game-data-dir crates/api/data/game-data \
  --out target/local-data/nav/289/274bot.navpack
```

The complete local navigation payloads live under
`target/local-data/nav/<revision>/`. Regenerate both game data and navigation
with the canonical builder when the engine checkout changes.

## Revision-gated PvP V0 facts

Revision 274 predates PvP V0, so its core omits `duel.rule_buttons`,
`special_weapons`, `item_combat_bonus`, `melee_weapon_tiers`, `duel_zones`,
the PvP V0 `style_spotanims` rows, and the per-item `destroy_death` flag.
`DuelControls::rule_button` reports `None` on 274, which
`crates/api/src/game_data_tests.rs` asserts (`rule_button_is_none_on_revision_274`).
Revision 289 carries every V0 fact. The builder generates each revision from
its matching local engine and content roots. Do not splice facts into an older
generated file.


## Combat facts

The NPC extract declares `combat_facts_version`.
The API refuses older combat data and requests regeneration before it reads required combat fields.
Do not add runtime defaults for missing combat facts.

The extractor reads all six NPC combat levels and the signed combat bonuses.
It also reads attack intervals, weapon damage styles, and style bonuses.
Each wearable item has signed per-style attack/defence bonuses plus strength and range bonuses.
An omitted parameter uses only its declared content default.
The extractor reads NPC stat defaults from the local engine decoder.
Missing or malformed required facts stop generation.
NPC rows also publish `max_dealt` and attack-rate, poison-severity, and multi-combat defaults from `combat.param`.
NPC damage types use the declared `npc_combat.param` default when omitted.
The extractor resolves that code through `combat_damagetypes.constant`.
A missing default or an ambiguous constant stops generation.
Version-1 rows require attack, defence, magic, the signed attack/defence bonus fields, and `max_dealt`; missing keys fail API decoding rather than becoming zero.

## Gathering tick facts

Gathering schema 4 requires a `tick_arm` row for every method.
The extractor checks the reviewed attempt-script families and classifies each
method as continuing, ending, or not armable. It also records the roll signal,
depletion and delay-drop behavior, the native arm delay, and the animation ids
resolved from the selected sequence pack.
Mining and woodcutting keep item-sorted per-tool facts. Mining rates and skill
animations come from the selected tool parameters. The decoder rejects missing
classifications, unknown kinds, and tool rows that are duplicate or incomplete.

Regenerate both revisions when changing this family schema. A revision 274
catalog cannot retain schema 3 when the decoder requires schema 4. These facts
do not enable tick manipulation or change gathering requests by themselves.

## Server-ordered content contracts

The revision 289 core includes a generated `server_ordered` catalog.
The extractor joins content aliases, decoded interface properties, and the selected engine paths.
It refuses missing or ambiguous aliases, changed controls, and incorrect interface ancestry.
The selected font and message animation must prove one banker player page for `bank_open_deposit`.
If that page is unproved, only this recipe is unavailable.
Unproved revisions do not inherit revision 289 facts.

Generic NPC numeric operations require a complete transform closure.
Static transforms, scripted transforms, temporary types, and reversion all participate in that proof.
For a visible temporary type, the closure first includes its possible base types.
It then follows all forward transforms from those types.
An unresolved dynamic transform cannot authorize an NPC operation.
Each action compares the ordered OPNPC/APNPC handler pair, with type, category, then global resolution.
The `_` script subject selects the global handler.
A bare subject selects an NPC type, even when a category has the same name.
Only an underscore prefix selects a category, for example `_saltfish`.
The runtime catalog contains only positive operation rows.
Unknown label callers and unsupported NPC parameters remain conservative analysis limits, not proved dynamic content.

The built-in Gatherer's `KnifeArm` has one narrower fishing exception.
Generation joins known fishing-resource targets to complete ordinary NPC type
configurations and present static OPNPC/APNPC operations.
It excludes Whirlpool, Big fish, and every catalog hazard NPC.
These rows live in `builtin_gatherer_fishing_operations`, separate from the
complete-closure allowlist.
The host grants this authority only to the current built-in Gatherer run.
Every other caller keeps the complete transform-closure rule.
The consumer checks the observed type before each arm and checked ending click.
A continuing request must also match the NPC index, current type, network
identity, and NPC-info packet generation at dispatch.
Ending arms contain no NPC click.
The operator accepts the remaining same-tick transform race for this built-in
consumer. The guardian still owns event handling.

The API selects game facts by supported client revision. It shares each
prepared catalog by revision. Private decoders construct the inner facts.
Public accessors borrow them without exposing constructors or `Deserialize`.
Recipe budgets, deadlines, and recovery limits are reviewed host policy.
Each predicate, outcome, cleanup condition, and predecessor has an explicit `basis`.
`checked` identifies a declaration that the generator checks against selected source.
`host-policy` identifies a host requirement or an unproved engine assumption.
Host-policy conditions do not assert engine behaviour, even when their names describe engine checks.
The inventory-reference, location, held-use, and appearance-handler assumptions remain unproved policy.
Existing source guards still check the content paths, count resume, and modal cleanup.
Each checked condition and predecessor cites the source spans for its check.
`synchronous` means only a checked synchronous server path.
Host-order edges have `basis: "host-policy"` and `synchronous: false`.
This includes the banker pause-resume edge, whose engine handler is not checked.
Loading these facts does not change existing interaction producers or send packets.

Run the extractor tests with the generation checks:

```bash
bun tools/game-data/server-ordered.test.ts
bun tools/game-data/npc-transforms.test.ts
```
