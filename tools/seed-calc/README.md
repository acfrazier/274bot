# Quest seed calculator

`seed-calc` is an offline developer tool. It is a separate `tools/` workspace crate, like `seat-model`.
No shipped executable includes its query interface or calculation code. It does not change product behavior.

The tool reads one JSON query per line. It writes JSON lines to standard output.
It writes one human summary per query to standard error. Query errors produce explicit JSON error records.

## Prepare combat facts

The selected revision-289 tables supply NPC identity, Hitpoints, maximum damage, attack intervals, weapon identity, and risk classification.
They do not supply every NPC roll, item bonus, wear requirement, or food delay.
Generate those facts from the same content tree:

```sh
node tools/seed-calc/extract-facts.ts /path/to/lostcity-289/content /tmp/fight-facts.json
```

The extractor reuses `tools/game-data/extractors/common.ts` and `gathering-content.ts`.
It records content spans and SHA-256 hashes.
It reads NPC rolls, weapons, worn-item bonuses, wear requirements, and ordinary fixed-heal foods.
Rust checks NPC and weapon facts against the selected tables.
It also checks each worn item against its source facts.
The extractor excludes quest-gated weapons and weapons with an extra Strength requirement.

## Fight queries

```json
{"mode":"fight","label":"chompybird:5 wolf","npc":"wolf","facts":"/tmp/fight-facts.json","weapon":"rune_scimitar","worn_items":["rune_full_helm","rune_chainbody","rune_platelegs","rune_kiteshield"],"food":"lobster","food_count":28,"free_slots":28}
{"mode":"fight","label":"three wolves","npc":"wolf","facts":"/tmp/fight-facts.json","weapon":"rune_scimitar","worn_items":["rune_full_helm","rune_chainbody","rune_platelegs","rune_kiteshield"],"food":"lobster","food_count":28,"free_slots":28,"sequential_kills":3}
{"mode":"fight","label":"ranged example","npc":"grip","facts":"/tmp/fight-facts.json","weapon":"yew_shortbow","ammunition":"rune_arrow","worn_items":["blue_dragonhide_body","blue_dragonhide_chaps","blue_dragon_vambraces"],"style":"ranged","spell":null,"food":"lobster","food_count":28,"free_slots":28}
```

The tool reads one JSON query per line and writes one JSON result per line.
It writes a short summary to standard error.
Query errors produce explicit JSON error records.
Selectors accept an item or NPC id, content alias, or exact display name.
An ambiguous name requires an id.
The default weapon is a bronze scimitar.
Set `"weapon": null` to model unarmed combat.
The unarmed model uses zero bonuses, crush damage, and a four-tick attack interval.
It supports accurate, aggressive, and defensive styles.
The weapon is separate from `worn_items`.
Do not list the weapon twice.
The optional `ammunition` and `spell` fields retain query details for fights that the tool does not model.

The calculator uses the operator's seed policy.
Attack, Strength, Defence, Hitpoints, Magic, and Ranged start at level 40.
Ranged fights use Ranged 50 and the specified ranged kit.
The search tests food counts from zero through `food_count`.
Each food item uses one free inventory slot.
At the base-40 stats, it selects the smallest count with death probability at most 1%, then tries 5%.
It raises one of Attack, Strength, Defence, or Hitpoints only if no base-40 count meets 5%.
Single-stat raises are tried in that order at each level, from 41 upward; the smallest level that meets 5% wins.
It uses a combined raise only if no single-stat raise through 99 meets 5%.
The result reports the accepted food count, death bound, policy reason, explored-state maximum, and the best previous candidate.
An absent food selector means the search has no food.
An invalid or unwearable kit returns an input error.

### Probability, food, and timing

Attack and Defence rolls use the content combat bonuses and the worn-item bonuses.
A tie misses.
The exact hit chance is:

- A > D: `1 - (D + 2) / (2 * (A + 1))`.
- A <= D: `A / (2 * (D + 1))`.

The dynamic program stores player Hitpoints, NPC Hitpoints, attack phase, queued damage, food, food cooldown, regeneration time, and kills left.
It builds the reachable state graph, then uses value iteration to calculate win probability and expected ticks.
Zero-damage cycles are handled by value iteration, not by closed-form cycle removal.
Results use IEEE-754 `f64`, not arbitrary-precision rational arithmetic.

The default opening assumes an aggressive NPC ready at tick zero.
The player also acts at tick zero.
Queued player damage lands one tick later.
The NPC attack resolves before food and player actions on the same tick.
A killing hit cancels the queued melee NPC attack.
Set `player_initiates` to true for a passive NPC.
The NPC then first attacks at `1 + floor(attackrate / 2)`, after the content retaliation flinch.
The result reports the selected timing assumption.
It does not include approach time or death animations.

The food decision calls the product's followed-route food guard.
The calculator does not copy the product eat rule.
Food healing and both action delays come from revision-289 content.
Eating adds the content skill delay to the next player action.
The food cooldown follows the content eat delay.
The player's Hitpoints regenerate by one every 100 ticks.

The self-damage timeline applies at most one overdue regeneration event on a non-delayed, non-modal tick. It does not heal during `p_delay` or `p_arrivedelay` (`engine/src/engine/World.ts:687-709`, `engine/src/engine/entity/Player.ts:816-830,945-959`, `engine/src/engine/script/handlers/PlayerOps.ts:359-379`).
`p_delay(0)` also counts: `P_DELAY` sets `delayed` and suspends the script through the following tick (`engine/src/engine/script/handlers/PlayerOps.ts:376-380`).
The rope setup's `forcemove` walk is a chain of `p_delay(0)` calls inside `agility_walk` (`content/scripts/skill_agility/scripts/agility.rs2:18-43`), so every setup-walk tick is delayed and does not regenerate.

The solver carries Hitpoints, food, and regeneration through `sequential_kills`.
Each new fight resets both attack clocks and starts with no recovery time.
The probability covers all requested kills against the same NPC type.
It does not model simultaneous attackers or travel between encounters.

### Not modelled

The tool reports `not_modelled` when:

- The product risk facts mark combat as bespoke.
- The NPC maximum hit is unknown.
- Dragonfire, forced special damage, or protection-counter mechanics apply.
- The NPC uses ranged projectiles or magic.
- The player uses ranged attacks or magic.
- Either participant can inflict poison.
- `npc_count` is not one.
- The exact solver reaches its 8,000,000-state limit or fails to converge.

Ranged accuracy and maximum damage need projectile flight and pending-damage state.
Magic also needs spell-dependent damage, freezing, and stat changes.
Poison needs a calibrated damage schedule.
The tool reports the reason and does not return a survival probability for these cases.

## Self-damage queries

Use `mode: "self_damage"` with a guide and its seed:

```json
{"mode":"self_damage","label":"upass","guide":"/Volumes/dev-scratch/274bot-evidence/QUEST-TERRA/paths/upass.json","seed":"/Volumes/dev-scratch/274bot-evidence/QUEST-TERRA/upass/seed.json","hitpoints":40}
{"mode":"self_damage","label":"mcannon","guide":"/Volumes/dev-scratch/274bot-evidence/QUEST-TERRA/paths/mcannon.json","seed":"/Volumes/dev-scratch/274bot-evidence/QUEST-TERRA/mcannon/seed.json","hitpoints":40}
```

`hitpoints` overrides the seed value. `role` selects a role when a guide has more than one.
`lobster_limit` defaults to 28.

The model reads the guide steps in order.
It recognizes Underground Pass rockslides and rope swings, plus Dwarf Cannon railing repairs.
It follows each step's `max_attempts` or `max_restarts` budget.
It also follows the eight authored rockslide steps in the rope recovery recipe.

The content roll is `floor(low * (99 - level) / 98) + floor(high * (level - 1) / 98) + 1`.
The attempt succeeds when this value is greater than the engine's byte roll from 0 through 255.
The result reports successful roll values per 256 attempts.
Rockslides deal 3 damage after a failed roll.
Rope falls deal `floor(current HP * 15 / 100) + 1`.
Railings deal `min(floor(base HP * 10 / 100), 3)` damage.

The no-food result is today's no-eating death risk.
The live Quester does not eat during these modeled steps, so every food count is conditional: it meets policy C only if the Quester eats at the modeled boundaries.
The hypothetical food rule eats between attempts and steps when Hitpoints are at or below the product eat line.
The rule uses the product food line and revision-289 lobster healing and delays.
It selects the smallest count at 1% death risk, then tries 5%.
It reports the probability of each named budget stop separately from death.
Every merge and every sum visits the states in a fixed order.
Two runs of the same guide and seed therefore store the same floating-point bits.

The model starts the 100-tick Hitpoints timer at phase zero.
It includes the source delays and the rope setup walk from the guide anchor.
Health regenerates only on eligible ticks: not while the player is delayed or has a modal interface.
Missed intervals do not accumulate into multiple heals; one overdue timer event can fire on the next eligible tick.
Engine order resumes scripts and processes queues before normal timers, and the normal timer checks `canAccess` (which rejects delayed or modal players).
The model excludes travel outside these actions and time for the bot to send the next action.
It does not model unrelated guide steps.

## Crossing queries

Provide a reference navpack, route endpoints, and the exact `cross` ids from the guide:

```json
{"mode":"crossing","label":"Family Crest: Musa Point to Brimhaven","navpack":"/path/to/274bot-289-v17.navpack","from":[2956,3147,0],"to":[2772,3230,0],"zones":["scorpion@2865,3156,0","scorpion@2870,3159,0","snake@2875,3164,0","scorpion@2861,3165,0","scorpion@2863,3169,0","scorpion@2862,3180,0"],"lobster_search_limit":19,"route_state":{"worn":[1163,1113,1079,1201,1333],"inventory":{"995":2010,"1265":1,"1755":1,"303":1,"305":1,"309":1,"311":1,"1592":1,"1597":1}}}
```

When `lobster_search_limit` is present, the tool checks the crossing at base 40 with the five-item melee kit worn.
It adds zero through the smaller of the limit and the free inventory slots.
It asks the current product planner and admission code to evaluate each candidate.
The result reports the first admitted lobster count and the product refusal below it.
It also reports the declared-zone result and the ungranted survivability result.
The ungranted result uses no danger grants.
Declared zones permit route planning through those zones; they do not bypass risk.
Both results still require a survivable product risk assessment.

The crossing result is a product admission verdict.
It does not contain a statistical death probability.
It does not replace the product risk replay with the fight solver.
The risk input uses the queried Defence and the worn kit's signed stab, slash, crush, and ranged defence bonuses.
Only the selected anti-dragon shield sets the dragonfire shield flag; a rune kiteshield does not.
Defence also affects route planning through combat-level exemptions.
Unknown facts remain unknown and do not become zero risk.
When `lobster_search_limit` is absent, the tool retains the original level search without food.

The optional `route_state` carries noncombat skills, quest completions, varps, inventory, and worn items.
Use numeric skill, varp, and item ids.
Use the product's quest display names for `completed_quests`.
Inventory counts reduce the number of free food slots.
Inventory items unlock only the product's normal route gates.
They do not grant food, prayer, or danger consent.
Omitted route facts remain absent.

## Tests

Run these commands on the designated build host:

```sh
CROSS_CALC_NAVPACK=/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack BOT_NAV_BUILD=skip cargo test -p seed-calc -- --include-ignored
BOT_NAV_BUILD=skip cargo clippy -p seed-calc --all-targets -- -D warnings
cargo fmt -p seed-calc -- --check
cargo fmt --all -- --check
```

Tests compare hand-computed hit chances with and without armour.
They check one product-selected eat and its exact healing and action delay.
A deterministic Monte Carlo simulation checks the exact food and regeneration solver.
Two additional simulations check the base fight solver.
Fight tests cover the positive-food-only bound, single-stat-first policy, and state search.
Self-damage tests check delay/modal regeneration gating, death before a due heal, no regeneration during the rope setup walk, live guide attempt limits, and bit-identical results from two runs of one model.
The unarmed query test preserves the bronze default and selects unarmed mode with `weapon:null`.
Crossing tests check a product refusal below the first admitted candidate.

## Source formulas

Paths below are relative to the lostcity-289 content or engine roots.

- `content/scripts/skill_combat/scripts/combat.rs2:4-12,49-60`: effective levels, combat rolls, maximum hits, and style bonuses.
- `content/scripts/skill_combat/scripts/player/player_combat_stat.rs2:62-100`: player roll construction.
- `content/scripts/skill_combat/scripts/player/player_combat.rs2:52-85`: hit comparison and damage-type selection.
- `content/scripts/skill_combat/scripts/player/player_melee.rs2:18-28,46-57`: uniform damage, queued damage, and weapon interval.
- `content/scripts/skill_combat/scripts/npc/npc_combat_melee.rs2:2-48`: NPC rolls, uniform damage, intervals, and dead-NPC suppression.
- `content/scripts/skill_combat/scripts/npc/npc_combat.rs2:7-24,315-343,391-428`: retaliation flinch, NPC Defence, and maximum damage cap.
- `content/scripts/skill_combat/configs/melee/*.obj` and `scripts/skill_combat/configs/ranged/*.obj`: weapon and worn-item bonuses recorded in `fight-facts.json`.
- `content/scripts/levelrequire/scripts/tier40.rs2` and `tier50.rs2`: melee and ranged wear requirements recorded in `fight-facts.json`.
- `content/scripts/player/configs/consumption/consume_normal.dbrow`: fixed food healing and action-delay facts.
- `content/scripts/player/scripts/consumption/consume.rs2:114-134`: food cooldown and skill action delay.
- `content/scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2:39-74`: food healing.
- Product `crates/script/src/combat/guard.rs:470-483`, `policy.rs:333-386`, and `schedule.rs:85-87,118-131`: the food decision and scheduled action.
- `content/scripts/login_logout/login.rs2:34-38` and `scripts/player/scripts/stat.rs2:58-60`: the 100-tick Hitpoints regeneration timer.
- `engine/src/engine/World.ts:357-378,665-717`: NPC processing precedes player processing.
- `engine/src/engine/World.ts:687-709` and `engine/src/engine/entity/Player.ts:816-830,945-959`: scripts and queues run before timers; normal timers require `canAccess`, which rejects delayed or modal players.
- `content/scripts/quests/quest_upass/scripts/upass_obstacles.rs2:19-31,93-128`: rockslide and rope rolls, damage, and delays.
- `content/scripts/quests/quest_mcannon/scripts/locs/railings.rs2:64-92`: railing roll, damage, and delays.
- `content/scripts/skill_agility/scripts/agility.rs2:18-43,93-101,123-139`: per-tile movement, exact movement, and forced movement.
- `engine/src/engine/script/handlers/PlayerOps.ts:359-379,591-600`: arrival delay, `p_delay`, and `STAT_RANDOM`.
- `content/scripts/skill_combat/scripts/combat.rs2:14-39,63-71`, `configs/combat.dbrow:140-148`, `content/scripts/player/scripts/equip.rs2:244-285`, and `player_melee.rs2:52-57`: unarmed styles, crush damage, zero worn bonuses, and attack interval.
- Product `crates/host-play/src/admission.rs:419-420`: crossing admission accepts a survivable assessment.
- Product `crates/script/src/combat/risk/replay.rs`: named zone grants do not bypass the survivability bound.
