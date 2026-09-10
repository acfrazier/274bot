# Generated game-data architecture

Architect: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: architecture only for brief 46 section E.
Not implementation, not LIVE, not a Quester, not a universal planner,
not a server interpreter, not a restore of foreign `gen-spelldb.ts`.

Read once: `AGENTS.md`, `docs/execution.md`, `docs/harness.md`,
`docs/compat/STATE.md`, briefs 43/44/45 and 26/27/38/39/40, plus the
named current seams. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work is this report only.
No product edits, LIVE, fixtures, STATE, matrix, other workers' files,
stash, reset, restore, checkout, merge, remotes, gitlink, or release
actions.

## Verdict

Keep the existing `tools/game-data` pipeline. It already produces
revisioned ordinary JSON, pins engine/content commits, hashes the
decoder closure, and refuses dirty relevant inputs. Consume those
assets with serde once per immutable profile and share the result.
Do not hand-maintain large ID/value tables in Rust. Do not generate
Rust source or catalog `.ts` files.

Proceed in four bounded increments, then stop:

1. Runtime serde load + cache-identity match + one-time JS publication
   for items, then qualified food/pickpocket lookups (brief 43).
2. Additive spell/staff extraction from the same generator for Alcher,
   Superheater, and the sixteen autocast combat spells (brief 27).
3. Additive production recipes (fletch cut/string, smithing bars,
   cooking cook/burn) when those capability cards run (briefs 39/40).
4. An extensible `acquisition` table filled only from those extracted
   families. Schema only; no planner.

Python/PowerShell helpers stay outside the app. Foreign
`tools/combat/gen-spelldb.ts` stays a frozen-catalog artifact, not a
host generator. Design approval is not source acceptance and not
native proof.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| HEAD at report write | `428e8e176e1aa836ad2280b2ce0bee541ee4de37` |
| Game-data product inspected | `bba5179cc55722ec4a28b3f1658fbd442113e074` |
| Item generator + provenance | `c1eda4a6`, `bc452cf9`, `3b53ce2c` |
| Consumption/thieving facts | `91769217`, provenance close `bba5179c` |
| Named client gitlink | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 46 SHA-256 | `4a5a7df764e4c25fc6f1eeafeaa1ce316df2422a5f69397a9714ab60ce7192a9` |
| Brief 43 SHA-256 | `83c918053c0bd0621544d8ee352312018548f5d1d282e6f63eafb750fbcf62f8` |
| Brief 45 SHA-256 | `4af983a63e7e0bf9d0defe71fe33ff574349b032fe36fcff2043955aae9de804` |
| 274 engine / content pins | `4c95f87efe00b068cadbd229d94736626907bd1a` / `000c19997e07206131bcb3c884265840efce416d` |
| 289 engine / content pins | `cc359656b4acd216ca452495874b6beba9a0ac75` / `92649430fcbc83538d8c4367ecb96cee1a67a944` |
| Historical ref (read-only) | `codex/memory-diagnostics` = `f9975102` |
| Kanban card | `t_e72d6cab` |

Current product is `bba5179c`. Concurrent untracked headed receipts,
`docs/compat/STATE.md`, and sibling `docs/harness-integration/01-*.md`
were not used as product. `crates/api/src/content.rs` and the script
shim still ship handwritten tables; generated JSON is not loaded.

`docs/memory/architecture-synthesis.md` on `codex/memory-diagnostics`
is an observation/transport/presentation decision. It does not extract
game facts. The inherited useful pattern is cache-identity binding
(`cache_id` + nav + flags), already copied into game-data provenance.
Do not revive owner-census, heaptrack, or Python collectors to get
tables.

## Current pipeline (authoritative)

Build-time, operator machine, pinned checkouts:

- `tools/game-data/generate.ts` imports each engine's `ObjType` /
  `NpcType` offline, parses selected content `.dbrow` text, writes
  `crates/api/data/game-data/{274,289}.json` plus `manifest.json`.
- `tools/game-data/verify.ts` rehashes live inputs, checks pins, dirty
  gates, unique IDs/aliases, Rune platebody > chainbody cost, distinct
  Dragonhide rows, Lobster=12 / Bread=4 / Anchovies=3, Guard level 40.
- `tools/game-data/generate.test.ts` is a fixture for the text parser
  (repeated rows, tuples, zero-heal stays unqualified).

Schema 3 payload keys: `schema_version`, `revision`, `provenance`,
`items`, `consumption`, `pickpocket`. 274: 3,894 items, 250
consumption, 12 pickpocket groups, SHA-256
`e1a95f255022587528008c7b5690d13b0008511e05c22eb4ccffc71b1cb1c192`.
289: 4,089 items, same fact counts, SHA-256
`baef264dfd146fdc202069bc68e6a144dd0592af8362b1d3f53d27a47a01729c`.
No absolute path or wall-clock is serialized. End users do not need
Node, a server checkout, or the generator.

Runtime today still ignores those files. `api::content::ITEMS` is one
rune_chainbody row. `FOOD_HEALS` is a handwritten display-name table.
`PICKPOCKET_SPOTS` mixes host tiles/leashes with handwritten levels.
`spelldb.js` is empty SETTINGS keys. Isolate load posts
`__rs2b0t_host.content` once from those consts (`script/src/load.rs`
around the prelude), then catalog modules read it. That publication
seam is the right one; the payload is wrong.

## Three worked examples

### Simple 1 — Lobster fixed heal

`content/scripts/player/configs/consumption/consume_normal.dbrow`
group `[heal_12]` lists `data=consumable,lobster` with
`data=stat_heal,hitpoints,12,0`. Generated row (both revisions):

- alias `lobster`, id 379, name `Lobster`
- `source_file=consume_normal.dbrow`, `source_row=heal_12`
- `qualification=fixed_hp_heal`, heal base 12 percent 0

Handwritten `FOOD_HEALS` already says 12. The consumer should take the
generated qualified view, not keep the const because it happens to
match.

### Simple 2 — Guard pickpocket

`pickpocket.dbrow` group `[pickpocket_guard]` lists `guard1`,
`guard2`, `ardougne_guard`, level 40, XP 468, stun 8 ticks / 2 damage,
success 50/240, loot coins 30–30 weight 128. Generated join (274):
NPC ids 9 / 10 / 32, all display name `Guard`, item coins id 995.
Same-name NPCs stay distinct rows. Host `PICKPOCKET_SPOTS` "Guard"
tile `(2661,3306,0)` and leash 19 stay policy. Only `required_thieving`
is a generated fact.

### Conditional — Kebab

`consume.rs2` dispatches `[opheld1,kebab] @player_consume_item(consume_effect_kebab, 2, 3)`.
`kebab.rs2` rolls `random(32)` into five branches (amazing 7+24%,
good 6+14%, some 3+7% if damaged, drain+damage, no-op). There is no
kebab row in `consume_normal.dbrow` or `consume_effects.dbrow`.
Generated consumption has **zero** kebab facts. `consume_effects.rs2`
is hashed but not parsed; `consume.rs2` / `kebab.rs2` are not even
in the dirty set.

Extraction limit: do not invent a kebab heal. Do not flatten the
random table into `FOOD_HEALS`. Label script-dispatched items
`unknown` / `not_fixed_hp_heal` only after an extractor that records
the dispatch without interpreting RuneScript. Until then, coverage
must list kebab as missing. Loadout food detection and `foodHealAmount`
must fail closed, not guess.

A generated conditional that *is* present: `consume_effects.dbrow`
`[attack_consumable_effect]` (`1dose1attack` …) with
`stat_change,attack,3,10` is `not_fixed_hp_heal`. That is the
pattern for potions. 107 of 250 consumption rows are qualified
fixed HP; 143 are explicitly not.

Multi-bite limit, already encoded: Cake shares `[heal_4]` with Bread
(4 HP per consume action). Handwritten Cake=4 is per-bite by accident;
handwritten Bread=5, pies, and pizzas disagree with the server action.
Do not invent whole-item totals.

## Revisioned schema

Keep **one JSON object per revision**. Brief 44's `{274,289}/`
directory is not required. Additive families become new top-level
arrays on the same object. Bump `schema_version` when a field is
removed or a type changes; appending a new array may keep the version
if the runtime treats missing arrays as empty. Current version is 3.

```text
GameData
  schema_version: u32
  revision: u32
  provenance: { engine_commit, content_commit, inputs[], content_inputs[],
                decoder_sources[], cache_identity{cache_id,nav_sha256,flags_sha256} }
  items[]
  consumption[]
  pickpocket[]
  spells[]            # later
  staves[]            # later
  recipes[]           # later, production
  acquisition[]       # later, derived/partial
```

Rules:

- Ordinary JSON, serde in Rust. No generated `.rs`, no per-revision
  JS source in git.
- Every row carries alias + selected-revision id + display name when
  the source has them. Same display name, distinct ids, stay distinct.
- Unknown/absent content is omitted, never filled from the other
  revision.
- Units stay raw server integers (`base`/`percent`, XP, ticks). Label
  them; do not convert to "wiki HP".
- `qualification` is required on any view a script might treat as a
  constant (`fixed_hp_heal` vs `not_fixed_hp_heal`; later
  `complete` / `partial` / `unknown` on acquisition).
- Cache identity is the selected world-capabilities triple, not a
  hostname. Runtime bind refuses a profile whose pack/flags hashes
  disagree.

Do not put this object on `GameSnapshot` or clone it per tick.

## Provenance and dirty-input gates

Keep: pinned HEAD equality, path-scoped dirty porcelain, SHA-256+byte
size of each consumed file, decoder import closure, no machine paths.

Close these holes on the next generator card, not in the consumer:

1. **Members isolation.** `ObjType.load` applies
   `if (!Environment.node.members && config.members)` and mutates
   tradeable/ops/params. `WorldConfig` defaults `members: true` but
   still reads the engine checkout env. Generation must set
   `NODE_MEMBERS=true` (or equivalent) so an operator F2P `.env` cannot
   truncate the pack. Do not serialize those mutated op strings as
   facts. The `members` boolean on the item row is the fact.
2. **Packed DB vs source text.** Facts currently come from `.dbrow`
   text. Packed `data/pack/server/dbrow.dat` exists (274: 135,800
   bytes) and is what the live engine loads via `DbRowType`. Hash it.
   If packed bytes drift from the pinned source parse, fail. Prefer
   the existing text parser while it matches; do not write a second
   RuneScript interpreter. `DbTableType` is optional later, not a
   rewrite target for items/food.
3. **Script-dispatched consumption.** Add `consume.rs2` (and later
   named effect scripts) to `content_inputs` as soon as coverage
   claims "all food". Hashing without parsing is not extraction.
4. **Cache identity source.** Copy from
   `.superpowers/world-capabilities/{274,289}/274bot.navpack.json`
   (or the reviewed equivalent) at generation time rather than
   duplicating literals in `generate.ts`. Runtime already has
   `known-cache-identities.json` for archive hashes; do not fork a
   third copy.

`npc.dat` is now in `provenance.inputs` (`bba5179c`). Keep it.

## Immutable runtime sharing

Load on `ProfileSelection::bind` / `ServerProfile` construction:

- `include_str!` the matching revision JSON into `api`.
- `serde_json::from_str` once. Store `Arc<GameData>` on
  `ServerProfile` beside the existing `Arc<ClientSessionProfile>`.
- Indexes: alias→item, id→item, display-name→[items] (multi),
  qualified food name→heal, pickpocket group/npc-id→level.
- Isolate construction borrows the Arc, posts JSON **once**, never
  again per tick.
- Offline / no selected cache: empty item view, explicit absence.
  Do not load 274 facts into a 289 isolate.

`api` currently has `serde` but no `serde_json` and does not include
the JSON. That is the consumer's first edit. `nav-input-audit` and
`__rs2b0t_food_of` must read the Arc, not `FOOD_HEALS`.

## Static JS module publication

Do not emit `spelldb.ts` into the catalog or shim.

The isolate already evaluates

```text
globalThis.__rs2b0t_host.content = { ... };
```

before the bot module. Keep that as the publication seam:

- `itemdb.js` already reads `host().content.items`. Post the selected
  revision's item rows (alias/id/name/cost is enough for Alcher;
  extra fields may ride along).
- `food.js` already reads `food_heals`. Post **only**
  `qualification==fixed_hp_heal` as `[display_name, base]`. Conditional
  and missing names stay absent so `foodHealAmount` throws `notImpl`.
- `spelldb.js` becomes the same: `SPELL_DB` and `STAFF_RUNES` from
  posted content, empty object if the family is not yet generated.
  Preserve SETTINGS key order from the generated combat-spell names
  once that table exists; until then the stub keys remain a
  catalog-compat surface, not rune authority.
- One-time read in JS is allowed. Do not re-serialize on snapshot.

This is "static module publication": rustyscript modules whose exports
are filled at isolate construction from immutable host data. It is not
a second generated-JS tree in git.

## Unknown and conditional facts

| Kind | Generator | Runtime |
|---|---|---|
| Fixed dbrow constant (lobster, guard level) | emit + `fixed_*` / complete | use |
| Dbrow with percent/stat_change (potions, pizza is still fixed per bite) | emit raw tuples + `not_fixed_*` | do not expose as FOOD_HEALS |
| Script-dispatched (kebab, strange fruit) | omit or later `unknown` stub | fail closed |
| Missing on this revision | omit | fail closed; no 274 fallback |
| Host policy (stands, leashes, loot keep, fire AABB) | never emit as server facts | stay in `api::content` |

`COMMON_BANK_LOOT` substrings and casket id 405 stay host policy even
though kebab/strange fruit names appear there.

## Coverage reporting

Extend `verify.ts` (build-time, not a runtime UI) to write
`docs/compat/evidence/generated-game-data/coverage.json`:

- Enabled catalog settings that read ITEM_DB / food / pickpocket /
  SPELL_DB / STAFF_RUNES / production recipes.
- For each: generated and qualified / generated but unqualified /
  missing / host-policy-only.
- Explicit rows for kebab, Bread/Anchovies mismatches, and Alcher
  ordered items (rune_platebody + rune_chainbody).

A test that only reprints a generated constant is not coverage.
Meaningful checks already exist (plate vs chain cost, Guard 40,
Lobster 12). Add: kebab absent; Bread generated 4 vs handwritten 5;
STAFF_RUNES fire providers once extracted; no cross-revision id
assumption.

## Acquisition-facts layer (not a Quester)

Append later, filled only from extracted families:

```text
AcquisitionFact
  output: { alias, id, name, qty }
  action: cook | smith | fletch_cut | fletch_string | pickpocket | ...
  inputs[] / tools[]     # item aliases
  requirements: { skills: {name: level}, quests: [] }
  sources[]              # npc/loc/item aliases from the same tables
  qualification: complete | partial | unknown
```

Examples that this can absorb without a planner:

- Fletch: `logs` + knife → `unstrung_shortbow` (level 5, XP 50) from
  `cut_logs.dbrow`; `unstrung_shortbow` + bow string → `shortbow`
  from `stringing/bows.dbrow`.
- Cook: `cooking_generic` uncooked→cooked/burnt + level + chances.
- Pickpocket: already have NPC + loot + level.

Do not ingest quest defs, `ToolAcquire.js`, or `AcquireTask`.
`AIOQuester` stays deferred. Locations that are curated tiles
(Catherby range, Ardougne plaza) stay host policy; generated sources
name loc/npc aliases, not walk pins.

Shop transfers (brief 38) need live stock snapshots and item `cost`
(already generated). Do not invent a shop-stock table. Fire lighting
(brief 40) is an interaction + host AABB, not a recipe table.

## Upcoming briefs — what to extract

| Brief | Extract | Do not extract |
|---|---|---|
| 26 loadout | qualified food aliases for `food_of` | worn-slot policy, quantities |
| 27 spells | `magic_combat_spells.dbrow` 16 combat spells (name, level, runes, `continue_by_autocast`); `magic_staff.dbrow` staff→rune; `magic_spells.dbrow` high_alch / superheat rune costs | ssb component ids (verify from selected cache; posted buttons win); `gen-spelldb.ts` |
| 38 shop | item cost (have) | live stock, buy/sell packets |
| 39 production | fletch cut/string, smithing `bar`/`product`/`level` | make-X dialog timing, anvil widget ids |
| 40 fire | nothing mandatory | burn-lane AABB, tinderbox use-on |
| 37 teleport | `magic_spells.dbrow` tele_coord + runes when that card runs | nav to the coord |

Combat-spell source path is
`scripts/skill_combat/configs/magic/magic_combat_spells.dbrow`
(Wind Strike: mind 1, air 1, level 1). Staff source is
`scripts/skill_magic/configs/magic_staff.dbrow` (fire: staff_of_fire,
fire_battlestaff, mystic_fire_staff, lava_battlestaff,
mystic_lava_staff → firerune; lava also in earth). High Alchemy:
nature 1, fire 5, level 55. Superheat: nature 1, fire 4, level 43.
Host helpers `castsAvailable` / `runeWithdrawList` stay Rust policy
on top of those facts.

## Capability inventory

### Built-in (ship, reuse)

- Generator + verifier + schema 3 JSON for items, consumption,
  pickpocket, with pins, dirty gates, cache identity, unique aliases.
- Isolate one-shot `__rs2b0t_host.content` publication.
- `itemdb.js` / `food.js` already read that blob.
- Host policy tables: cow fields, fire plots, cook stands, pickpocket
  tiles/leashes, common-loot predicate, FOOD_OPTIONS names/order.

### External (do not swallow)

- Pinned 274/289 engine+content checkouts on the operator machine.
- Frozen catalog `gen-spelldb.ts` / `src/bot/data/spelldb.ts`.
- `.superpowers` Python/PowerShell launchers and world-capability
  manifests (identity source, not a runtime).
- Memory-campaign capture/controller reports.

### Missing (useful)

- Serde load, Arc on `ServerProfile`, cache-identity refuse.
- Qualified FOOD_HEALS / pickpocket level replacement.
- Spell/staff/production/acquisition arrays.
- Coverage.json including kebab-absent.
- NODE_MEMBERS pin; packed `dbrow.dat` hash; consume.rs2 in the gate.
- Read-only profile diagnostic: schema, revision, item count,
  cache match (optional; not a UI editor).

### Obsolete / do not reintroduce

- Handwritten `ITEMS` after the consumer lands.
- Handwritten heal numbers that disagree with qualified facts.
- Foreign ITEM_DB / gen-spelldb as runtime authority.
- Per-tick world copies of the database.
- Generated Rust structs, a second scenario engine, a Quester, a
  shop-stock dump, a fire "recipe" table, F2P-truncated packs as
  the members-world view.

## Ownership, API, UI seams

### Application (api / script / host-play)

- `api`: `GameData` types, include JSON, indexes, qualified views.
- `host-play`: Arc on `ServerProfile`; bind-time identity check.
- `script`: post content once; thin shims; `food_of` / Alcher ITEM_DB
  / later SPELL_DB read the Arc. No isolate opcode.
- Panel/TUI: no editor. Optional identity line in an existing
  diagnostics/support view. Do not hash 1.4 MiB JSON on the UI thread.

### Shipped CLI

None for end users. Regeneration is not a `274bot` subcommand.

### Build-time

`tools/game-data/generate.ts`, `verify.ts`, fixture test, coverage
receipt. Operator runs tsx against pinned checkouts when pins change.
Byte-identical second generation remains the proof.

### External operator automation

Pin engine/content, keep checkouts clean on the consumed paths, run
generate+verify, commit JSON+manifest. Server-admin / givebank
fixtures stay isolated and must not leak into assets.

## Staged implementation cards

Suggested orch cards after this design. Not created here. No LIVE.

1. **Runtime item consumer (brief 43)** — serde load, Arc share, post
   full selected item rows, Alcher ordered/custom/dragonhide/unknown
   tests. Do not change bank routing or startup. `content.rs` ITEMS
   can shrink to a test helper or go away.
2. **Qualified food + pickpocket consumer** — replace FOOD_HEALS
   numbers and `required_thieving`; keep names/order/stands/leashes.
   Bread=4, Anchovies=3, kebab absent. Same card as 1 if the worker
   already owns those files; otherwise immediately after.
3. **Generator hygiene** — NODE_MEMBERS, hash `dbrow.dat`, hash
   `consume.rs2`, read cache identity from world-capabilities.
   Coverage.json with kebab-missing. No new families.
4. **Spell/staff extraction (brief 27 facts half)** — additive
   `spells` + `staves` from the paths above; verifier checks Wind
   Strike runes and every fire staff in STAFF_RUNES. Runtime helpers
   stay on the spell capability card.
5. **Production recipes (brief 39)** — fletch cut/string + smithing
   rows. BankFletcher mode policy stays script-owned.
6. **Acquisition schema** — empty/partial table filled from 4–5.
   No Quester card.

Cards 1–2 are the compatibility-release wiring. Card 3 can ride with
the next generator family. Card 6 is the first cut if scope slips.

Hotspot: `crates/api/src/content.rs`, `crates/script/src/shim/mod.rs`,
`crates/script/src/load.rs`. The item consumer and food consumer
should be one worker if they land together; do not pile spell helpers
onto those files in the same patch.

## Risks

- **Silent 274 fallback** for unknown 289 aliases. Forbidden.
- **Kebab/pie/pizza treated as FOOD_HEALS.** Fail closed / per-action
  only.
- **F2P Environment** mutating decoder output during generate.
- **Packed dbrow vs source drift** after a content pin that was not
  packed.
- **Posting 4k items every tick or on every snapshot.** Once per
  isolate only.
- **Restoring gen-spelldb.ts** because Superheater imports
  STAFF_RUNES. Extract JSON; keep thin JS.
- **Quester creep** via acquisition. Schema may exist; planner may
  not.
- **Hardcoded cache identity** drifting from world-capabilities.
- **Shared-file hotspot** with the active item-metadata worker
  (`t_f1572cf0`) and startup worker. This report does not edit them.

## Focused validation (no LIVE in this design)

Already green at `bba5179c`: generate twice, verify.ts, parser
fixture, plate/chain, Dragonhide, Lobster/Bread/Anchovies, Guard 40.

Add with the consumer, not here:

- AlcherLogic on host-supplied metadata: both requested items, sort
  richest first, custom alias, distinct dragonhide ids, unknown
  absent, no cross-profile leak.
- `foodHealAmount('Bread')==4`, `'Anchovies'==3`, `'Kebab'` throws.
- Guard required_thieving from generated group, tile still 2661,3306.
- Bind 289 with 274 JSON hidden: 289-only ids remain 289.

Do not run LIVE, headed catalog, or regenerate against dirty engines
to accept this design. Later live ordered-Alcher is root's.

## What belongs where (summary)

| Need | Place |
|---|---|
| Item/food/spell/recipe facts | Build-time JSON → serde Arc on profile |
| Dirty/pin/hash gates | Build-time `tools/game-data` |
| Cache identity match | Application bind |
| JS ITEM_DB / SPELL_DB / food heals | One-shot host content blob |
| Stands, leashes, loot keep, fire AABB | Application host policy |
| castsAvailable / runeWithdrawList | Application Rust helpers |
| Live shop stock / widgets | Snapshot, not generated tables |
| Coverage of enabled settings | Build-time verify receipt |
| Quester / AcquireTask / gen-spelldb | Stay out |
| Pin checkouts, run tsx | External operator |

## Suggested stopping point

Stop when enabled catalog scripts read generated item metadata and
qualified food/thieving through the existing isolate publication
seam, with kebab still honestly absent, and when spell/staff facts
needed by Alcher/Superheater are generated the same way. That is
enough for this compatibility release.

Do not wait for a Quester, packed-DbRow rewrite, shop-stock dump,
fire recipes, a user-facing regenerate CLI, or in-app table editor.
Do not restart the memory campaign to get there.
