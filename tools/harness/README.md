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
throwaway-HOME wrapper. Before each scratch server starts, the runner hashes
account usernames from the copied `db.sqlite.account` table and `.sav`
basenames in `data/players/<profile>`; only SHA-256 fingerprints are retained.
The driver rejects a minted name present in either pre-login set, then requires
every account to complete a real logout/login before setup. These `.sav` files
are release-289 binary `Player.save()` records (magic `0x2004`, version 7 and
CRC32), not plaintext usernames in the manifest. The driver then requires a
fresh same-session tutorial-completion reply and bound inventory tab before it
moves to the anchor or seeds the test inventory.

The runner never starts, stops or edits Engine A or Engine Q. Only one copy
runs at a time; free disk must be at least 130 GiB before copying and starting
it. The runner prefers loopback ports 44794/1280/10098 and records any free-port
fallback. Protected A/Q ports are never candidates.

A and C copies use unchanged content and map inputs: `content_patch` is null,
and source-versus-copy content identities match before and after packing.
Each B case adds only its exact static OBJ rows to `m49_53.jm2`; it does not
edit object config. The runner preserves existing map rows, packs the original
content for a baseline, then runs the real map/config packers after the map-only
patch. It records and compares before/after hashes for the attested
`server/obj.dat`, `server/npc.dat`, and `client/config` packs, failing closed if any differ.
A per-run engine `ObjType.load()` decode records all four named objects'
actual unchanged default respawn rate of 100; no respawn config override is
applied. This keeps production source attestation intact.
Every case records `server_tick_ms: 600` and a plain `{"scratch": authority}`
manifest. A root Engine A identity fact is not A/C capture authority.

Development Prometheus statistics are disabled. Log-only hooks in each copied
`World.ts`, `Obj.ts` and object-option handler record actual server cycles,
completed lifecycle turns, player-processing order, active accounts, tiles and
their latest PLAYER_INFO sequence, plus server-side Take acceptance or rejection.
Static removal and respawn receipts each log one line containing the real server
cycle, item ID and coordinates, immediately after the existing lifecycle action.
They add no state, delay or packet, and product code never reads these receipts.
The same patch applies to all twelve fresh scratch copies, never Engine A, Q,
builder or concord engines.

The exact respawn proof measures actual removal to actual respawn: 98 server
cycles for the unchanged default 100 scaled to 99 lifecycle turns. It does not
substitute an earlier input-arming receipt or a later inventory-observation PI.
The original wire receipt and player-processing order remain checked, and all
pickup settlement bounds still use PLAYER_INFO. Each respawn event must also
match the first observed fresh row's post-lifecycle output window.

`GRINCH_SERVER_LOG` identifies each owned log. Source and patched digests and
actual unified diffs remain in the cycle, Take and object-lifecycle instrumentation
records. `obj-lifecycle-instrumentation.diff`, `.sha256` and `.json` are retained
both at the evidence root and per copy. The runner rejects a different lifecycle
patch in any later copy. All hooks only log and change no game behaviour.

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
driver never estimates an offset across logout/login. A running-script capture
starts at the first post-Start frame with its exact sampled RunKey and native
state. Earlier queued-Start observations remain in the raw metadata.
Generation and RunKey are read together under the slot lock after native work
and same-frame walk completion on every main PLAYER_INFO edge. This read-only
observer and its storage exist only in unit tests or explicit `test-support`
builds, not in normal panel/TUI `live-harness` builds. The control thread sends
Start after an observed completed frame; that frame is the pre-Start fence.
Batched outputs keep their own runtime references; timer
samples never backfill missing frame authority.
Each completed sample also records the synchronous host-log ordinal fence.
Only rows emitted by that fence may describe its status. Later non-PI evidence
wakes can share a PI tick, but cannot rewrite that earlier capture.
The raw ledger also records received PLAYER_INFO during fresh-account tutorial
relog preparation, before the relog runner advances. Preparation and hold
states do not discard observed output edges or relax complete-stream checks.
The full-nonselected-pack case stops during its first native observation.
Its sampled active RunKey stays absent. For that case only, the driver reads
the existing retained native terminal status and joins its real RunKey to
the exact `inventory-blocked` state row. The typed trace retains the native
card, phase, failure, observed Start generation change and absent active key.
Later PLAYER_INFO frames must retain that same stopped generation and full
pack. A retained diagnostic never supplies active-key authority to a running
case or replaces any raw runtime sample.
Positive helper-Drop captures retain at least 40 observed idle ticks after the
last selected-item inventory increase, not the first pickup. The oracle
checks the final 40 frames for healthy Waiting at the exact anchor.

After client teardown joins every worker, capture waits for the server's own
debug-profile row for the last required cycle. The wait registers a filesystem
append notification before reading, so it neither sleeps nor loses an append
between read and wait. Shutdown without the row fails. The required complete
cycle window comes from exact captured PLAYER_INFO/output joins. Missing rows
inside it fail; unmatched trailing rows stay in the raw ledger and do not count
as cycle authority.

Helper Drops wait for the sampled native RunKey. Their action clock uses the
receiver PLAYER_INFO edge with the same logged lifecycle turn as the helper
PLAYER_INFO at the accepted send. Metadata retains those exact send references
and the later inventory-decrement and receiver confirmations separately.
Public-reveal intervals never start at a later confirmation frame.

Bank route proof considers only ordinary catalog entries with no skill, quest,
setting or custom object/open-first/NPC access override. A default booth needs
no override. The accepted physical bank interaction must match the candidate's
packed access geometry at its resolved routable stand; a generic object name
or the walk stand alone is not proof.

C's opening proof uses the first actual bank interface-open and full-table
packets, not the first later loaded-bank PLAYER_INFO. Every existing client
callback records the public modal `opened_observation`, per-component table
`full_observation`, session/bank generations and host-log fence. Both stamps
come from the client's shared inventory/modal packet-order counter. The
captured relevant stream contains those first opening packets and the accepted
deposit click, ordered by actual callback ownership and, within a callback,
the original shared packet stamps. Its ordinal is the observed PI tick plus
the position in that stream; it does not claim to count every inbound packet.
The click must strictly follow both packets from its loaded bank generation.
The table receipt retains `arrival_bank_generation`, even if the modal was
still closed. A separate `table_binding` records `bound_to_generation`, the
first loaded callback, its identical table stamp and its complete real bank
snapshot. A newer table before that binding or before deposit fails. No arrival
generation or PI frame is relabeled.

A UI change in a callback that also observes a new PLAYER_INFO has unknown
side-of-edge order. The harness retains that ambiguity and supplies no UI
ordinal. The oracle fails closed if it could change the ordering conclusion.
An ambiguous table is still provably earlier when its actual stamp and callback
precede the unambiguous modal and the deposit strictly follows that modal.
This does not supply a table ordinal. Each such opening batch remains counted
in the S4 report, and metadata retains every raw callback. Missing or equal
required ordinals still fail. No product seam, extra drain, added delay, changed
PI frame, settlement bound or acceptance cap is involved.

The contested competitor reacts to observed ground changes on every client
frame, not only PLAYER_INFO. Its Take ledger retains the actual row and scene
generation at send, separately from the owning latest PLAYER_INFO identity.
An intervening object update never invents a new PLAYER_INFO or rewrites its
ground page. Settlement still uses PLAYER_INFO, and all three required wins
must retain the real same-cycle Takes and player-processing order. The harness
does not delay or alter native Grinch work to arrange those wins.
The competitor's projected identity starts at its first retained cycle-window
frame, including that frame's real client session. Its earlier login and
pre-Start activity proof remain in the uncropped raw ledger.
Each participant's 20-edge fresh-row bound uses that participant's own ground
frames and logical ticks. Cross-account race comparisons use real server cycles,
not one participant's logical tick as the other participant's clock.

Contested traces retain every actual competitor send-owning PLAYER_INFO frame
and cycle, including a send before Grinch's first retained frame. Projection
does not change the raw frame, its tick, its session or its cycle. A missing or
changed send owner fails capture. The cycle ledger covers that earlier owner too.
The competitor must see three fresh copies for its three races; Grinch must
see all five copies. After its third win, the competitor stays continuously
active at the tile but sends no further Take. Its ground page need not contain
the two later copies that Grinch collects. Pack settlement uses at most four
of each sender's own later PLAYER_INFO frames, not inventory equality in the
send's server cycle. Exact 98-cycle lifecycle gaps, 20-tick fresh-row bounds,
constant P=2, processing order, send counts and all cell caps remain unchanged.

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
