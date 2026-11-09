# Write a quest guide for the Quester

> **Who this is for:** you want to write a quest guide that the Quester runs. When you finish, you can author a guide file, check its shape, and follow a worked example. To run guides from a folder, see [The built-in scripts](../manual/built-in-scripts.md#quester-run-guides-from-a-folder). To change the runner or the format itself, see [Quest paths for contributors](../dev/quest-paths.md).

A guide is a compiled, restartable description of one quest. The compiler checks its shape and content references before a run. The runner follows the live quest tab and journal as progress proof.

## Quest guide files

A guide is one JSON file with schema 3. It names the quest, the checks the Quester runs before it starts, and the roles, sequences, and steps the runner follows. Unknown fields and unregistered step kinds fail before any run.

- Top level: `$schema`, `schema: 3`, `id`, `display_name`, `kind`, `required`, `tested_stats`, `partner`, optional `prereq`, `quest`, and `roles`.

  `id` is the content quest stem, for example `cook`. `kind` is `Quest` by default. A `Miniquest` role needs its own typed `progress_reader`, and one without it is rejected. `required` holds header requirement ids that gate Start.

  `tested_stats` is optional. The runner reads it only as a warning: when it is `null`, the tested stats warning reads `No qualified stats recorded`. It never blocks Start, so an author may record qualified stats by hand. `partner` stays `null` in every bundled guide. Guide authoring reserves `blackarmgang` and `hero` for Shield of Arrav and Hero's Quest.
- `quest` holds provisioning and eligibility input: `members`, `quest_points`, `requirements`, `items`, `acquire` recipes, `bank`, `coin_float`, `loadouts`, `areas`, `tools`, and `owns_inventory`.

  `requirements` entries name quest-points, skill, quest, item, or members-world gates. `items` entries are `acquirable` with an optional `acquire` recipe, or `must_have`. An optional `from_stage` holds an item out until that stage.

  `bank` is `"nearest"` or an authored tile with a `source`, plus `required: true` only when that bank is necessary. `loadouts` entries map `worn` slots to item aliases and list carried item-alias rows with quantities. `areas` are named walk boxes with a `source`.

  The compiler requires this header. A guide with `members: true` blocks Start on a free-to-play world with `quest requires a members world`.

### Roles and steps

- `roles[]`: `role` (`null` for solo), `progress_binding`, `progress`, an optional owned `progress_reader` step (required for `Miniquest`; invoked initially and after every advancing step), guarded `prelude[]`, and `sequences[]`. `progress` declares quest colours, journal stage rules, flags, and monotonicity. Keep the prelude short.
- `sequences[]`: `stage`, `required`, `terminal`, `recovery_entry`, optional `cross` declarations, optional `order` (`"authored"` default, `"nearest"`, or `"ordered"`), and `steps[]`.
- `steps[]`: `id`, `kind`, `version`, `args`, `skip_if`, and `settle` are required. `comment` is optional. `advances` is required for Explicit handler kinds (`talk`, `interact`, `use_on`, `make`, `combat`, `gather`, `thieve`, `answer_count`, `if_button`, and `x:*`). Default kinds compile as `false` when it is omitted.

### Retry limits

Optional `max_attempts` sets a per-step limit from 1 to 32. Pair it with a nonblank `stop` name. Choose and justify the limit from quest content. Use this pair only in `prelude[]` and `sequences[].steps[]`. The compiler rejects the pair on `quest.acquire` recipe steps and `progress_reader` steps.

The runner counts failed begins, soft poll failures, and settle timeouts. It also parks after 32 consecutive counted failures across budgeted steps, even if another step runs after a failure. A successful settle resets this run-wide count.

Hard poll failures (`Blocked`, `CombatInterrupted`, or `NeedsEvidence`) and user input still stop the guide at once. User input does not spend an attempt. If the per-step limit is spent, the runner reports its named stop. Without the pair, the existing five-failure park limit applies.

After a settle timeout, an advancing step rereads progress before it retries.

Optional `failure_signal` (a predicate document, valid only with `max_attempts` and `stop`) names what the content shows when one attempt ran and failed without changing anything the watchdog tracks, for example a `message` fact on the content's failure text. It is checked on the chat since the step began, when a settle timeout closes an attempt whose step ran to its end (its operation was sent). A timeout with the signal counts as progress for the no-progress watchdog, so `max_attempts` is the only bound on those retries. A timeout without the signal, or from a step that never ran to its end, still counts toward the watchdog's eight-observation park. Cite the content line that prints the signal in the step `comment`. Example: Dwarf Cannon's railing repair uses `{"Fact":{"kind":"message","version":1,"args":{"any":["but fail and cut yourself trying."]}}}` (`railings.rs2:78-91`).

`skip_if` and `settle` use externally tagged predicate documents: `{"All":[...]}`, `{"Any":[...]}`, `{"Not":...}`, and `{"Fact":{"kind":"...","version":1,"args":{...}}}`. Keep this envelope as written.

Step ids must stay unique across recipes and the selected role's prelude, sequences, and `progress_reader`. A solo guide declares exactly one role. Paired guides declare one role per gang.

### Prerequisite switches

The optional header `"prereq": {"quest_start": true}` adds a path-level grant for
quest starts on every walk. Use `"dialogue": true` to grant prerequisite
dialogue steps on every walk.
An omitted or `false` header field adds no path-level grant; it does not
revoke global or script grants.

For one `walk` step, use `allow_prereq_dialogue` or `allow_quest_start` in its
arguments. A per-walk `true` adds a grant; `false` is the request-level
prohibition. An omitted per-walk field uses the global, script, and header
grants.
These switches do not change the guide's explicitly authored quest conversations.

## Danger zones a guide crosses

Declare each danger zone where the guide must enter or cross it.
A declaration is the guide's request to cross danger zones on the walks it covers.
A sequence's `cross` covers every route in that stage.
The sequence field uses stable navigation zone ids, with a `name` and `reason` for each zone.

```json
"cross": [{
  "id": "renegade_knight@2773,3399,0",
  "name": "Keep Le Faye Renegade Knight",
  "reason": "Cross the ground floor to reach Sir Mordred."
}]
```

Walk and combat steps can also declare an `args.cross` array of stable zone ids.
Those requests apply only to that step's routes, including a combat step's first approach.
A prelude step keeps its own requests.
Each route is covered by the union of its step and current stage declarations, without duplicates.
A guide can declare any number of zones.

The compiler rejects unknown ids.
Stage declarations also require unique ids and nonempty names and reasons.
A navigation pack must be available when a guide declares zones.

A Quester walk step cannot use `args.allow_danger_zones`. The compiler rejects
that field. Declare stage requests in the sequence's `cross` array. Use
`args.cross` only for a step-scoped request.

Panel and TUI list stage and step declarations before Start, with their scopes.
The next stage does not retain the previous stage's declarations.

A walk may cross danger zones when the global Danger routing setting is `Always`,
or when the guide requested it for that walk (a stage or step `cross` that covers
it). Nothing else decides this. The global setting has no exception: with it on,
every Quester walk may cross danger zones, declared or not. An old saved Quester
card value does not change it. With the setting off, a walk that no declaration
covers avoids danger zones, and a route whose only path crosses one is refused.

A walk that a declaration covers may cross any danger zone, not only the declared
ones. The food guard is armed for the whole walk when a route uses a declared
crossing. Protection runs only where a step requests it. A permitted walk
proceeds like a walk under the global setting: the survival model's bound is
logged, and the walk's own food guard, protection and escape stay on.

## Steps

Every step `kind` below is version 1. Explicit kinds require `advances`; the rest compile as `false` when it is omitted.

| `kind` | Class | Purpose |
| --- | --- | --- |
| `walk` | Default | Walk to an authored `tile` (`source`, `radius`, optional `cross` danger-zone names, `guard: "protect"`, and per-step teleport or Wilderness overrides). An omitted `radius` means 1; an explicit `0` requires the exact tile, and a blocked or unstandable tile is refused rather than approached. |
| `talk` | Explicit | Selected NPC dialogue with shared answer rules (`prefer`, `choose`, `line_rules`, `strict`). An optional end gap (`gap_ticks`; see Gaps, waits, and combat finish), `continue_only` draining, or an `expect_combat` handoff can follow. |
| `interact` | Explicit | One selected scene, ground-item, named, or held-item operation with optional repetition (`until` inventory-count goal) and shared dialogue handling. |
| `use_on` | Explicit | Held-item operation onto one NPC, loc, item, or exact ground stack, with the same `until` goal and dialogue handling as `interact`. |
| `acquire` | Default | Run a named `quest.acquire` recipe as a restartable cursor. |
| `wait` | Default | Bounded wait for `until`, with `max_ticks`. Optional `park_on_timeout: true` parks immediately at expiry instead of consuming runner retries. |
| `bank` | Default | Authored bank visit. |
| `buy` | Default | Authored shop purchase. |
| `make` | Explicit | An action that makes an item (for example cooking or crafting). |
| `equip` / `unequip` | Default | Wear or remove equipment. |
| `loadout` | Default | Apply a named `quest.loadouts` entry (`loadout`, `at: "quest_bank"` or `"nearest"`, `allow_lower_tier`, `strip`, `exclusive`); see Loadouts and paired guides. `exclusive` requires exactly the listed worn items and cannot be combined with `strip` or `allow_lower_tier`. |
| `setting` | Default | Flip an observed game setting, currently `retaliate` on or off. |
| `combat` | Explicit | Native fight with a tactic (`melee_mode`, `ranged_style`, or mage `spells`), optional `cross`/`guard` walks, looting, and dialogue answers. |
| `partner` | Default | One side of a paired Shield of Arrav / Hero's Quest handoff (a guide-authoring reference; no bundled Beta 2 guide uses it). |
| `gather` | Explicit | One finite Woodcutting, Mining, or Fishing goal (`skill`, one `resource` key or `method` id, `until` inventory goal, optional `anchor`/`radius`). |
| `thieve` | Explicit | One finite pickpocket goal against a selected NPC (`target`, `until` inventory goal, optional `anchor`/`radius`). |
| `answer_count` | Explicit | Answer the numeric count dialog (content `p_countdialog`) with `value`. Waits at most `max_ticks` observed ticks for the dialog, sends one answer, and ends when the dialog is closed. `source` cites the content. |
| `if_button` | Explicit | Press one interface component (`component`, the content pack id) once it and every layer above it are shown. Waits at most `max_ticks` observed ticks, sends one press, and ends on the host's receipt. A `buttontype=pause` component sends the resume press that `p_pausebutton` waits for. `source` cites the content. |
| `x:mm:solve_reinitialisation` | Explicit | Monkey Madness: solve the open 5 by 5 reinitialisation board (`pieces`: the alias prefix of its 24 pieces, `reinitialisation`; `arrival`: the `tile` and `source` where the cutscene after the last piece leaves the player). Open the panel first with an `interact` step. The step reads the board the client holds, plans the slides with the shared sliding-puzzle solver and clicks one piece per observed move, then waits for the cutscene to reach `arrival`. It fails after three clicks that moved nothing, when the board closes unsolved, or when the cutscene does not arrive within 34 ticks. |

### Count dialogs and interface buttons

Content can ask for a number (`p_countdialog`) or wait for a press on its own interface. Neither is a dialogue page, so `talk` does not answer them. Use the two steps in order after the conversation that opens them:

1. A `talk` step whose conversation ends at the prompt. Set `gap_ticks` when the script closes the chat and delays before the prompt (`p_delay(n)` costs n + 1 ticks).
2. `answer_count` with the number to type, or `if_button` with the component to press. Both send one request, and only while the prompt is shown. The step never repeats a request; a failure goes to the runner's bounded retry.
3. Settle on what the content does next (a chat line, an item, a stage), not on the step itself. Both steps are Explicit: declare `advances`.

`max_ticks` is observed game ticks from the step's start. Size it from the content delay before the prompt, and cite it in `source`. Use `talk` with `continue_only` for the chat pages that follow, and use `answer_once` only for menus.

Death Plateau's dice game uses all three: Harold's stake (`death_guard_equiproom.rs2:227-228`), the roll button `death_dice:com_28` (`:46-47`, shown after three `p_delay(2)` at `:88-99`) and the continue button `death_dice:com_30` (`:123`, `p_pausebutton`).

## Checks a guide can use

Combinators are `All`, `Any`, and `Not`, with the envelope shown above. Every fact below is version 1. Only `stage_in`, `flag`, and `quest_colour` read quest progress, and any of them inside `settle` requires `advances: true`.

| Fact | Reads |
| --- | --- |
| `has_item`, `item_count_at_least` | Backpack contents by selected alias (or exact id for count goals). |
| `worn`, `equipment_only` | Worn equipment. |
| `near`, `in_area`, `on_level` | Player tile against an authored tile, named area boxes, or plane. |
| `message`, `message_state` | Chat ring: a line newer than the step's start, or a recoverable set/clear state. |
| `quest_colour`, `stage_in`, `flag` | Quest tab colour, journal stage, or journal flag. |
| `in_combat`, `combat_end` | Combat flag or a finished fight. |
| `npc_present`, `npc_absent`, `loc_present`, `ground_item_near` | Scene entities (ground goals accept an exact `at` tile). |
| `npc_near` | `{ "npc": config, "radius": nonnegative integer }`: a visible NPC of that config is within the inclusive Chebyshev radius of the player on the same level. Reuses NPC-presence matching; unknown until both NPC-list and player-position evidence are observed. |
| `modal_open` | Whether a main modal is posted. |
| `skill_at_least`, `hp_fraction_below`, `prayer_points_at_least` | Observed stats. |
| `bank_known`, `bank_has` | The account's bank memory: whether the bank is known (seen this session or from the saved record; `Unknown` until then), or whether it holds an item. |
| `loadout_ready` | A named loadout is worn and carried. |
| `retaliate` | The observed retaliate setting. |
| `pack_only` | Whether every held pack row is a listed object or an item the guide protects (see Rules of thumb). |
| `members_world` | Whether the bound server's world is a members world (see Rules of thumb). |
| `partner_item_count_at_least` | The paired partner's observed item count. |

## Loadouts and paired guides

A `quest.loadouts` entry names worn items and carried item/qty rows using selected item aliases. Compilation resolves each row once into the display-name rows the bank runner consumes.

A `loadout` step applies one entry at `quest_bank` or `nearest`: `allow_lower_tier` permits lower-tier alternatives, and `strip` removes worn items outside the entry. `exclusive` requires exactly the listed worn items. It cannot be combined with `strip` or `allow_lower_tier`.

`loadout_ready` proves the entry is held. An active recipe's declared inputs can independently request the same items, and active-loadout carry floats join provisioning like coin floats.

#### Paired guides

Shield of Arrav and Hero's Quest declare `partner` in guide authoring: a two-role arrangement with reciprocal handoffs driven by `partner` steps on each account. The guide id must be `blackarmgang` or `hero`. Each account runs its own Quester with reciprocal `partner_account` settings and opposite gangs (see [Quest paths for contributors](../dev/quest-paths.md)). Validation compiles both gang roles. A run activates only the account's effective gang after owned membership is admitted.

## How a run reads a guide

1. **Read progress when needed.** The runner reads quest colour and, for an in-progress quest with `progress.rules`, the journal. Reads are requested at startup, after lifecycle changes or death, on retry, or on an explicit read request. Choosing a step requests one when it needs fresh evidence. A running operation or a successful advancing step can also request a read. Unknown colour or unavailable evidence causes a bounded wait, not a guessed stage.
2. **Resolve the stage.** Journal rules and declared flags resolve progress to a symbolic stage, which must name a sequence. If a completed journal read does not match a sequence, the runner parks with the journal evidence rather than guessing stage zero.
3. **Select a step.** At each boundary, the runner checks `prelude` before the current stage sequence. `True` skips a step, and `Unknown` blocks lower-priority steps until evidence becomes available. An attempted step is not automatically done: `skip_if` must describe an already-achieved condition. The sequence's `order` decides which proven-`False` step starts; see Step order within a stage.
4. **Run the step.** A family plan may drive one or several actions; its run is polled over ticks. Provisioning recipes run their own sequence of steps. Temporary blocking or missing evidence can wait; repeated failures can park the guide.
5. **Read and settle.** When a successful step has `advances: true`, the runner requests a progress read and then evaluates `settle` against current evidence inside the family's settle window. The read's latency does not consume that window. A true settle resets the failure streak and selects from the newly resolved stage. A timeout is recorded as a step failure, not treated as a skip; an advancing step requests a fresh progress read before another attempt.

## Step order within a stage

   - `"authored"` (default): the first one, re-read from the top at every boundary. Every step therefore needs a `skip_if` that becomes true once it is done, or it is selected again.
   - `"nearest"`: the one whose anchor is closest to the player; every step in the sequence must carry an anchor.
   - `"ordered"`: steps run top to bottom through an in-memory cursor. A settled step moves the cursor past itself. A proven-`True` `skip_if` jumps it forward. That jump is committed even when the choice then blocks on an `Unknown` skip or runs out of steps.

     A failed step is retried in place until the failure streak parks. The cursor is not persisted. It starts over whenever the run starts or restarts (Start, Resume, a new session, death) or the stage changes.

### Restarts and random events

A restart replays the sequence from its first step ("safe replay"). The first progress read after the restart resets the sequence, even if that read remains unresolved.

A random event is not a restart. The engine spawns one beside the player about 500 ticks after login. The sources are `content/scripts/login_logout/login.rs2:40` and `content/scripts/macro events/scripts/macro_events.rs2:2-12`. It can therefore arrive during any step.

The Quester keeps its step, progress, and cursor through the event. A journal read of the same stage also keeps the step. The Quester does not classify the event by name.

During a script hold, the random-event handler answers the event with its own interactions. The host drops the script's queued requests. The runner then uses observations to resume an action or detect a teleport.

### Resume interrupted actions

An interrupted Talk, Use-on, Thieve, or walk sends its request again at most once. It first waits one observed tick for an in-flight server effect to arrive.

The action checks its own goal: an open dialogue page, a higher item count, or arrival at the destination. It sends again only if the goal remains unmet and the request was lost or taken over.

A Use-on request dropped before dispatch returns to its step. The step checks its `until` count or product first, then chooses the target again from the current snapshot. The log records one `info` line for each resume.

A Talk whose accepted Talk-to opened no dialogue sends again instead of failing at its first-page deadline. A Talk still approaching its NPC keeps a route that the host still follows.

A dropped, unsent route or a route whose observed-tick budget expired during the hold is planned again once. No new route is needed when the NPC is already in reach.

### Teleports during a hold

A teleport during the hold drops the step in flight. A plane change counts as a teleport. So does a jump between observed frames that running cannot cover at two tiles per tick, plus one tick of slack.

Maze and Mime events teleport the player away and back. The jump therefore appears between frames of the hold, not necessarily between its ends. See `macro_event_maze.rs2:15-18` and `macro_event_mime.rs2:9-11`.

The same rule covers ignored Genie or Mysterious Old Man teleports (`macro_events.rs2:120-137`). It also covers a return to Lumbridge when the original tile is blocked (`macro_events.rs2:235-241`). A short walk away from a hostile event is not a teleport.

After a teleport, progress and the cursor stay. The sequence continues from the same step after dropping its in-flight action. A finished step keeps waiting for its own settle check.

Paired work keeps its step through every event. This includes partner admission, a partner begin, and an active partner step.

The host reports position on every frame, including during the hold. A jump away and back inside one observed tick still counts when those frames show it.

A trip entirely between two frames with the same position cannot be observed. The step then stays, and its action resumes a lost request as described above.

With `settings.random_events` off, the handler neither holds the script nor acts. The Quester keeps its step, progress, and cursor, and logs the event. It does not classify a teleport without a surrounding hold.

The step's own checks handle effects from ignored events, such as attacks, content teleports, or trapped squares.

### Author safe ordered steps

Write every ordered step so repeating it is harmless, and let the last step advance the stage. A sequence that runs off its end without a stage change parks with `no step for stage` after a confirming progress read.

Use `ordered` for stages whose steps leave nothing observable behind, such as Hazeel's five valves or Plague City's clerk → door → Bravek. Authored order would select the first step forever.

## Finish, recipes, and watchdog

6. **Finish.** A selected terminal sequence with no steps completes the run where the guide finished. Completion does not add a bank retreat. The queue logs the next guide being prepared; that guide's preparation handles inventory. Authored `bank` steps still run normally.
7. **Recipes.** Steps within an `acquire` recipe form a cursor: after a step settles, the recipe continues at the next step, skipping only steps whose `skip_if` is proven `True`. Write recipes as restartable paths from missing items to the required inventory; the enclosing quest sequence is selected normally.

   The cursor can pass the last step while the `acquire` step's own `settle` and `skip_if` are still `False` (a burnt pie, a product lost on the way). The recipe then restarts from its first step, and every child `skip_if` is checked again. Only a pass that began a child restarts.

   The `acquire` step's optional `max_restarts` arg (default 8) bounds this; past it the step parks with `<recipe> gave up after <N> attempts: <settle> is still false`. A restart repeats every child whose `skip_if` is false, so guard buys, hand-ins and consumed items with a `skip_if` that holds once they are done.
8. **Watchdog.** At step boundaries, the watchdog observes the stage, player tile, inventory, worn items, and total posted skill XP. Three unchanged observations warn; eight park for no progress. Journal flags, chat, and colour alone do not change its signature, so a flag-only transition is not watchdog progress. The one exception is a settle timeout that shows its step's declared `failure_signal` (see Retry limits): it clears the count.


## How a run gathers items

Provisioning checks the inventory first and plans from the account's bank memory, which the host updates whenever the bank is open; the Quester keeps no bank receipt of its own. Collecting or making an item outside the bank does not change that memory.

Unless `quest.owns_inventory` bypasses preparation, missing required or acquirable items, coin floats, active-loadout carry floats, and tools needed by a gather step may cause a bank check. An item with `from_stage` is ignored until the quest reaches that stage (an Unknown stage keeps it out), so it causes neither an acquisition nor the initial bank scan early.

For a gather step, the Provisioner withdraws one method-usable tool only when none is carried or equipped and the bank has one. This uses the same bank scan, capacity planning, and withdrawal route as recipes.

An observed bank without a compatible tool does not block provisioning; the gather family reports its normal missing-tool failure. `quest.tools` remain preservation hints only, not withdrawal hints or extra requirements: absent entries alone never cause a bank visit. The initial scan and ingredient/tool withdrawals share the open bank session.

### Close only an owned bank

Closing the bank:

- A bank session ends when provisioning is ready, is blocked, or is about to start a recipe. Then the Provisioner closes the bank, and waits in observed game ticks until the bank window is gone. A Path step does not act before that.
- The Provisioner closes only a bank that its own trip opened. It records this only when the bank kernel sent its own open request, and a game frame then shows a bank session that appeared after that request. A bank that was already open when the first trip began stays open.
- The kernel counts a request only after it emitted the request: the stand or booth click, the bank-object click, or the first teller talk or dialogue press. A chest-lid click is not an open request. A teller trip that waits for a banker in the scene, or whose click the tick's input budget refused, has emitted nothing. A bank that opens then is the player's.

### Track the bank session

- The record names the login session and bank session. A relog, or a bank closed and reopened by someone else, voids the record. Nothing is then closed. This also applies during a close and between two trips of one session.

  A new bank session joins the record only when the same account's bank kernel opened it with its own request. This includes a trip that opens the bank again after the earlier session ended.
- If a trip fails, or the run is paused and resumed, the open bank is closed as soon as the host allows input again, and before the journal is read. A restart by the watchdog keeps the record, so the new instance does the same.
- A bank that does not close after the bounded retries is logged and left. Provisioning does not retry without limit.
- An operator Stop sends no close and discards the record.
- An authored `bank` step opens the bank again when it needs it.

### Bank memory and inventory space

Bank memory has three states. A bank never seen (`Unknown`) costs one scan trip. A shortage known only from an earlier login (`Hint`) costs one verifying trip. Only a shortage in a bank seen open this login (`Session`) blocks in place. The Quester keeps no other bank record.

Preparation preserves the guide's record items, tools and floats. There is no unconditional inventory freshen or deposit sweep.

Unrelated rows are banked only when the next withdrawals or acquisition need more slots than the posted inventory has free. Only enough rows are banked to make that room.

Available bank hints are withdrawn only when they fit alongside the active recipe's peak, which may mean banking unrelated rows to make room; otherwise the authored steps acquire them later. A completed guide leaves its remaining inventory in place for the next guide to assess.

An unmeetable speculative peak does not itself park the guide. Preparation falls back to immediate withdrawals while still requiring room for the active acquisition's final item. Later inventory pressure is left to the running family.

Provisioner's bank scan, capacity deposit and withdrawal boundaries are included in the stage's bounded run trace; the terminal `finish` line is emitted even if that trace reaches its event cap.

## Rules of thumb

- **Polarity.** `skip_if` means "this is already done"; `settle` means "I can see it worked". `{"Any":[]}` means never skip; do not write `{"All":[]}`.
- **Self-skipping cleanup.** Step choice restarts at the first step after every settled boundary. A step with an empty settle (`{"All":[]}` on a `bank deposit_all`) therefore runs again unless its `skip_if` turns true.

  Skip a `deposit_all` sweep with this fact: `{"Fact":{"kind":"pack_only","version":1,"args":{"objs":[…same keep list…]}}}`. It is true when every held pack row is a listed object or one of the guide's protected items. That is the same set `deposit_all` always keeps, so the sweep runs only while something is left to deposit.

  None of the listed objects is required, and an unobserved inventory is `Unknown`.
- **Observable settles only.** Use inventory facts (`has_item`, `item_count_at_least`), position (`near`, `in_area`, `on_level`), chat (`message` after the step began; `message_state` for recoverable hints), quest colour (`quest_colour`), and journal progress (`stage_in`, `flag`). A direct journal-progress fact in `settle` requires `advances: true`. A settle that no evidence can make true will time out.
- **`advances`.** Declare whether the server can change quest progress on Explicit kinds. Use `true` for starting the quest, hand-ins, and dialogues that advance the journal; use `false` for purchases, replacing a lost item, or informational dialogue. Default-class kinds may omit the field.
- **Sources and names.** Give each authored destination/anchor a `source` (for example, a content placement, source file and line, or dated live observation). A `near` fact is an observation and has no source. Use content config names, not display names; use a `name` target only when the content pack makes a location ambiguous.
- **Stage keys.** Use `"<quest>:<n>"` when the content varp value is verified; otherwise use a symbolic stage key. A rule's `varp` is a fixture seed and status hint, not a runtime gate.
- **Recipes vs sequences.** Put item acquisition in `quest.acquire` recipes referenced by `acquire` steps. Keep stage sequences focused on that stage's actions.
- **Ordered sequences.** Reach for `"order": "ordered"` only when consecutive steps have no observable "done" evidence. Where evidence exists (an item held, an area reached, a journal flag), prefer `authored` order with a real `skip_if`, because that also recovers correctly after a restart instead of replaying.

## Damage from a step

A step whose own operation calls `damage_self` or queues `damage_player`,
directly or through a procedure, declares `hazard` beside `args`:

```json
"hazard": {
  "of": "none",
  "plus": 7,
  "source": "content/scripts/quests/quest_desertrescue/scripts/quest_desertrescue.rs2:266-271"
}
```

The formula is `max(0, min(cap, floor(stat * mul / div) + plus))`.
`of` selects `none`, `current`, or `base` Hitpoints. `none` means a constant.
`mul` defaults to 0, `div` to 1, and `plus` to 0. The optional cap applies
after the addition. The source must cite the content that deals the damage.
A random range declares its maximum, not its average.

The compiler refuses an empty source, a zero divisor, a zero cap, and a
declaration that cannot deal positive damage at any Hitpoints from 1 to 99.
It also refuses a scaled formula without a multiplier and an invalid constant.
Do not put a hazard on a `walk` or `combat` step. Their own guards handle food.

The Quester prices the hazard from step selection until selection chooses
another step. A new hazard step waits one tick before it begins, so survival
can decide first. Known incoming damage and the hazard add together.
A bite cancels the current step and closes a game page before eating.
The restarted step keeps its hazard and the family's original quantity target.

Without food, the Quester holds until its current Hitpoints clear the dynamic
eat line. It closes pages while it holds. A page still open on the 25th parked
tick causes a typed stop. The recovery log records the bound at the park,
after a committed hit, and after a page closes. That bound applies only without
live attackers or poison. A hazard that cannot clear its eat line at full
Hitpoints stops with `hazard-unsurvivable` before the first operation.

A step without a declaration retains the half-Hitpoints fallback on an
unattributed damage tick. A supply need on that tick is reported, not parked.
The next tick can park if the need remains. The first undeclared hit can kill
the bot before this fallback acts. One warning per selected step names that hit.

A multi-hit loop is not one step event. Mark it as uncovered in the guide's
comment rather than declaring only one hit. Damage from an area or map timer
is also not a step hazard. Cancelling a step does not stop that damage.
Survival phase 2 owns those hazards. Underground Pass remains uncovered.

## The members world fact

- **Members world.** `{"Fact":{"kind":"members_world","version":1,"args":{}}}` is `True` on a members world, `False` on a free-to-play world and `Unknown` only when the profile has no bound server (for example a Direct connection).

  It follows the world type the bound server declares or resolves. The declarers are an explicit flag, the rs2b2t worlds, or the local engine's `world.json` file. It never follows the account: a members account on a free-to-play world still reads `False`.

  It takes no arguments and needs no other evidence, so it may appear in `skip_if`, `settle`, `wait.until` and any other predicate position. Branch with `{"Not":...}` for the free-to-play case. `Unknown` blocks lower-priority steps like any other unknown `skip_if`.

## Dialogue answers

- **Dialogue on `interact`/`use_on`.** One policy for both. Omission is strict Continue-only for observed chat and selected Scroll/Book pages. It drains Continue pages and refuses menus without sending an answer, leaving unrelated main interfaces untouched. No page is required.

  Omission-mode no-page completion waits for acceptance and a fresh idle-player tick. For Loc/Npc/name scene targets, no-page completion also requires post-acceptance movement or primary animation. This applies unless the target was within interaction range at acceptance. A fresh idle tick remains required in either case. Loc/Npc/name interactions require their own accepted dispatch receipt. This boundary covers arrival and primary animation without a fixed delay.

  With omitted dialogue, reaching `until` completes immediately even if a page is open or an optional dialogue driver is active. The step does not adopt a newly observed page. It does not continue draining an existing one. The next owner handles any page that remains.

  `settle_ms` bounds the step even while a page is open and is not extended by dialogue draining; a count first observed after the deadline times out.

  Explicit `"continue"` or the object form requires a page after every accepted round, including every `until` round. No page after the bounded opening wait fails with `expected dialogue did not open`.

  Use `"continue"` when content requires a page. Use the object form with `prefer`/`choose`/`line_rules` when content can open a menu. `"none"` never touches dialogue. A `talk` step that adopts an already-open page (no Talk-to sent) answers only through its own `prefer`/`choose`/`line_rules`, never the default last option.
- **Chat-to-scroll transitions.** An owned conversation can finish on the selected scroll (including the source-proven quest-completion scroll) or book interface after its chat closes. The shared dialogue driver requires a witnessed owned page or an accepted, fresh Continue/Answer receipt; an in-flight advance waits within the existing page bound. An unrelated interface, a refused answer, or a scroll without an owned conversation does not prove dialogue success.

### Repeated menus

Some NPCs reopen the identical menu after every question. By default, `prefer` checks the same entries on every page. Its first matching entry therefore answers the reopened menu repeatedly.

Set `answer_once: true` to skip a `prefer` entry that already answered a page in that conversation. Selection then continues with the next entry. This option works on `talk` args, an `interact`/`use_on` `dialogue` object, and a combat `finish`.

`line_rules` and `choose` do not consume a `prefer` entry. If every matching `prefer` entry already answered, `strict` fails as it does for a missing answer. Without `strict`, the usual fallback applies.

With `answer_once`, author at most 32 entries. A larger list fails compilation with `invalid-dialogue-options`.

Filliman Tarlock reopens the same four-option menu after each of his three questions. "Ok, thanks." ends the conversation (`filliman.rs2:14,255-269,233-234`). The mirror step needs stage `spoken_filliman` (`:276-279`). Answer the repeated menu once and leave:

  ```jsonc
  "args": {
    "npc": "filliman_tarlock_spirit",
    "prefer": ["How long have you been a ghost?", "Ok, thanks."],
    "answer_once": true
  }
  ```

## Gaps, waits, and combat finish

- **Dialogue end gap (`gap_ticks`).** A conversation counts as over 1 quiet tick after the chat is first observed closed. The engine sends the same chat close at a script's end and at a script's own `if_close`. A step whose NPC script closes the chat and `p_delay`s before its next page authors `gap_ticks`: the engine hole, where `p_delay(n)` costs n+1. An authored value must be from 1 to 30; anything else fails to compile with `invalid-dialogue-options`.

  Authors put it on `talk` args or in an `interact`/`use_on` `dialogue` object, and cite the content lines in the step's `comment`. Size it by every route the step's conversation can take in the engine script. Count the stages it can run at, the items it can already carry, its answers, and resumed or restarted steps. Do not size it by only the route the step is named for. The longest reachable hole wins.

  During the hole, an inventory change, a player move, a new game chat line, or an active player animation re-arms the gap, adding `gap_ticks` again. Size it from the longest quiet stretch, not one quiet moment.

  Each page is answered on the tick it is first observed; a choice page whose text changed on the same chat root waits one tick for its option rows.
- **Combat `finish`.** Flat `prefer`/`choose`/`strict`/`line_rules`/`answer_once` fields use the shared dialogue selector, and `max_ticks` measures observed game ticks, not host polls.

`interact.wait_until` optionally gates each new operation round with a predicate.
False or unknown evidence waits without clicking; the fixed `settle_ms` deadline
bounds the wait. With `until` inventory counting, a refused operation drains its
dialogue and retries inside the same step, checking `wait_until` again. The
Knight's Sword portrait search uses this to avoid Search while Sir Vyvin is
adjacent; its preceding 200-tick wait parks immediately if he does not leave.

## Worked example: Cook's Assistant

`source` fields are data values the format requires, not links to open. A `source` says where the tile, item, or step fact comes from: a content placement, a source file and line, or a dated live observation. Editors do not open them. Copy the shape, not the addresses, when you author a new guide.

Comments are annotations, not valid JSON; the real file has none. Changes from the shipped file are marked `← new`.

```jsonc
{
  "$schema": "../path.schema.json",            // ← new: editors validate against the generated schema
  "schema": 3,                                 // ← clean schema 2 → 3 cutover
  "id": "cook",                                // content quest stem (C scripts/quests/quest_cook)
  "display_name": "Cook's Assistant",          // UI only
  "required": [],                              // ids from quest.requirements that gate Start (none)
  "tested_stats": null,                        // optional; read only as a warning, never a gate
  "partner": null,                             // null in every bundled Beta 2 Path
  "quest": {
    "members": false,
    "quest_points": 1,
    "requirements": [],                        // symbolic eligibility requirements; their kinds are typed
    "items": [                                 // provisioning plan input; `acquire` names a recipe below
      { "obj": "pot_flour",   "qty": 1, "kind": "acquirable", "acquire": "acquire:flour", "from_stage": "cook:1" },
      { "obj": "egg",         "qty": 1, "kind": "acquirable", "acquire": "acquire:egg", "from_stage": "cook:1" },
      { "obj": "bucket_milk", "qty": 1, "kind": "acquirable", "acquire": "acquire:milk", "from_stage": "cook:1" }
    ],                                         // `from_stage` (optional stage key) holds an item out until that
                                               // stage: no need and no Unknown-bank scan before it. Order within
                                               // `items` is the acquisition order once every item is due.
    "acquire": {
      "acquire:egg": [
        {
          "id": "take-egg",                    // unique in the whole document
          "kind": "interact", "version": 1,
          "advances": false,                   // ← new: `interact` is Explicit; taking an egg does not touch the quest
          "args": {                            // step.interact.v1: exactly one target key
            "target": { "ground": "egg" },     // ground item by obj config name
            "op": "Take",
            "anchor": { "tile": [3227, 3300, 0], "source": "A defs/cooksassistant.ts:12" },
            "radius": 2,
            "wait_if_missing": true            // wait up to 120 s for the respawn instead of failing
          },
          "skip_if": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "egg" } } },   // "already done"
          "settle":  { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "egg" } } }    // "I can see it worked"
        }
      ],
      "acquire:milk": [
        {
          "id": "take-bucket", "kind": "interact", "version": 1,
          "advances": false,                   // ← new
          "args": {
            "target": { "ground": "bucket_empty" }, "op": "Take",
            "anchor": { "tile": [3225, 3294, 0], "source": "A defs/cooksassistant.ts:13" },
            "radius": 2, "wait_if_missing": true
          },
          "skip_if": { "Any": [                // either holding the bucket or already holding the milk
            { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "bucket_empty" } } },
            { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "bucket_milk" } } }
          ] },
          "settle": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "bucket_empty" } } }
        },
        {
          "id": "milk-cow", "kind": "use_on", "version": 1,
          "advances": false,                   // ← new: `use_on` is Explicit
          "args": {                            // step.use_on.v1: item → exactly one of npc | loc | item
            "item": "bucket_empty",
            "target": { "npc": "cow" },
            "anchor": { "tile": [3255, 3288, 0], "source": "A defs/cooksassistant.ts:14" },
            "radius": 4,
            "product": "bucket_milk"           // the family also settles on product growth within settle_ms (default 20 s)
          },
          "skip_if": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "bucket_milk" } } },
          "settle":  { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "bucket_milk" } } }
        }
      ],
      "acquire:flour": [                       // a cursor: runs top to bottom, each step restartable through its skip_if
        {
          "id": "take-pot", "kind": "interact", "version": 1,
          "advances": false,                   // ← new
          "args": {
            "target": { "ground": "pot_empty" }, "op": "Take",
            "anchor": { "tile": [3208, 3213, 0], "source": "A defs/cooksassistant.ts:16" },
            "radius": 2, "wait_if_missing": true
          },
          "skip_if": { "Any": [
            { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_empty" } } },
            { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_flour" } } }
          ] },
          "settle": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_empty" } } }
        },
        {
          "id": "pick-wheat", "kind": "interact", "version": 1,
          "advances": false,                   // ← new
          "args": {
            "target": { "loc": "wheat" }, "op": "Pick",   // loc by config name; the op must be offered by the loc
            "anchor": { "tile": [3158, 3300, 0], "source": "A defs/cooksassistant.ts:15" },
            "radius": 2
          },
          "skip_if": { "Any": [
            { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "grain" } } },
            { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_flour" } } },
            // message_state = a recoverable hint from the chat ring: the newest `set` or `clear` line wins.
            // "Grain is in the hopper" → no more wheat needed.
            { "Fact": { "kind": "message_state", "version": 1, "args": {
                "set":   ["you put the grain in the hopper", "there is already grain in the hopper"],
                "clear": ["the grain slides down the chute", "the empty hopper"] } } },
            // "Flour is waiting in the bin" → no more wheat needed.
            { "Fact": { "kind": "message_state", "version": 1, "args": {
                "set":   ["the grain slides down the chute", "flour bin downstairs is full", "flour bin downstairs is now full"],
                "clear": ["you fill a pot with", "flour bin is already empty"] } } }
          ] },
          "settle": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "grain" } } }
        },
        {
          "id": "walk-mill-top", "kind": "walk", "version": 1,      // `walk` is Default class: no `advances` key needed
          "args": { "tile": [3166, 3306, 2], "source": "A defs/cooksassistant.ts:17", "radius": 2 },
          "skip_if": { "Any": [
            { "Fact": { "kind": "near", "version": 1, "args": { "tile": [3166, 3306, 2], "radius": 6 } } },
            { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_flour" } } },
            { "Fact": { "kind": "message_state", "version": 1, "args": {
                "set":   ["you put the grain in the hopper", "there is already grain in the hopper"],
                "clear": ["the grain slides down the chute", "the empty hopper"] } } },
            { "Fact": { "kind": "message_state", "version": 1, "args": {
                "set":   ["the grain slides down the chute", "flour bin downstairs is full", "flour bin downstairs is now full"],
                "clear": ["you fill a pot with", "flour bin is already empty"] } } }
          ] },
          "settle": { "Fact": { "kind": "near", "version": 1, "args": { "tile": [3166, 3306, 2], "radius": 2 } } }
        },
        {
          "id": "fill-hopper", "kind": "use_on", "version": 1,
          "advances": false,                   // ← new
          "args": {
            "item": "grain",
            "target": { "loc": "hopper_lumbridge" },
            "anchor": { "tile": [3166, 3306, 2], "source": "A defs/cooksassistant.ts:17,35" },
            "radius": 8,
            "settle_ms": 20000                 // the family's own product/message window; must be ≥ 1
          },
          "skip_if": { "Any": [
            { "Not": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "grain" } } } },  // nothing to put in
            { "Fact": { "kind": "message_state", "version": 1, "args": {
                "set":   ["you put the grain in the hopper", "there is already grain in the hopper"],
                "clear": ["the grain slides down the chute", "the empty hopper"] } } },
            { "Fact": { "kind": "message_state", "version": 1, "args": {
                "set":   ["the grain slides down the chute", "flour bin downstairs is full", "flour bin downstairs is now full"],
                "clear": ["you fill a pot with", "flour bin is already empty"] } } }
          ] },
          "settle": { "Fact": { "kind": "message", "version": 1, "args": {   // a game message newer than the step's start
            "any": ["you put the grain in the hopper", "there is already grain in the hopper"] } } }
        },
        {
          "id": "operate-controls", "kind": "interact", "version": 1,
          "advances": false,                   // ← new
          "args": {
            "target": { "name": "Hopper controls" },   // display-name target: the pack names this loc ambiguously
            "op": "Operate",                            // compiles to name + op, nearest within radius; warns `ambiguous-name`
            "anchor": { "tile": [3166, 3305, 2], "source": "authored approach tile for the hopper controls" },
            "radius": 8,
            "settle_ms": 20000
          },
          "skip_if": { "Any": [
            { "All": [                         // holding grain that is not yet in the hopper → fill first
              { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "grain" } } },
              { "Not": { "Fact": { "kind": "message_state", "version": 1, "args": {
                  "set":   ["you put the grain in the hopper", "there is already grain in the hopper"],
                  "clear": ["the grain slides down the chute", "the empty hopper"] } } } }
            ] },
            { "Fact": { "kind": "message_state", "version": 1, "args": {
                "set":   ["the grain slides down the chute", "flour bin downstairs is full", "flour bin downstairs is now full"],
                "clear": ["you fill a pot with", "flour bin is already empty"] } } }
          ] },
          "settle": { "Fact": { "kind": "message", "version": 1, "args": {
            "any": ["the grain slides down the chute", "flour bin downstairs is full, i should empty"] } } }
        },
        {
          "id": "walk-mill-base", "kind": "walk", "version": 1,
          "args": { "tile": [3166, 3306, 0], "source": "A defs/cooksassistant.ts:18", "radius": 2 },
          "skip_if": { "Fact": { "kind": "near", "version": 1, "args": { "tile": [3166, 3306, 0], "radius": 6 } } },
          "settle":  { "Fact": { "kind": "near", "version": 1, "args": { "tile": [3166, 3306, 0], "radius": 2 } } }
        },
        {
          "id": "empty-bin", "kind": "interact", "version": 1,
          "advances": false,                   // ← new
          "args": { "target": { "name": "Flour bin" }, "op": "Empty", "radius": 8, "settle_ms": 15000 },
          "skip_if": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_flour" } } },
          "settle":  { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_flour" } } }
        }
      ]
    },
    "bank": { "tile": [3092, 3242, 0], "source": "289 observed walkable east approach to Draynor booth 2213" },   // or "nearest"
    "coin_float": 0,
    "loadouts": {},
    "areas": {},
    "tools": ["obj:pot_empty", "obj:grain", "obj:bucket_empty", "obj:egg"],   // kept when provisioning deposits
    "owns_inventory": false
  },
  "roles": [
    {
      "role": null,                            // solo quest
      "progress_binding": "journal:cook",
      "progress": {
        "colour": { "not_started": "cook:0", "in_progress": "cook:1", "complete": "cook:2" },   // tab colour → stage key
        "rules": [],                           // colour-only Path: the journal is never opened
        "flags": [],
        "monotonic": false
      },
      "prelude": [],                           // cross-stage overrides would go here (rare; warns above four)
      "sequences": [
        {
          "stage": "cook:0", "required": [], "terminal": false, "recovery_entry": null,
          "steps": [
            {
              "id": "start", "kind": "talk", "version": 1,
              "advances": true,                // starting the quest changes the tab colour → re-read before settle
              "args": {                        // step.talk.v1
                "npc": "cook",                 // npc by config name; must offer Talk-to
                "anchor": { "tile": [3209, 3215, 0], "source": "A defs/cooksassistant.ts:11" },
                "leash": 6,                    // approach within 6 tiles before opening dialogue
                "prefer": ["What's wrong?", "Yes, I'll help you."]   // option text fragments, first match wins
              },
              "skip_if": { "Any": [] },        // never skip: the stage itself says the quest is not started
              "settle": { "Fact": { "kind": "quest_colour", "version": 1, "args": { "quest": "cook", "is": "in_progress" } } }
              // quest_colour in settle ⇒ `advances` must be true (settle-needs-advance otherwise)
            }
          ]
        },
        {
          "stage": "cook:1", "required": [], "terminal": false, "recovery_entry": null,
          "steps": [                           // re-selected from the top after every settle: first step whose skip_if is False
            {
              "id": "egg", "kind": "acquire", "version": 1,          // `acquire` is Default class
              "args": { "recipe": "acquire:egg" },
              "skip_if": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "egg" } } },
              "settle":  { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "egg" } } }
            },
            {
              "id": "milk", "kind": "acquire", "version": 1,
              "args": { "recipe": "acquire:milk" },
              "skip_if": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "bucket_milk" } } },
              "settle":  { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "bucket_milk" } } }
            },
            {
              "id": "flour", "kind": "acquire", "version": 1,
              "args": { "recipe": "acquire:flour" },
              "skip_if": { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_flour" } } },
              "settle":  { "Fact": { "kind": "has_item", "version": 1, "args": { "obj": "pot_flour" } } }
            },
            {
              "id": "hand-in", "kind": "talk", "version": 1,
              "advances": true,                // the hand-in completes the quest
              "args": {
                "npc": "cook",
                "anchor": { "tile": [3209, 3215, 0], "source": "A defs/cooksassistant.ts:11" },
                "leash": 6,
                "prefer": []                   // no choices: the dialogue is linear
              },
              "skip_if": { "Any": [] },        // reached only when the three acquires above are skipped (items held)
              "settle": { "Fact": { "kind": "quest_colour", "version": 1, "args": { "quest": "cook", "is": "complete" } } }
            }
          ]
        },
        {
          "stage": "cook:2", "required": [], "terminal": true, "recovery_entry": null,
          "steps": []                          // terminal + empty = DONE (legal only on a terminal sequence)
        }
      ]
    }
  ]
}
```

The schema checks document shape, exact per-kind `args`, and which `kind@version` values are registered. It requires `advances` on Explicit handler kinds. It cannot resolve content names, verify offered actions, enforce exactly-one target choices, or establish what the server actually changes. The compiler enforces semantic relations such as direct-progress settles requiring `advances: true` and validates content references. Only a live run can confirm that an `advances` value matches the server's behavior.

Compilation failures keep the guide and step identifiers. Unresolved `obj`, `npc`, and `loc` references include the authored alias in `detail`. The queue message preserves that alias and the failing step id.

To change the runner, the step handlers, or the schema itself, see [Quest paths for contributors](../dev/quest-paths.md).


