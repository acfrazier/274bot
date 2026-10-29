# Quest paths for contributors

> **Who this is for:** you maintain the Quester guide format, its step handlers, or the guide pipeline. You read Rust. For the author format, see [Write a quest guide for the Quester](../scripting/quest-guides.md). For how users run guides, see [Quester: run guides from a folder](../manual/built-in-scripts.md#quester-run-guides-from-a-folder).

## Migrate a guide to schema 3

Schema 3 is the only accepted document version. `PATH_SCHEMA` is `3` (`crates/script/src/quester/path.rs:35`), and any other top-level `schema` fails with `unsupported-schema` (`crates/script/src/quester/compile.rs:678-679`).

1. Change the top-level `"schema"` value from `2` to `3`.
2. Add `"$schema": "../path.schema.json"` to every file in `crates/script/paths/289/`, or the matching relative path in another guide directory.
3. Add `"advances": true` or `"advances": false` to every Explicit step kind: `talk`, `interact`, `use_on`, `make`, `combat`, `gather`, and `thieve`. Use `true` when the server changes quest progress, and `false` when it does not. Default-class kinds may omit the key, which compiles as `false`.

The compiler rejects a missing `advances` on an Explicit kind with `advances-undeclared`, and a direct progress fact in `settle` without `advances: true` with `settle-needs-advance` (`crates/script/src/quester/compile.rs:1239-1252`). A direct progress fact is any of `stage_in`, `flag`, or `quest_colour`, matched through the handler's `ProgressRead` class (`crates/script/src/quester/compile.rs:1330-1341`). A journal count used as an item quantity (`qty: { "progress": ... }`) is inventory evidence, not a direct progress fact, and does not require it (`crates/script/src/quester/families/s2.rs:57-62`, `crates/script/src/quester/families/mod.rs:849-854`).

## Register a step or fact handler

Step handlers live in `crates/script/src/quester/families/mod.rs:55-82`, registered with `step!`. Predicate handlers follow in the same file (`:84-298`), registered with `fact!`. Both macros are defined in `crates/script/src/quester/compile.rs:114-145`. Each row names one `Args` type, which supplies compiler decoding and the generated schema. Do not maintain a separate argument-field list.

Each `step!` row declares an `AdvanceClass`: `Explicit` or `Default` (`crates/script/src/quester/compile.rs:95-100`). Quest-specific steps use the same macro: `x:totem:solve_combination` is Explicit (`crates/script/src/quester/handlers/totem.rs:60-63`). The compiler looks handlers up by `kind` and `version`; an unregistered pair fails with `unknown-handler` (`crates/script/src/quester/compile.rs:1235`). Adding an optional `Args` field keeps the handler version. Renaming, removing, or retyping a field needs a new version row, and the generated schema then exposes the new contract (`crates/script/src/quester/schema.rs:56-100`).

## Generated schema and shape checks

`crates/script/src/quester/schema.rs:193` renders `paths/path.schema.json` from the Rust document and handler types, and `:212` resolves the output path. The `#[ignore]` test `regen_path_schema` rewrites the file (`crates/script/tests/path_schema.rs:336`). Numeric constraints use standard JSON Schema `type`, `minimum`, and `maximum`, with no custom format plugin (`crates/script/tests/path_schema.rs:300-335`).

`npx --yes -p ajv-cli@5.0.0 ajv validate --spec=draft2020 --errors=text -s crates/script/paths/path.schema.json -d crates/script/paths/289/<id>.json`

Rerun a failing large guide with `--errors=json --verbose`. JSON gives `instancePath`, `schemaPath`, and data, plus the applicable schema fragment (`parentSchema`) that names the `Args` shape. The text form names only the data path.

## Bounded content acquisitions

### Kill until the required item drops

Use the combat family's `until_drop` policy for a chance-only required item:

```json
{
  "until_drop": {
    "obj": "bronze_axe",
    "max_kills": 24,
    "stop": "GAP-entrana-zombie-axe-budget"
  }
}
```

Add this object to the usual `combat` arguments, alongside the target and tactic.
The policy takes the required item through the combat family's existing loot phase.
It completes only when the item reaches the inventory.
The kill count persists across native re-engagements within that step.
Only a confirmed `Killed` result spends the budget.
Disappearance, a missing target, or an attempt timeout does not count as a kill.

The final permitted kill still receives the normal loot wait and pickup.
The appearance deadline does not cancel a pickup that already started.
If the item remains absent, the runner parks with the authored `stop`.
The combat receipt and the `combat_kills` status field retain the total kill count.
The policy cannot share a step with `finish`, `until`, or `win`.
The compiler rejects a zero budget, an empty stop, or an unresolved item.

The guide author derives the kill budget from the content's drop rate.
Cite that rate and the budget calculation in the guide.
For Lost City's zombie axe, `drop tables/scripts/zombie.rs2:102-110` gives a rate of 50/128.
A 24-kill budget leaves a miss probability of `(78/128)^24`, about 0.0007%.
This is a bound, not a promise that an item will drop.
Lost City and Eadgar can use this policy for their chance-only axe and key sources.

### Select a production menu opened by item use

Use the existing `use_on` family to open the menu.
Set its `settle` predicate to `{"All": []}`.
The item-use step confirms accepted dispatch and leaves the production menu for `make`.
The next `make` step observes the posted chat or main menu.
`modal_open` covers main interfaces only and does not detect chat production menus.
The item-use step does not adopt the menu as dialogue or answer its buttons.
Then use `make` with neither `loc` nor `anchor`:

```json
{
  "product": "unstrung_shortbow",
  "menu": {
    "obj": "shortbow",
    "source": "skill_fletching/scripts/cut_logs.rs2:18-47"
  },
  "qty": 1,
  "make_x": false
}
```

The `product` names the output item.
The optional `menu.obj` names the item represented by the posted menu row.
Some menus show an input item instead of the output.
`qty` is a held target: hold at least this many output items after production.
It is not an instruction to make that many additional items.
The machine selects the named product and a quantity that does not exceed the remaining target.
The compiler rejects a missing half of the `loc` and `anchor` trigger pair.
Existing loc-triggered `make` steps retain both fields.

The same selector works after item-on-item and item-on-loc use.
Ikov can use it after a knife on logs.
Zombie Queen can use it for bronze wire smithing.
Nature Spirit can use it for the silver sickle.
Dragon Slayer can use it for the unfired bowl.
These steps reuse the shared production machines and do not add another production loop.

### Take a temporary ground item

Add `max_ticks` to a ground-item `interact` with operation `Take`:

```json
{
  "target": { "ground": "jail_key" },
  "op": "Take",
  "anchor": {
    "tile": [2931, 9690, 0],
    "source": "Use the guide's content-derived Jailer combat stand."
  },
  "radius": 8,
  "max_ticks": 100
}
```

Use the guide's actual combat stand, not the example tile.
The bound starts when the step begins.
It covers both the missing-spawn wait and inventory settlement.
Repeated host polls do not spend observed game ticks.
The step waits for the item without a separate `wait_if_missing` setting.
A successful Take needs an inventory increase, not merely an accepted click.
An inventory increase at the final permitted tick still completes.
An increase first observed after that tick does not complete.
Expiry parks with `ground pickup exceeded its tick budget`.
The typed receipt records the observed tick count and the final held count.
The status fields expose these values as `ground_waited_ticks` and `ground_held`.

The compiler rejects zero, other target kinds, and operations other than `Take`.
This form cannot combine `max_ticks` with:

- `settle_ms`
- `dialogue`
- `until`
- `wait_until`
- An exact target tile
- `reachable_only`

Use the ordinary form for those contracts.
Scorpion Catcher can use this form for the Jailer key.
The content posts that key for 100 ticks in `drop tables/scripts/jailer.rs2:10`.

## Ship a bundled guide

Every bundled guide is indexed and embedded three ways, and all three must agree.

1. Add the row to `crates/script/paths/289/index.json`. The index has its own schema (`schema: 1`), checked by `validate_index` (`crates/script/src/quester/queue.rs:369`) and asserted by the schema test (`crates/script/tests/path_schema.rs:341-360`). A shipped draft carries `"status": "draft"`. A quest the server cannot run keeps a row with `name` and an end-user `unavailable` reason, with `file` kept when a guide was authored so it stays compiled.
2. Add the file to the `path_bytes` match in `crates/script/src/quester/compile.rs:1774` for runtime embedding.
3. Run `cargo test -p script bundled_paths_decode_and_compile`. The test reads each indexed row's `file` from disk, not through `path_bytes`, and compiles every indexed guide (`crates/script/tests/path_schema.rs:341-390`). Errors carry the guide, step, code, and detail. A filename whose stem differs from the document `id` fails with `path-id-mismatch` (`crates/script/src/quester/registry.rs:1101-1107`).

Queue order follows the release index. Draft rows are opt-in for an empty quest selection (`crates/script/src/quester/queue.rs:163-164`). Unavailable rows stay blocked with their reason (`crates/script/src/quester/queue.rs:22-35`).

## Folder loading pipeline

Folder loading runs on the existing preparation worker; active runs keep their snapshot (`crates/script/src/quester/registry.rs:1-2`). Limits are constants at the top of that file. Files cap at 1 MiB (`MAX_PATH_BYTES`, `:19`), rows at 256 (`MAX_REGISTRY_ROWS`, `:31`), diagnostics at 32 lines (`MAX_DIAGNOSTIC_LINES`, `:36`). Observed folder entries cap at 4096 (`MAX_FOLDER_ENTRIES`, `:35`).

The loader sorts valid folder filenames and loads at most the first 256 in filename order; later files are omitted with a `too-many-paths` diagnostic (`crates/script/src/quester/registry.rs:1402-1429,1295-1301`). An invalid override falls back to its bundled guide, while an invalid draft stays visible but unavailable, with the validation error as its reason (`crates/script/src/quester/registry.rs:1336-1364`). Other documents still load. The report reads `Reload Paths: N folder documents, M registry rows, K validation errors (folder)` (`crates/script/src/quester/registry.rs:1212-1218`). Picker labels append `[folder]` or `[draft]`, plus the reason after an em dash (`crates/script/src/quester/registry.rs:1233-1255`). Folder guides support revision 289 only (`crates/script/src/quester/registry.rs:744-747`).

Both front ends share the labels in `crates/frontend-core/src/quester_paths.rs:12-18`. The panel wraps a success summary and leaves an error unwrapped (`crates/panel/src/app.rs:4404-4408`). The terminal app shows only the first line, with details in the Logs (`crates/tui/src/shell.rs:175-180`; every notice is also logged at `crates/frontend-core/src/quester_paths.rs:484-490`).

Reload triggers: the **Reload Paths** command (`crates/frontend-core/src/quester_paths.rs:374-401`), any settings change (`:337-370`), a saved enabled source at startup once game data is ready (`:319-327,413-433`), and every quest Start through preparation (`crates/script/src/quester/card.rs:317`). Turning the setting off resets the registry to bundled (`crates/script/src/quester/registry.rs:217-223`).

## Check a guide before a live run

`script::quester::probe` is compiled for unit tests and for the `test-hooks` dev-dependency that integration tests use (`crates/script/src/quester/probe.rs:1-3`, `crates/script/src/quester/mod.rs:13`). `assert_no_shadowed_rules` fails on any journal rule that resolves to an earlier stage or to nothing (`crates/script/src/quester/probe.rs:20-35`). `Probe::choice` returns the `Choice` that selection makes for a seeded snapshot, and `recipe_choice` does the same inside a recipe (`crates/script/src/quester/probe.rs:78-93`). `progress_for_stage` and `known_empty_bank` build the evidence (`crates/script/src/quester/probe.rs:155-191`).

Probe every multi-step sequence at each state it should resume from, and every `ordered` sequence at cursors `0..n`. A false `advances` on an advancing action leaves the cached stage stale and repeats the action until timeout. A true `advances` on a non-advancing action costs a needless progress read. Compare the guide with captured journal text and world evidence after a settle timeout. Compilation cannot prove that `advances` matches the server.

## Runner notes for contributors

Sequence order is `SequenceOrder::Authored`, `Nearest`, or `Ordered` (`crates/script/src/quester/path.rs:44-62`). The ordered cursor restarts on every run start, resume, death, or stage change, so every step must be safe to replay (`crates/script/src/quester/runner.rs:1944-1985`). One exception keeps the cursor while paired work is pending (`crates/script/src/quester/runner.rs:3197-3199`).

Journal latency never spends the family's post-read settle window (`crates/script/src/quester/runner.rs:1991-1992`). The no-progress watchdog warns at three unchanged step boundaries and parks at eight (`crates/script/src/quester/watchdog.rs:1-2,83-100`). The run trace keeps 32 events and folds repeats into `(repeated N additional times)` (`crates/script/src/quester/runner.rs:126,212`). An `acquire` recipe restarts at most 8 times past its last settled child (`crates/script/src/quester/families/mod.rs:3633-3648`).

Paired validation compiles each declared gang role before the guide is available, and names the failing gang alongside the per-file error (`crates/script/src/quester/registry.rs:1159-1180`). A run activates only the account's effective gang after owned membership admission (`crates/script/src/quester/pair.rs:65`, `crates/script/src/quester/gang.rs:132-160`).

Bank memory has three states. A bank never seen (`Unknown`) costs one scan trip. A shortage known from an earlier login (`Hint`) costs one verifying trip. Only a shortage in a bank seen open this login (`Session`) blocks in place (`crates/script/src/quester/provision.rs:4-6,27-29,511-512`). An item with `from_stage` is ignored until its stage, and an `Unknown` stage keeps it out (`crates/script/src/quester/compile.rs:183,797`, `crates/script/src/quester/provision.rs:275`). The terminal `finish` line is emitted even when the trace hits its event cap, and the queue logs the next guide being prepared (`crates/script/src/quester/runner.rs:974-977,3876`).
