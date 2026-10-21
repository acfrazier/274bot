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

## Grinch live cells

`grinch_live.py` runs all twelve real Grinch A, B and C cases sequentially. Each
case uses the same owned scratch lifecycle: copy the engine, content, real map
inputs and account storage into its own evidence subdirectory, use free
loopback ports, pack the real inputs and start `src/app.ts`. The runner requires
absolute paths for `--engine-dir`, `--catalog-root`, `--nav-pack`,
`--evidence-dir`, and `--isohome`; the evidence directory must be new.
`--cells A B C` is the default. Use the reviewed revision-289 nav pack and a
throwaway-HOME wrapper. Fresh accounts complete a real logout/login before
setup. The driver then requires a fresh same-session tutorial-completion reply
and bound inventory tab before it moves to the anchor or seeds the test
inventory.

The runner never starts, stops or edits Engine A or Engine Q. It reserves free
ports for every scratch instance; protected A/Q ports are never candidates.
Free disk must be at least 130 GiB before each copy and start.

All A and C scratch copies use unchanged content and map inputs: their
`content_patch` authority is null, and source-versus-copy content identities
must match before and after packing. Only the five B cases add their existing
fixture rows and object respawn rates. A, B and C all use the same real
`tools/pack/Build.ts` and `src/app.ts` path, and every scratch case records
`server_tick_ms: 600`. Development Prometheus statistics are disabled; cycle,
lifecycle, player-processing, output and Take evidence comes from log-only
hooks in each copied engine. The retained
`cycle-instrumentation.diff` and `take-instrumentation.diff` files contain the
actual source-versus-patched unified diffs. The original Engine A source stays
unchanged.

Every case writes a plain `{"scratch": authority}` manifest and applies
`GRINCH_SERVER_LOG` to the scratch server log. A root Engine A identity fact may
also be retained, but it is not the A/C capture authority. Server logs
corroborate packet observations; PLAYER_INFO remains the settlement clock.
The B fixture witnesses remain tied to their content patch.

Native traces retain Grinch state and Take settlements. Accepted decoded
batches count Take sends only when they contain exactly one R289 OPOBJ3;
other opcodes may share the batch. Change-only Watchdog debug records identify
recovery actions. Slot run generations and restart records identify script
replacement. Missing observations fail the oracle instead of becoming zero
counters.

`InteractTrace` deliberately stays out of the structured session-log sink.
The runner supplies its captured process log as `GRINCH_STDERR_LOG`. Packet
and interaction references use actual line ordinals in that file, not invented
host-log ordinals. Metadata records the captured prefix length because Cargo
can append its final test summary after the driver reads the log.

Structured records use their owning client header tick. Native stderr clocks
join to one exact RunKey-linked PLAYER_INFO sequence and must agree with the
corresponding structured state clocks. Missing or ambiguous joins fail; the
driver never estimates an offset across logout/login. The qualified run starts
at the first post-Start frame with its exact sampled RunKey and native state.
Earlier queued-Start observations remain in the raw metadata.

The scratch manifest records the real server stop outcome; it does not predict
cleanup. Each engine/content copy is deleted only after the server, packer and
capture groups have proven stops. Logs and receipts remain, including the
source-versus-patched hook diffs. A missing receipt or an unproven stop fails
closed and retains the copy for inspection. Each cell's full oracle runs only
after its scratch cases have stopped. The A/C traces record observed
busy-area interference with receipt ticks instead of attributing unexplained
row changes to Grinch.

Artifacts include per-variant typed traces and metadata, oracle receipts,
scratch content-patch digests, world configuration, pack/capture/server logs,
and actual process lifecycle receipts. A captured trace is not a passing
qualification without its full cell oracle. `--self-test` checks synthetic
patch, port and exact-test-cardinality inputs plus real owned-process cleanup.
With `--engine-dir`, it also checks hook compatibility and TypeScript syntax
on temporary source copies. These checks are not a live result.
