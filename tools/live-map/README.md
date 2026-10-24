# Live path map

This tool lists the live cells that cover changed product paths. A live path changes bot actions in the game or changes what the operator sees. See `AGENTS.md:142` for the full rule.

Run the tool from the repository root:

```sh
python3 -B tools/live-map/live_map.py --diff BASE..HEAD
```

The tool reads changed names with `git diff --name-only`. It lists the covering cells and their exact commands. Run each listed cell three times on the changed code. The tool then lists uncovered live paths and unclassified paths. Exit code 0 means it found no gaps. Exit code 1 means a live path has no cell or a path is unclassified. Exit code 2 means the input or Git diff is invalid.

Run the self-tests with temporary Git repositories:

```sh
python3 -B tools/live-map/live_map.py --self-test
```

## Validate test selectors

Before using a map, list the tests selected by every unique cell command:

```sh
ISOHOME=/Users/acfrazier/experiments/274bot/.superpowers/bin/isohome \
  python3 -B tools/live-map/live_map.py --verify-cells
```

The verifier runs each cell's Cargo test selector with its declared package and feature arguments and `-- --list`. It fails if a selector exits unsuccessfully or matches no tests. This checks test selection only; it does not run the live cell.

## Add a cell

1. Write and run the live cell against real game content. Do not seed the target resource or entity.
2. Add one `[[cover]]` row for the narrowest product paths the cell exercises.
3. Add each cell as `name|command` in that row's `cells` array. Keep the command exact.
4. Add test-only files to `[not_live]`, not `[live]`.
5. Run the tool on the change. Confirm that it lists the new cell and no changed live path remains uncovered.

A path that matches `[not_live]` is not live, even if it also matches `[live]`. A path that matches neither is unclassified. Do not use a broad live glob to hide an uncovered path.

Commands use `${ISOHOME:?}` for the absolute isohome executable and `nice -n 10` for Cargo. Before a live run, set `ISOHOME=/Users/acfrazier/experiments/274bot/.superpowers/bin/isohome`, choose a unique `BOT_LIVE_NAME_PREFIX`, and provide every `${NAME:?}` shown in the selected command. Those inputs may include an engine directory and port-specific nav pack, a cache or unpack directory, `CATALOG_ROOT`, `BOT_NAV_PACK`, `BOT_UNPACK_DIR`, `RELOG_*`, `BOT_EVIDENCE_DIR` or `LIVE_EVIDENCE_DIR`, and a fresh writable evidence path. Quest path smoke also needs `QUESTER_PATH` and `QUESTER_SEEDS`. Each value must name the local input or writable run path required by that cell.

The scenario registry and nightly cell lists are separate from changed-path matching. Inspect them with:

```sh
python3 -B tools/nightly/run_nightly.py --list
```
