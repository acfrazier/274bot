# Quest seed calculator

`seed-calc` is an offline developer tool. It is a separate `tools/` workspace crate, like `seat-model`.
No shipped executable includes its query interface or calculation code. It does not change product behavior.

The tool reads one JSON query per line. It writes JSON lines to standard output.
It writes one human summary per query to standard error. Query errors produce explicit JSON error records.

## Prepare fight facts

The product's selected revision-289 loaders supply NPC identity, Hitpoints, maximum damage, attack intervals, weapon identity, and risk classification.
Those loaders do not expose NPC Attack, Defence, or equipment accuracy bonuses.
Generate only the missing roll data from the same content tree:

```sh
node tools/seed-calc/extract-facts.ts /path/to/lostcity-289/content /tmp/fight-facts.json
```

The extractor reuses `tools/game-data/extractors/common.ts` and `gathering-content.ts`.
It joins the existing selected NPC and weapon tables. It records content spans and SHA-256 hashes.
Rust checks overlapping NPC facts, weapon identity, strength bonus, style, and interval against the product loaders.
A mismatch refuses the query. Source defaults come from `combat.param`.
The NPC stat default is one, from `engine/src/cache/config/NpcType.ts`.
The extractor excludes quest-gated weapons and weapons with an additional Strength requirement.

## Fight queries

```json
{"mode":"fight","npc":95,"facts":"/tmp/fight-facts.json","candidate":{"attack":40,"strength":40,"defence":40,"hitpoints":40}}
{"mode":"fight","npc":"skeleton_unarmed","facts":"/tmp/fight-facts.json","weapon":"iron_mace","style":"accurate"}
{"mode":"fight","npc":95,"facts":"/tmp/fight-facts.json","sequential_kills":3}
```

Run queries from a file or standard input:

```sh
BOT_NAV_BUILD=skip cargo run -p seed-calc -- queries.jsonl
```

NPC and weapon selectors accept an integer id, config alias, or exact display name.
An ambiguous display name requires an id. The default weapon is a bronze scimitar.
Only that weapon is worn. No armor, food, potions, prayer, poison, or special attacks apply.
Supported player styles are `accurate`, `aggressive`, `defensive`, and `controlled`, when the weapon offers them.
Where two buttons share a semantic style, the tool uses the first button in content order.

The search tests every legal level in ascending order. It does not assume monotonic probabilities.
The policy is Attack = Strength = Defence = L, with Hitpoints = max(10, L).
The result includes the first seed with death probability at most 0.01 and the immediately preceding tested level.
An optional candidate uses its supplied levels. Weapon requirements still apply.
No accepted level through 99 produces `no_seed_within_policy`, not a fabricated seed.

### Probability and timing

Attack roll A and Defence roll D are inclusive maxima. A tie misses.
The exact hit chance is:

- A > D: `1 - (D + 2) / (2 * (A + 1))`.
- A <= D: `A / (2 * (D + 1))`.

A successful hit draws damage uniformly from zero through the maximum, inclusive.
The dynamic program stores both remaining Hitpoints and the event phase.
Its event cycle covers the least common multiple of both attack intervals.
Positive damage moves to a smaller Hitpoints state. Zero damage moves to the next phase.
The solver eliminates the complete zero-damage cycle analytically. It has no time cutoff or probability-tail truncation.
Results use IEEE-754 `f64`, not arbitrary-precision rational arithmetic.
Expected ticks measure time until the kill or player death, whichever occurs first.

The default opening assumes an aggressive NPC ready at tick zero.
The player swings at tick zero. Queued melee damage reaches the NPC at tick one.
Later player damage follows the weapon interval. NPC damage follows its attack interval.
Queued player damage precedes an NPC attack on the same tick. A killing hit cancels that melee NPC attack.
Set `player_initiates` to true for a passive NPC.
Player damage still lands at tick one. The NPC first attacks at `1 + floor(attackrate / 2)`, after the retaliation flinch.
The result reports the selected timing assumption. It does not include approach time or death animations.

The model holds the input stats fixed. It excludes regeneration, skill-level changes during combat, movement, missed input, and other actors.
These are conditional combat probabilities, not a guarantee for an arbitrary live engagement.

Set `sequential_kills` for consecutive fights against the same NPC type.
The solver carries remaining Hitpoints between kills. It adds no healing or regeneration.
Each new encounter resets both attack clocks to the selected opening, with zero recovery time.
The reported probability covers all required kills. The expected length ends at the final kill or the player's death.
This mode does not model simultaneous attackers or the travel between encounters.

### Not modelled

The tool reports `not_modelled` for these cases:

- NPC risk facts flag bespoke combat or unknown maximum damage.
- Dragonfire, forced special damage, or protection-counter mechanics apply.
- NPC attacks use ranged projectiles or magic spells.
- Either participant can inflict poison.
- `npc_count` is not one.

Ranged accuracy and maximum damage use simple formulas, but flight time and posthumous hits add pending-damage state.
Magic also includes spell-dependent damage, freezing, and stat changes. These cases need calibration.
The poison field states whether poison can change the result. The tool never silently removes poison damage.

## Crossing queries

Provide a reference navpack, route endpoints, and the stage's declared `cross` ids:

```json
{"mode":"crossing","navpack":"/path/to/274bot-289-v17.navpack","from":[2772,3230,0],"to":[2956,3147,0],"zones":["scorpion@2865,3156,0","scorpion@2870,3159,0","snake@2875,3164,0","scorpion@2861,3165,0","scorpion@2863,3169,0","scorpion@2862,3180,0"]}
```

The calculator uses the product planner, risk replay, and host admission.
It does not copy crossing geometry, damage costs, survival thresholds, or admission rules.
The output records route zones and two distinct product decisions:

- `ungranted_survivability` is the campaign seed floor. Routing permits the declared zones, but admission receives no grants.
  Product admission must therefore accept the survival assessment. The result includes its refusal immediately below the floor.
- `declared_cross_admission` is actual admission with the stage's named consent.
  Current product code can accept a fully granted crossing even when the survival assessment refuses it.
  This admission is not a survival proof. The calculator reports it but does not use it as the seed minimum.

Both calculations use no food or prayer. Defence follows L and Hitpoints follow max(10, L).
Attack, Strength, Ranged, Magic, and Prayer stay at one.
Each candidate uses its actual engine-derived combat level, including natural zone exemptions from those required stats.
The product risk input does not use Defence directly. Defence can affect planning through combat-level exemptions.
A product refusal remains a refusal. Unknown facts never become zero risk in the ungranted calculation.
The crossing result is an admission result, not a statistical death probability.

## Tests

Run these commands on the designated build host:

```sh
BOT_NAV_BUILD=skip cargo test -p seed-calc
BOT_NAV_BUILD=skip cargo clippy -p seed-calc --all-targets -- -D warnings
cargo fmt --all -- --check
```

The tests enumerate inclusive hit-roll pairs and check hand-computed cases on both sides of A = D.
Two independent damage simulations use 200,000 deterministic Monte Carlo trials each.
Their tolerances are 0.006 absolute win probability and 0.25 expected tick.
A separate analytic case checks zero-damage cycles and melee tie priority.
Crossing tests call the product admission for a refusal below the computed floor and acceptance at that floor.

## Source formulas

Paths below are relative to the lostcity-289 content or engine roots.

- `content/scripts/skill_combat/scripts/combat.rs2:4-12,49-60`: effective level, combat roll, maximum hit, and style bonuses.
- `content/scripts/skill_combat/scripts/player/player_combat_stat.rs2:62-100`: player roll construction.
- `content/scripts/skill_combat/scripts/player/player_combat.rs2:52-85`: hit comparison and damage-type selection.
- `content/scripts/skill_combat/scripts/player/player_melee.rs2:18-28,46-57`: uniform damage, NPC queue, and weapon interval.
- `content/scripts/skill_combat/scripts/npc/npc_combat_melee.rs2:2-48`: NPC +9, rolls, uniform damage, interval, and dead-NPC suppression.
- `content/scripts/skill_combat/scripts/npc/npc_combat.rs2:7-24,315-343,391-428`: retaliation flinch, NPC Defence, and maximum damage cap.
- `content/scripts/skill_combat/scripts/npc/npc_combat_ranged.rs2:1-54`: ranged rolls and projectile-delayed damage.
- `content/scripts/skill_combat/scripts/npc/npc_combat_magic.rs2:17-43,91-120`: spell-dependent effects and delayed damage.
- `engine/src/engine/script/handlers/NumberOps.ts:37-40`: inclusive random integer generation.
- `engine/src/engine/World.ts:357-378,665-717`: NPC processing precedes player processing.
- `engine/src/engine/entity/Npc.ts:179-181,561-584`: NPC queues run before NPC interactions.
- Product `crates/script/src/combat/select.rs:96-229`: existing `select::facts::npc_max_hit` loader.
- Product `crates/script/src/combat/risk/facts.rs:102-173`: risk classification and unknown facts.
- Product `crates/script/src/combat/risk/replay.rs`: crossing assessment.
- Product `crates/host-play/src/admission.rs:330-407`: assessment and public `permits`, which calls `permits_view`.
