# Quest paths for contributors
## Migrating a Path to schema 3

For Paths authored against schema 2, make this clean cutover before adding new work:

1. Change the top-level `"schema"` value from `2` to `3`.
2. Add `"$schema": "../path.schema.json"` to every file in `crates/script/paths/289/` (or the corresponding relative path in another Path directory).
3. Add `"advances": true` or `"advances": false` to every Explicit step kind: `talk`, `interact`, `use_on`, `make`, `combat`, `gather`, and `thieve`. Use `true` when the server changes quest progress; use `false` when it does not. Default-class kinds can omit the key, which compiles as `false`.

A direct progress fact (`stage_in`, `flag`, or `quest_colour`) anywhere inside a step's `settle` requires `advances: true`. A journal count used as an item quantity (`qty: { "progress": ... }`) is inventory evidence, not a direct progress fact, and does not require it. The schema does not accept version 2; do not mix schemas in a release. Validate the shape with the pinned AJV command in §4.6 and compile every indexed Path with `cargo test -p script bundled_paths_decode_and_compile` before asking for a live run.
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
The `gather` step supports Woodcutting, Mining and Fishing through the same
selected gathering catalog and native action as bundled Paths. Acquisition
recipes and typed progress readers use that catalog as well.
### 4.6 Checking a Path before asking for a live run

1. Editor: no red squiggles under the `$schema` line's document.
2. Shape, quick form (prints `<file> valid` or the failing data path):
   `npx --yes -p ajv-cli@5.0.0 ajv validate --spec=draft2020 --errors=text -s crates/script/paths/path.schema.json -d crates/script/paths/289/<id>.json`
   Numeric constraints use the standard JSON Schema `type`, `minimum`, and `maximum`; no custom format plugin or disabled format checks are needed. For a failing large Path, rerun with `--errors=json --verbose`: JSON gives `instancePath`, `schemaPath`, and data, plus the applicable schema fragment (`parentSchema`) that identifies the Args shape; the text form only names the data path.
3. **Facts and embedding:** add every bundled Path to `paths/289/index.json`; `bundled_paths_decode_and_compile` reads each indexed row's `file` from disk, not through `path_bytes`. Add the file to the `path_bytes` match in `compile.rs` for runtime embedding, then run `cargo test -p script bundled_paths_decode_and_compile` to compile every indexed Path. Errors include the Path, step, code, and detail. A shipped draft guide carries `"status": "draft"` in its index row. A quest the server can't run gets a row with `name` and an end-user `unavailable` reason (keep `file` if a Path was authored, so it stays compiled); see `docs/api/script.md`.
4. **Rule shadows and selection probes** (`script::quester::probe`, available to `crates/script/tests` through the `test-hooks` dev-dependency): `assert_no_shadowed_rules(&compiled)` feeds every `progress.rules` entry exactly its own needles (its `all` plus each `any` in turn) through first-win resolution and fails on any rule that resolves to an earlier stage or to nothing; a rule that only wins through a `not` needle present in the live journal but absent from the later rule's needles is reported too, so pin that text in the later rule's `all`. `Probe { path, selected, quests, progress, bank }.choice(stage, cursor, &snapshot)` returns the `Choice` that `select` makes for a seeded `GameSnapshot` (inventory, local player tile, locs, chat) and `progress_for_stage`/`known_empty_bank` build the evidence; probe every multi-step sequence at each state it is expected to resume from, and every `ordered` sequence at cursors 0..n, to prove no step is selected forever.
5. **Live fixture:** compare the Path with captured journal text and world evidence, especially after a settle timeout. Compilation cannot prove that `advances` reflects the server's actual behavior; a false declaration for an advancing action can leave the cached stage stale and repeat the action until timeout, while a true declaration for a non-advancing action causes an unnecessary progress read.
