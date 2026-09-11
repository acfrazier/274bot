# Native sequencing observation transfer and bank approach

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 14:14 UTC. Kind: bounded read-only source/finished-log
audit for brief 133. Not implementation, LIVE, fixtures, ledger, STATE,
dim, Git staging, merge, or push. Root owns any follow-up. No new tasks.

Read once: `AGENTS.md`, `docs/execution.md`, briefs 133 and 123, report
09 / 09a, and 04k composition. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report and `docs/compat/evidence/native-sequencing-observation-cost/`.
`bank.js` / `bank_open.rs` / `cake_stall.js` / `cake_stall.rs` are
byte-identical to host `21446414d`. Later HEAD movement is unrelated
(retaliate identity in `load.rs` / `host-play`).

## Verdict

**Bulk JS-to-Rust observation copies are real and unnecessary on the
BoneBurier world-nearest path and on every cake `delayUntil` poll.
They are not a measured cause of isolate tick time, and they are not
the cause of the BoneBurier 60 s open timeout.**

Old-catalog ArdyCakes on the same `21446414` export FAILs on both
revisions after steal attempts: isolate `terminate_execution` of an
over-budget tick, then `execution terminated` / `Unknown error`. Slow
ticks of ~230–390 ms coexist with the cake loc/inv JSON roundtrip.
That is not matched proof that the copy caused the FAIL or the
millisecond values. Root already queued 134 to remove bank/cake bulk
roundtrips with native compact observations.

`Bank.openNearestWorld` already owns approach in Rust. JavaScript still
hands the live `s.locs` and `s.banks` arrays into
`__rs2b0t_bank_open` on every `delayUntil` pump. Rustyscript JSON-encodes
those arrays; `bank_open::read_observation` always allocates a `Vec<Booth>`
with owned names/actions, even in `WaitNearest`, which only reads
`nearest_booth`, `here`, bank flags, and the 60 s Instant. Cake stall
does the same with `s.locs` and `s.inv`. PeriodicBank is the control:
it already sends `has_booth_stands` as a boolean.

The four finished `21446414` BoneBurier receipts (274/289 × both
catalogs) all PASS after a catalog retry. At the `'could not open a
bank — retrying'` line the traveller is still mid-route, not adjacent.
After the second `WalkNearestBank`, follow `Arrived` at `(3269,3170)`
and `OpenBooth` uses scene loc `(3268,3169)` id 2213 (Chebyshev 1).
Walk reach radius 1 and booth adjacency 1 matched on the successful
open. Do not restore the 120 s JS wait.

The two finished old-catalog ArdyCakes receipts FAIL (exit 1) after
steal attempts: isolate `terminate_execution` of an over-budget tick.
Do not treat any of these elapsed times as a performance result: every
receipt sets `elapsed_is_not_performance_measurement: true`, and cells
overlapped.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Log / export host | `21446414d21b50088aabf100df4392c3674b89fa` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Headed binary SHA-256 | `65b25315a3d0b049e37582fe42ca8b4cc3926d7c46979c8cb433b7b1edd18b4b` |
| Campaign HEAD at write-up | `025e31281aae64a3ec944bef532b42eabfd8dbd2` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 133 SHA-256 | `fcf26e34b1974751e25f11bcf50cc4b1dd7afbed28b51d9b4b302b3c56e8960e` |
| Kanban card | `t_97028e08` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| `bank.js` | sha256 `52ee4c0ff2d33d323c07891e1d6cf87651bfada476a20759a4548ab278da7166` |
| `bank_open.rs` | sha256 `06c965780fbd818c82cfbda0a9ad1db0408ce8a3010fdcdef8f8ff73c0890dc5` |

Per-file hashes and receipt identities:
`docs/compat/evidence/native-sequencing-observation-cost/refs.json`.
Log scans: `bone-21446414-scan.json`, `ardy-cakes-21446414-scan.json`.

## 1. Exact transfer/copy path

Pump (`execution.js`): `delayUntil(cond, 0)` parks with no JS timeout.
`__rs2b0t_pump` runs the cond on every posted PLAYER_INFO tick.

Bank (`bank.js` 19–30, 33–70): `bankOpenObservation()` builds

- scalars: `ingame`, `here`, `bank_open`, `bank_loaded`, `bank_generation`, `nearest_booth`
- bulk: `locs: s.locs`, `banks: s.banks` (array references, not a JS deep clone)

then `rustyscript.functions.__rs2b0t_bank_open({ op, token, observation })`.
`load.rs` registers that function as `|args: &[serde_json::Value]|`.
The bulk copy is V8 → JSON → `read_observation`:

- every loc becomes a `Booth` with owned `name` / `actions` (`bank_open.rs` 459–503)
- `banks` is walked only for `has_booth_stands` (any `kind == "booth"`)
- scene loc objects also carry health/combat/animating/reachable fields
  that bank-open never reads (`load.rs` `scene_entity_object`)

FB snapshot materialize already publishes `locs` / `banks` into V8 when
those deltas change (`isolate_fb` `DeltaMask.locs` / `banks`). The 123
controller adds a second walk of the same live arrays back into Rust on
every wait poll.

Cake (`cake_stall.js` 24–46, 119–126): same pattern with `s.locs` and
`s.inv`. `cake_stall::read_observation` always owns every loc name/actions
and every inv `(name, count)` (`cake_stall.rs` 375–443), then
`selected_stall` rebuilds `StallLoc` views (`319–343`).

PeriodicBank (`periodic_bank.js` 22–34) is already the thin observation:
`nearest_booth`, flags, and `has_booth_stands: (s.banks || []).some(...)`.
No loc array crosses the JSON seam.

## 2. Phase-specific facts actually required

| Controller / phase | Needs | Does not need |
|---|---|---|
| Bank `begin` / `WaitNearest` (world) | `ingame`, `here`, `nearest_booth`, bank flags, `has_booth_stands` bool | `locs`, full `banks` |
| Bank `WaitReady` / `WaitFresh` | bank flags + generation | `locs`, `banks`, `nearest_booth` |
| Bank named `begin` / `WaitStand` select | current `locs` (id, tile, name, actions, distance) | `banks` |
| Bank `WaitSelected` identity | **fresh** `locs` at arrival (`selected_still_present`) | retained begin-time loc list |
| Cake `WaitStand` | `here`, `ingame`, 60 s Instant | `locs`, `inv` |
| Cake `start_steal` | current `locs` for `select_baker_stall` | — |
| Cake `WaitSteal` / count / restock | `inv` names+counts, combat/abort | `locs` |

Stale retained loc views are not acceptable. Named identity must re-read
the current snapshot at arrival. World-nearest never needed the lists
after `begin`, and `begin` only needed the booth-stand boolean.

Native bounds already in source and to keep: walk 60 s, bank ready 5 s,
steal 2400 ms, autocast 3 s. Pause/hold freeze Instants; reset/abort
bumps the token. JS timeout 0 is the 123 correction (removed 120 s).
Leave that alone.

## 3. Proven copy work vs unmeasured latency

Proven:

- every parked pump JSON-encodes live `locs`+`banks` (bank) or `locs`+`inv` (cake)
- Rust allocates owned strings for those rows on `begin` and every `next`
- isolate `SLOW_TICK` is 50 ms (`load.rs` 921); a slow tick also skips
  stale queued ticks

Finished-log counts (isolate `slow tick N: Xms`, all ≥ 50 ms):

| Cell | slow_n | median ms | min | max |
|---|---|---|---|---|
| r274 old | 84 | 58.49 | 50.14 | 72.69 |
| r274 newer | 87 | 59.05 | 50.22 | 71.90 |
| r289 old | 80 | 57.68 | 50.03 | 66.60 |
| r289 newer | 87 | 58.42 | 50.27 | 64.38 |

Two clusters per log: first walk until catalog retry, then the later
approach/open. Host `d_observe_us` / hitch `observe_us` (typically
14–27 ms per hitch in the r274-old walk) is a **different clock** from
isolate elapsed.

Not proven, and must not be claimed:

- that the JSON loc copy **is** the 50–60 ms
- any matched performance delta
- attribution across concurrent headed cells (old pair started
  14:02:38/14:02:39, newer pair 14:04:57/14:05:00; all four receipts
  refuse performance use)

The copy path is a correctness/efficiency defect relative to “do not
deep-copy the world on every read.” It is not a measured win yet.

## 4. 60 s timeout vs walk radius vs booth adjacency

BoneBurier `restock` → `Banking.bankNearest` → `Bank.openNearestWorld`
(`banking.js` 80–82, `BoneBurier.ts` 105–112). Rust `Mode::NearestWorld`
queues one `walk-nearest-bank` and waits 60 s Instant for
`near(here, nearest_booth)` with `ACCESS_RADIUS = 1`.

Host `WalkNearestBank` (`host-play` 1713–1734) routes with
`route_with_radius(..., 1)` onto the packed booth tile from
`nearest_bank_booth`. Follow then uses `close_enough: 0` onto that
approach tile (`3488–3491`). Logged `[nav-walk] radius=2` is mid-hop
click-ahead (`traveller.rs` `CLICK_AHEAD = 2`); the last hop is
`radius=0`.

r274-old (`r274-bone-burier-100adccc-21446414.log`), representative of
all four:

1. Tick ~38: first `WalkNearestBank` from `(3220,3220,0)`.
2. Line 410, host tick ~123: `'could not open a bank — retrying'` while
   `[nav-follow] here=(3288,3284) walk=true ... hops=13`. Not arrived.
   Chebyshev to the later booth is still ~114 in z. Native 60 s expired
   mid-route. Catalog retries; it does not inflate the owner clock.
3. Line 413: second `WalkNearestBank`. Same dest key keeps the armed
   route (`route_with_radius` radius>0 reuse).
4. Line 655: `Arrived { at: (3269, 3170, 0) }` last hop radius 0.
5. Line 659: `OpenBooth { x: 3268, z: 3169, id: 2213 }`. Chebyshev 1.
6. Withdraw 28 Bones, bury continues. Predicate PASS.

So the timeout is **walk not finished in 60 s**, not **arrived outside
adjacency**. On the successful open, packed approach tile and scene
`nearest_booth` agree within radius 1. Do not restore 120 s as a fix.
Do not guess why the packed route from Lumbridge spawn to Al Kharid was
long (gate/graph); the logs do not name remaining hops or packed dest
at expiry.

**One bounded future diagnostic (bank):** on `walk-timeout` (and on the
transition to `open-booth`), log one line with `here`, `nearest_booth`
tile/id/distance or none, armed route dest, and whether follow is still
walking. That splits this mid-route class from a future
arrived-but-not-adjacent class. No LIVE in this card.

## 5. ArdyCakes old-catalog FAILs (finished 21446414 logs)

Operator addendum during this audit. Same headed export/binary as BoneBurier.
Old catalog only (`100adccc`); both revisions. Not LIVE from this card.

| Cell | exit | elapsed_s | log SHA-256 (prefix) | slow_n | ≥200 ms | median ms | min | max | terminal |
|---|---|---|---|---|---|---|---|---|---|
| r274 | 1 | 112.602 | `ea0b9d97792311b0…` | 143 | 93 | 238.01 | 75.85 | 471.80 | tick 177 `Unknown error` |
| r289 | 1 | 108.448 | `f3fed565edf73c53…` | 138 | 96 | 234.07 | 75.42 | 472.18 | tick 167 `Unknown error` |

Both receipts set `elapsed_is_not_performance_measurement: true` and
overlapped (pids 10467 / 10560, start 14:13:19 / 14:13:30).

Shared mechanical sequence:

1. Start on baker stand `(2668,3312,0)`. `stealCakes` posts
   `Loc Steal from` `(2667,3310)` id 2561 — the native one-attempt ABI.
2. Mix of `guard caught the steal` (combat → catalog flee `Walk` to
   `(2655,3298)`, then `WalkTo` back to stand) and many
   `steal pass made no progress — re-entering` (55 lines r274, 44 r289).
3. Isolate ticks during steal/re-enter clusters are mostly 230–390 ms
   (r274 max 471.80; r289 max 472.18). Flee-walk ticks in r274 drop to
   ~76–90 ms — still over `SLOW_TICK` 50 ms, below the steal cluster.
4. Last in-flight tick is interrupted because the next game tick arrived
   while it was still running past 50 ms (`load.rs` `on_game_tick` →
   `terminate.terminate_execution()`). Logs:
   `interrupted slow tick 176 (556.684542ms)` then
   `paint eval: Uncaught Error: execution terminated` then
   `tick 177: Unknown error` / FAIL (r289: interrupted 166 at 562.93 ms,
   tick 167).

Proven: FAIL is a script error after V8 `terminate_execution` of an
over-budget isolate tick, during catalog re-entry of the native steal
ABI. Cake `observation()` still JSON-copies full `s.locs` and `s.inv`
on every pump of that ABI (`cake_stall.js` 24–46, 119–126).

Not proven: that the loc/inv roundtrip **is** the 230–390 ms, or that
removing it would have prevented terminate/FAIL. Host hitch
`observe_us` in the same windows is a different clock (~18–23 ms on the
r274 tail). Do not treat these elapsed seconds as a matched result.
Do not dim ArdyCakes from this audit.

## 6. Smallest Rust-owned snapshot/borrow/view seam

Do not move `select_named` / `select_baker_stall` back into JS.
Do not cache a world clone.

Recommended seam, isolate-thread only:

1. During the existing FB materialize/pump, fill a TLS view from the
   current `SnapshotReader`: `here`, flags, `nearest_booth`,
   `has_booth_stands: bool`, and borrowed loc/inv slices already in the
   buffer. Clear it in `on_reset` with the rest of the controller.
2. JS `begin`/`next` send only `op`, `token`, mode, stand, wanted
   name/action. Stop putting `locs`/`banks`/`inv` on the JSON payload.
3. Inside Rust, read lists only in the phases in the table above.
   `WaitNearest` never walks locs. Named identity uses the **current**
   view at arrival, not a begin-time copy.

Smaller world-nearest-only patch, if a first hop is wanted:
mirror PeriodicBank — JS computes `has_booth_stands` and omits `locs`
and `banks`. That is enough for BoneBurier `openNearestWorld`. It does
not fix named `WaitSelected` identity polls or cake `WaitStand`. The TLS
view does.

Preserve selected identity, per-isolate clearing, 60 s / 5 s / 2400 ms /
3 s, and Pause/hold freeze. No new opcode. No foreign JS controller.

## Sorted findings

1. **World-nearest polls serialize unchanged bulk lists.** Proven in
   source. Fix with the TLS view or the PeriodicBank-shaped observation.
2. **Cake polls serialize full `locs`+`inv` on every steal/wait pump.**
   Same seam. 134 is the implementation card.
3. **60 s bank timeout is unfinished packed walk, then catalog retry, then
   adjacent open.** Proven in finished BoneBurier logs. Do not inflate
   the bound.
4. **BoneBurier 50–60 ms isolate ticks coexist with the copy path but are
   not attributed.** Receipts forbid performance use.
5. **ArdyCakes old-catalog FAIL is isolate terminate of an over-budget
   tick after steal/no-progress re-entry.** Proven log sequence. Do not
   assert the JSON copy caused the milliseconds or the FAIL.

No implementation in this card.
