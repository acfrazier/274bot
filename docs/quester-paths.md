# Quester Path authoring guide

A Path is a compiled, restartable description of one quest. The compiler validates its typed document structure and content references before a run; the runner uses the live quest tab and journal as progress evidence. This guide describes the Path format and authoring workflow.

## Migrating a Path to schema 3

For Paths authored against schema 2, make this clean cutover before adding new work:

1. Change the top-level `"schema"` value from `2` to `3`.
2. Add `"$schema": "../path.schema.json"` to every file in `crates/script/paths/289/` (or the corresponding relative path in another Path directory).
3. Add `"advances": true` or `"advances": false` to every Explicit step kind: `talk`, `interact`, `use_on`, `make`, `combat`, `gather`, `thieve`, and every `x:*` quest-specific handler. Use `true` when the server changes quest progress; use `false` when it does not. Default-class kinds can omit the key, which compiles as `false`.

A direct progress fact (`stage_in`, `flag`, or `quest_colour`) anywhere inside a step's `settle` requires `advances: true`. A journal count used as an item quantity (`qty: { "progress": ... }`) is inventory evidence, not a direct progress fact, and does not require it. The schema does not accept version 2; do not mix schemas in a release. Validate the shape with the pinned AJV command in §4.6 and compile every indexed Path with `cargo test -p script bundled_paths_decode_and_compile` before asking for a live run.

## Loading draft Paths without rebuilding

Open **Nav config** in panel, or **settings** (`o` on Overview) in TUI. Enable
**Load quest Paths from folder** and choose a folder. The setting is off by
default. Changing it refreshes the picker. A saved enabled folder reloads
automatically when selected game data is ready at startup. The default is
`<profile data directory>/quester/paths/289`; a checkout's
`crates/script/paths/289` also works. These documents are for revision **289**.
The default folder is not created automatically.

Put each Path in `<folder>/<id>.json`, with the same `id` inside the document.
An optional `index.json` uses the shipped index's schema:

```json
{"schema":1,"paths":[{"id":"cook","file":"cook.json"},{"id":"my-draft","file":"my-draft.json"}]}
```

Bundled rows retain their shipped order. A valid folder document replaces the
bundled document with the same id; new ids are appended as **draft** rows, in
folder-index order when an index is present, then filename order for unindexed
files. Picker labels and running status distinguish **folder** overrides and
**draft** Paths from bundled ones. The active Path's source, SHA-256 digest,
and initial step comment are written to the log when it starts. Folder status
also shows the current step comment in the existing script chrome.

Use **Reload Paths** to reread and validate the folder without starting a quest.
Every Quester Start also rereads it. Stop/Start to run edits; Reload does not
replace a Path that is already running. Delete an override and Reload (or Start)
to restore the bundled document. Turning the setting off ignores the folder
entirely.

Files are limited to 1 MiB each. The loader sorts valid folder filenames and
loads at most the first 256 in filename order; later files are omitted.
Validation reports the filename, step, error code, and detail. An invalid
override falls back to its bundled Path; an invalid draft remains visible but
unavailable, with the validation error as its reason. Other documents still
load. Shipped server-unavailable quests retain their server-content restriction.

Symlinked JSON files are followed; the opened target must be a regular file
within the size limit. The report is capped at 32 diagnostic lines and ends
with an `N more` summary when details are omitted. The TUI shows only the
summary; per-file details are available in Logs and Nav config.

Before asking for a live run, use the pinned AJV commands in §4.6 for document
shape and `cargo test -p script bundled_paths_decode_and_compile` for bundled
selected-data compilation. Every folder document within the 256-file cap is
decoded and compiled on every Reload and Start, including overrides to bundled
ids. JSON Schema validation alone cannot resolve content aliases or validate
runtime handler semantics.
For a paired document, validation compiles each declared gang role before making
the Path available. Every role must compile; a failing role rejects the whole
document, and its gang is named alongside the original per-file compile error.
Reload applies the same validation to edited roles. Runtime activation still
chooses only the account's effective gang after owned membership admission.
Validation and content-family preparation run on the existing preparation worker.
Folder gather steps, acquisition recipes, and typed progress readers use the same
selected gathering catalog as bundled Paths.

---

## 4. Authoring guide

### 4.1 Questionable → Path

| Questionable concept | Path concept | Notes |
| --- | --- | --- |
| `QuestRoot` | `PathDocument { $schema, schema, id, display_name, required, tested_stats, partner, quest, roles[] }` | Source ownership belongs in git history. A Path ships when it is indexed and embedded by id. `roles[]` supports partner quest declarations; solo Paths use `role: null`. |
| `QuestSequence { Sequence, Steps[] }` | `SequenceDocument { stage, required, terminal, recovery_entry, order?, steps[] }` | `stage` is a symbolic key such as `"cook:1"`, not an ordered number. Use one sequence per stage; `terminal: true` with no steps marks completion. `order` picks how the next step is chosen (§4.3 item 3). |
| Flat `QuestStep` with an `InteractionType` and optional fields | `StepDocument { id, kind, version, args, comment?, advances?, skip_if, settle }` | Each registered `kind@version` has its own typed `args` schema. Unknown fields are rejected. |
| `InteractionType` values such as `WalkTo`, `Interact`, `UseItem`, `AcceptQuest`, `CompleteQuest` | `kind` values such as `walk`, `interact`, `use_on`, `talk` | Accepting or completing a quest is an ordinary `talk` step with `advances: true`. |
| `DataId`, `Position`, `TerritoryId`, `StopDistance` | Config names such as `args.npc`, `args.target.loc`, `args.item`; sourced `anchor.tile`; `radius` or `leash` | Targets use content config names, not numeric ids. `bank.keep_ids` is an explicit raw-id exception; `tested_stats[].skill` is generated by a live PASS. Authored destination tiles carry a `source`; observed `near.tile` facts do not. |
| Dialogue and point-menu choices | `talk.prefer[]` or `talk.choose` | `prefer` matches text fragments in order; `choose` is a 1-based menu index. |
| Step skip conditions | `skip_if`: `All` / `Any` / `Not` over `Fact` | Only proven `True` skips. `Unknown` blocks selection; it is never treated as `False`. `{"Any":[]}` means never skip; `{"All":[]}` is rejected as always-skipped. |
| Completion flags or QuestWork conditions | `settle` | `settle` is an observable predicate checked after the step's machine finishes. Direct progress facts require `advances: true`; reading a journal count as an item quantity does not. |
| Required QuestWork values | Sequence membership plus `skip_if` | The stage selects the sequence; use `Not(stage_in/flag)` in `skip_if` for sub-stage branching. |
| `PickUpQuestId`, `TurnInQuestId`, `NextQuestId` | No Path fields | The quest itself is the Path; queue order is in `paths/289/index.json`. |
| Disabled step | Remove the step | Do not leave inert steps in the document. |
| Comment | `comment` | Optional human-readable note; it does not affect execution. |
| Per-quest hand-written schema | `crates/script/paths/path.schema.json` | Generated from the Rust document and handler types; each Path points to it with `$schema`. |
| Controller cursor | Runner stage and step-boundary selection | The runner derives the stage from quest colour and journal evidence, then selects the first eligible step at each boundary. The cursor is diagnostic, not a durable completion marker. |

### 4.2 Document anatomy (schema 3)

- Top level: `$schema`, `schema: 3`, `id` (the content quest stem, e.g. `cook`), `display_name`, `required` (header requirement ids that gate Start), `tested_stats` (`null` until a live PASS writes it), `partner`, `quest`, and `roles`.
- `quest`: provisioning and eligibility input: `members`, `quest_points`, `requirements[]`, `items[]`, `acquire` recipes, `bank`, `coin_float`, `loadouts`, `areas`, `tools`, and `owns_inventory`. The compiler requires this header.
- `roles[]`: `role` (`null` for solo), `progress_binding`, `progress`, guarded `prelude[]`, and `sequences[]`. `progress` declares quest colours, journal stage rules, flags, and monotonicity. Keep the prelude short.
- `sequences[]`: `stage`, `required`, `terminal`, `recovery_entry`, optional `order` (`"authored"` default, `"nearest"`, or `"ordered"`), and `steps[]`.
- `steps[]`: `id`, `kind`, `version`, `args`, `skip_if`, and `settle` are required. `comment` is optional. `advances` is required for Explicit handler kinds (`talk`, `interact`, `use_on`, `make`, `combat`, `gather`, `thieve`, and `x:*`); Default kinds compile as `false` when it is omitted.

`skip_if` and `settle` use externally tagged predicate documents: `{"All":[...]}`, `{"Any":[...]}`, `{"Not":...}`, and `{"Fact":{"kind":"...","version":1,"args":{...}}}`. Keep this envelope as written.

Step ids must remain unique across recipes and the selected role's prelude, sequences, and `progress_reader`. Solo Paths compile the first declared role. Paired Paths compile the role chosen by effective gang after owned membership admission.

When adding a handler in Rust, register it with `step!` or `fact!`. The row names one `Args` type, which supplies both compiler decoding and the generated schema; do not maintain a separate argument-field list.

### 4.3 How the runner executes a Path

1. **Read progress when needed.** The runner reads quest colour and, for an in-progress quest with `progress.rules`, the journal. Reads are requested at startup, after lifecycle changes or death, on retry or an explicit read request, when selection needs fresh evidence, when a running operation asks for a progress read, and after a successful step marked `advances: true`. Unknown colour or unavailable evidence causes a bounded wait, not a guessed stage.
2. **Resolve the stage.** Journal rules and declared flags resolve progress to a symbolic stage, which must name a sequence. If a completed journal read does not match a sequence, the runner parks with the journal evidence rather than guessing stage zero.
3. **Select a step.** At each boundary, the runner checks `prelude` before the current stage sequence. `True` skips a step, and `Unknown` blocks lower-priority steps until evidence becomes available. An attempted step is not automatically done: `skip_if` must describe an already-achieved condition. The sequence's `order` decides which proven-`False` step starts:
   - `"authored"` (default): the first one, re-read from the top at every boundary. Every step therefore needs a `skip_if` that becomes true once it is done, or it is selected again.
   - `"nearest"`: the one whose anchor is closest to the player; every step in the sequence must carry an anchor.
   - `"ordered"`: steps run top to bottom through an in-memory cursor. A settled step moves the cursor past itself; a proven-`True` `skip_if` jumps it forward; a failed step is retried in place until the failure streak parks. The cursor is not persisted: it starts over whenever the run starts or restarts (Start, Resume, a new session, death, a random event) or the stage changes, so a restart replays the sequence from its first step ("safe replay"). Write every ordered step so repeating it is harmless, and let the last step advance the stage: a sequence that runs off its end without a stage change parks with `no step for stage` after a confirming progress read. Use it for the stages whose steps leave nothing observable behind (Hazeel's five valves, Plague City's clerk → door → Bravek), where `authored` order would select the first step forever.
4. **Run the step.** A family plan may drive one or several actions; its run is polled over ticks. Provisioning recipes run their own sequence of steps. Temporary blocking or missing evidence can wait; repeated failures can park the Path.
5. **Read and settle.** When a successful step has `advances: true`, the runner requests a progress read and then evaluates `settle` against current evidence inside the family's settle window. The read's latency does not consume that window. A true settle resets the failure streak and selects from the newly resolved stage. A timeout is recorded as a step failure; it is not treated as a skip.
6. **Finish.** A selected terminal sequence with no steps completes the run where the Path finished. Completion does not add a bank retreat. The queue logs the next Path being prepared; that Path's preparation handles inventory. Authored `bank` steps still run normally.
7. **Recipes.** Steps within an `acquire` recipe form a cursor: after a step settles, the recipe continues at the next step, skipping only steps whose `skip_if` is proven `True`. Write recipes as restartable paths from missing items to the required inventory; the enclosing quest sequence is selected normally.
8. **Watchdog.** At step boundaries, the watchdog observes the stage, player tile, inventory, worn items, and total posted skill XP. Three unchanged observations warn; eight park for no progress. Journal flags, chat, and colour alone do not change its signature, so a flag-only transition is not watchdog progress.

Provisioning checks the inventory first and remembers the last complete bank receipt for the active Path. Collecting or making an item outside the bank does not erase that memory; a recipe's bank actions publish their updated receipts. Missing required or acquirable items, coin floats and active-loadout carry floats may cause an initial bank check; held supplies skip it. An item with `from_stage` is ignored until the quest reaches that stage (an Unknown stage keeps it out), so it causes neither an acquisition nor the initial bank scan early. `quest.tools` are preservation hints only, not withdrawal hints or extra requirements: absent tools alone never cause a bank visit. Active recipe inputs can independently request those items. The initial scan and ingredient withdrawals share the open bank session.

Preparation preserves the Path's record items, tools and floats. There is no unconditional inventory freshen or deposit sweep: unrelated rows are banked only when the next withdrawals or acquisition need more slots than the posted inventory has free, and only enough rows to make that room. Recipe-less authored goals may be gathered or transformed in stages; their full quantities are not reserved at once. Available bank hints are withdrawn only when they fit alongside the active recipe's peak, which may mean banking unrelated rows to make room; otherwise the authored steps acquire them later. A completed Path leaves its remaining inventory in place for the next Path to assess.
Unknown recipe transformations retain a conservative peak when room can be made, which can deposit one extra unrelated row. An unmeetable speculative peak does not itself park the Path; preparation falls back to immediate withdrawals while still requiring the active acquisition's final item slot, and leaves later inventory pressure to the running family. Provisioner's bank scan, capacity deposit and withdrawal boundaries are included in the bounded run trace; the terminal `finish` line is emitted even if that trace reaches its event cap.

### 4.4 Rules of thumb

- **Polarity.** `skip_if` means "this is already done"; `settle` means "I can see it worked". `{"Any":[]}` means never skip; do not write `{"All":[]}`.
- **Observable settles only.** Use inventory facts (`has_item`, `item_count_at_least`), position (`near`, `in_area`, `on_level`), chat (`message` after the step began; `message_state` for recoverable hints), quest colour (`quest_colour`), and journal progress (`stage_in`, `flag`). A direct journal-progress fact in `settle` requires `advances: true`. A settle that no evidence can make true will time out.
- **`advances`.** Declare whether the server can change quest progress on Explicit kinds. Use `true` for starting the quest, hand-ins, and dialogues that advance the journal; use `false` for purchases, replacing a lost item, or informational dialogue. Default-class kinds may omit the field.
- **Sources and names.** Give each authored destination/anchor a `source` (for example, a content placement, source file and line, or dated live observation). A `near` fact is an observation and has no source. Use content config names, not display names; use a `name` target only when the content pack makes a location ambiguous.
- **Stage keys.** Use `"<quest>:<n>"` when the content varp value is verified; otherwise use a symbolic stage key. A rule's `varp` is a fixture seed and status hint, not a runtime gate.
- **Recipes vs sequences.** Put item acquisition in `quest.acquire` recipes referenced by `acquire` steps. Keep stage sequences focused on that stage's actions.
- **Ordered sequences.** Reach for `"order": "ordered"` only when consecutive steps have no observable "done" evidence. Where evidence exists (an item held, an area reached, a journal flag), prefer `authored` order with a real `skip_if`, because that also recovers correctly after a restart instead of replaying.
- **Dialogue on `interact`/`use_on`.** One policy for both. Omission is strict Continue-only for observed chat and selected Scroll/Book pages. It drains Continue pages and refuses menus without sending an answer, leaving unrelated main interfaces untouched. No page is required. Omission-mode no-page completion waits for acceptance and a fresh idle-player tick. For Loc/Npc/name scene targets, no-page completion also requires post-acceptance movement or primary animation unless the target was within interaction range at acceptance; a fresh idle tick remains required in either case. Loc/Npc/name interactions require their own accepted dispatch receipt. This boundary covers arrival and primary animation without a fixed delay. With omitted dialogue, reaching `until` completes immediately even if a page is open or an optional dialogue driver is active; the step does not adopt a newly observed page or continue draining an existing one. The next owner handles any page that remains. `settle_ms` bounds the step even while a page is open and is not extended by dialogue draining; a count first observed after the deadline times out. Explicit `"continue"` or the object form requires a page after every accepted round, including every `until` round. No page after the bounded opening wait fails with `expected dialogue did not open`. Use `"continue"` when content requires a page. Use the object form with `prefer`/`choose`/`line_rules` when content can open a menu. `"none"` never touches dialogue. A `talk` step that adopts an already-open page (no Talk-to sent) answers only through its own `prefer`/`choose`/`line_rules`, never the default last option.
- **Chat-to-scroll transitions.** An owned conversation can finish on the selected scroll (including the source-proven quest-completion scroll) or book interface after its chat closes. The shared dialogue driver requires a witnessed owned page or an accepted, fresh Continue/Answer receipt; an in-flight advance waits within the existing page bound. An unrelated interface, a refused answer, or a scroll without an owned conversation does not prove dialogue success.
- **Combat `finish`.** Flat `prefer`/`choose`/`strict`/`line_rules` fields use the shared dialogue selector, and `max_ticks` measures observed game ticks, not host polls.
- **Versions.** Adding an optional Args field keeps the handler version. Renaming, removing, or retyping a field requires a new version; the generated schema will then expose the new contract.

### 4.5 Fully annotated example — Cook's Assistant (schema-3 form)

Comments are annotations, not valid JSON; the real file has none. Changes from the shipped file are marked `← new`.

```jsonc
{
  "$schema": "../path.schema.json",            // ← new: editors validate against the generated schema
  "schema": 3,                                 // ← clean schema 2 → 3 cutover
  "id": "cook",                                // content quest stem (C scripts/quests/quest_cook)
  "display_name": "Cook's Assistant",          // UI only
  "required": [],                              // ids from quest.requirements that gate Start (none)
  "tested_stats": null,                        // written only by a live PASS; never a gate
  "partner": null,                             // Shield of Arrav / Hero's only
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
    "bank": { "tile": [3093, 3243, 0], "source": "A defs/cooksassistant.ts:145" },   // or "nearest"
    "coin_float": 0,
    "loadouts": {},
    "areas": {},
    "tools": ["obj:pot_empty", "obj:grain", "obj:bucket_empty", "obj:egg"],   // kept when provisioning deposits (S4)
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

Compilation failures keep the Path and step identifiers. Unresolved `obj`, `npc`, and `loc` references include the authored alias in `detail`; the queue message preserves that alias and the failing step id.

### 4.6 Checking a Path before asking for a live run

1. Editor: no red squiggles under the `$schema` line's document.
2. Shape, quick form (prints `<file> valid` or the failing data path):
   `npx --yes -p ajv-cli@5.0.0 ajv validate --spec=draft2020 --errors=text -s crates/script/paths/path.schema.json -d crates/script/paths/289/<id>.json`
   Numeric constraints use the standard JSON Schema `type`, `minimum`, and `maximum`; no custom format plugin or disabled format checks are needed. For a failing large Path, rerun with `--errors=json --verbose`: JSON gives `instancePath`, `schemaPath`, and data, plus the applicable schema fragment (`parentSchema`) that identifies the Args shape; the text form only names the data path.
3. **Facts and embedding:** add every bundled Path to `paths/289/index.json`; `bundled_paths_decode_and_compile` reads each indexed row's `file` from disk, not through `path_bytes`. Add the file to the `path_bytes` match in `compile.rs` for runtime embedding, then run `cargo test -p script bundled_paths_decode_and_compile` to compile every indexed Path. Errors include the Path, step, code, and detail. A quest the server can't run gets a row with `name` and an end-user `unavailable` reason (keep `file` if a Path was authored, so it stays compiled); see `docs/api/script.md`.
4. **Rule shadows and selection probes** (`script::quester::probe`, available to `crates/script/tests` through the `test-hooks` dev-dependency): `assert_no_shadowed_rules(&compiled)` feeds every `progress.rules` entry exactly its own needles (its `all` plus each `any` in turn) through first-win resolution and fails on any rule that resolves to an earlier stage or to nothing; a rule that only wins through a `not` needle present in the live journal but absent from the later rule's needles is reported too, so pin that text in the later rule's `all`. `Probe { path, selected, quests, progress, bank }.choice(stage, cursor, &snapshot)` returns the `Choice` that `select` makes for a seeded `GameSnapshot` (inventory, local player tile, locs, chat) and `progress_for_stage`/`known_empty_bank` build the evidence; probe every multi-step sequence at each state it is expected to resume from, and every `ordered` sequence at cursors 0..n, to prove no step is selected forever.
5. **Live fixture:** compare the Path with captured journal text and world evidence, especially after a settle timeout. Compilation cannot prove that `advances` reflects the server's actual behavior; a false declaration for an advancing action can leave the cached stage stale and repeat the action until timeout, while a true declaration for a non-advancing action causes an unnecessary progress read.

## Reading a run

Quester writes its run trace to the operator log (and to the per-session log file when enabled). `info` lines show the Path start, stage changes, step begins/skips/settles, acquisition-child activity, combat outcomes, and finish/Stop. `warn` lines name step failures and the parked reason. A skip line includes the `skip_if` predicate that evaluated true. Identical retry events are written once, with a later summary such as `(repeated 4 additional times)` rather than one line per retry. The trace retains at most 32 distinct events.

Preparation may run acquisition recipes before an authored quest step begins. These use the same acquisition-child trace as an authored `acquire` step, including their final settle or failure. A root `step ... begin` line therefore appears only when that authored step actually starts, not while the provisioner is collecting its prerequisites. Items gated by `from_stage` stay out until their stage, so a quest that gates every item (Cook gates all three on `cook:1`) talks first and scans later.

