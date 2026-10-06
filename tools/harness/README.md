# Offline player-save fixture tools

These tools write account save files (`.sav`) for harness preparation without a
running engine. The writer uses the engine's own `Player.save()` and checks the
result with `PlayerLoading.verify` and `PlayerLoading.load`. It never edits the
engine tree.

| File | Role |
| --- | --- |
| `write_player_fixture.ts` | The writer. Flags: `--username NAME` (1–12 letters, digits or underscore), `--output PATH.sav`, `--fixture thiever\|bone_burier_v2` (default `thiever`), `--profile main`, `--receipt PATH.json`, `--overwrite`, `--server-root PATH`. It refuses to overwrite an existing save without `--overwrite`. |
| `run_write_player_fixture.sh` | Bundles the writer with `esbuild` from the engine's own `node_modules`, using the stubs in `stubs/` for the `World`, `Environment`, network-player and bzip2 modules, and keeps the real bzip2 wasm for pack loads. It resolves the engine root first and exits 2 when it is missing or lacks the engine sources or `esbuild`. |
| `test_server_root_selection.sh` | Regression for root selection: with `BOT_SERVER_ROOT` forced to root A it writes a save with `--server-root` A, one with `--server-root` B and one more with B as control, then runs the checker. Needs `TEST_ROOT_A` (a 274 engine root) and `TEST_ROOT_B` (an isolated 289 engine root, recognised by `changePlayerOccCollision` in its `Player.ts`); the roots must differ, and each needs `src/engine/entity`, `data/pack/server/obj.dat` and `node_modules/.bin/esbuild`. |
| `check_server_root_selection.py` | Asserts the outputs by receipt provenance: the control A save comes from root A, the CLI B save comes from root B (not from the env root) and matches the control B save byte for byte, and it differs from the A save. |

## Engine root

One root feeds the source, modules, `data/pack`, bzip2 and the receipt:
`--server-root` on the command line wins over `SERVER_ROOT`, which wins over
`BOT_SERVER_ROOT`. There is no machine default. A missing root, an empty
`--server-root` value or a root without the engine sources exits 2 and names
the variables.

## Example

```sh
./tools/harness/run_write_player_fixture.sh \
  --username fixtest01 \
  --output /tmp/fixtest01.sav \
  --receipt /tmp/fixtest01.receipt.json \
  --fixture thiever \
  --server-root /path/to/engine \
  --overwrite
```

Root-selection regression:

```sh
TEST_ROOT_A=/absolute/path/to/274-engine \
TEST_ROOT_B=/absolute/path/to/isolated-289-engine \
./tools/harness/test_server_root_selection.sh
```
