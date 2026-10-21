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

`grinch_live.py` runs all twelve real Grinch A, B and C cases sequentially.
Each case copies engine, content, real map inputs and account storage into
its own evidence subdirectory, packs the inputs and starts `src/app.ts`.
It requires absolute paths for `--engine-dir`, `--catalog-root`, `--nav-pack`,
`--evidence-dir`, and `--isohome`; the evidence directory must be new.
`--cells A B C` is the default. Use the reviewed revision-289 nav pack and a
throwaway-HOME wrapper. Fresh accounts complete a real logout/login before
setup. The driver then requires a fresh same-session tutorial-completion reply
and bound inventory tab before it moves to the anchor or seeds the test
inventory.

The runner never starts, stops or edits Engine A or Engine Q. Only one copy
runs at a time; free disk must be at least 130 GiB before copying and starting
it. The runner prefers loopback ports 44794/1280/10098 and records any free-port
fallback. Protected A/Q ports are never candidates.

A and C copies use unchanged content and map inputs: `content_patch` is null,
and source-versus-copy content identities must match before and after packing.
Only the five B cases add static OBJ rows and set tested raw respawn rates to
502. They preserve existing map rows and use the real config/map packers.
Every case records `server_tick_ms: 600` and a plain `{"scratch": authority}`
manifest. A root Engine A identity fact is not A/C capture authority.

Development Prometheus statistics are disabled. Log-only hooks in each copied
`World.ts` and object-option handler record actual server cycles, completed
lifecycle turns, player-processing order, active accounts and tiles, emitted
PLAYER_INFO sequences, and server-side Take acceptance or rejection. The
driver joins exact records, never inferred elapsed-time cycles. Server logs
corroborate observations; PLAYER_INFO remains the settlement clock.
`GRINCH_SERVER_LOG` identifies the owned log for every case. Source and patched
digests and actual unified diffs remain in `cycle-instrumentation.json`,
`take-instrumentation.json` and their corresponding `.diff` files. The hooks
only log and change no game behaviour; Engine A source stays unchanged.

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
Generation and RunKey are read together under the slot lock after native work
and same-frame walk completion on every main PLAYER_INFO edge. This read-only
observer and its storage exist only in live/test-support builds. The control
thread sends Start after an observed completed frame; that frame is the
pre-Start fence. Batched outputs keep their own runtime references; timer
samples never backfill missing frame authority.
Positive helper-Drop captures retain at least 40 observed idle ticks after the
last selected-item inventory increase, not the first pickup. The oracle
checks the final 40 frames for healthy Waiting at the exact anchor.

Helper Drops wait for the sampled native RunKey. Their action clock uses the
receiver PLAYER_INFO edge with the same logged lifecycle turn as the helper
PLAYER_INFO at the accepted send. Metadata retains those exact send references
and the later inventory-decrement and receiver confirmations separately.
Public-reveal intervals never start at a later confirmation frame.

The driver validates the selected nav facts before login. The runner checks
the scratch management endpoint's actual configuration, then captures the
cell. Success and failure cleanup stop only its recorded process groups.
The scratch manifest records the real server stop outcome; it does not predict
cleanup. Each engine/content copy is deleted only after the server, packer and
capture groups have proven stops. Logs and receipts remain, including the
source-versus-patched hook diffs. A missing receipt or an unproven stop fails
closed and retains the copy for inspection. Each full cell oracle runs only
after its scratch cases have stopped. The A/C traces record observed busy-area
interference with receipt ticks instead of attributing unexplained row changes
to Grinch.

Artifacts include per-variant typed traces and metadata, oracle receipts,
scratch content-patch digests, world configuration, pack/capture/server logs,
and actual process lifecycle receipts. A captured trace is not a passing
qualification without its full cell oracle. `--self-test` checks synthetic
patch, port and exact-test-cardinality inputs plus real owned-process cleanup.
With `--engine-dir`, it also checks hook compatibility and TypeScript syntax
on temporary source copies. These checks are not a live result.
