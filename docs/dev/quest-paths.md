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
