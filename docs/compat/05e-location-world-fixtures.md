# Location and world catalog fixtures

This bounded extension keeps the frozen DoorOpener, GnomeCourse and FlaxPicker
cards and script source unchanged. Five shared scenario names apply to both
frozen catalogs on revisions 274 and 289:

- `door_opener`: `stand=3208,3212,0`, default `obstacle='door, gate'`.
- `door_opener_gate`: `stand=3213,3260,0`, `obstacle='gate'`.
- `gnome_course`: source default obstacle CSV and `searchRadius=20`.
- `gnome_course_radius`: `searchRadius=8`, same complete obstacle order.
- `flax_picker`: default Seers field/gate/bank tiles.

Selected 274/289 loc scripts and maps: wooden door 1530/1531 Open/Close at
(3208,3211,0) on m50_50; wooden gate 1551/1552 at (3213,3261,0); flax loc 2646
op2 Pick and item flax 1779; gnome dests from `gnome_course.rs2` (log z-7,
climb-down 0_38_53_55_28 = 2487,3420,0, pipe 154/4058).

DoorOpener only clicks a shut candidate within one tile of the player, so both
door cases inject an adjacent stand. Seed Close of an open leaf must finish
before Start. After Start the independent witness needs that selected loc to
become the open id through a same-session loc change. Script counters, a queued
Open, or an unrelated loc elsewhere fail.

GnomeCourse must show the log dest, the selected ground return, the pipe, then
further Agility XP back at the start tile. Snapshot `tile.level` is always 0 and
the loc sweep is `minusedlevel` only, so plane is not a native player fact;
ground return is the packed climb-down coord. `searchRadius=8` cannot see the
log from the pipe exit (chebyshev 13 on selected maps). That is a foreign
option/re-sync finding; the host and script are not rewritten. If a live cell
cannot finish a lap inside `SCRIPT_GOLD_DEADLINE`, report the measured reason
rather than stretching the watch.

FlaxPicker starts empty at (2741,3444,0). The witness needs exact 1779 x28, those
script-created stacks in a later loaded Seers bank generation, return to the
field, then further 1779. No seeded flax. Display name Flax is shared with
cert_flax 1780.

Panel and host-play keep using `scenario::get` / `names()`.

## Verification

Implementation baseline: host `a4157243817d8606f6c22a5ca2583f106d98bb85`,
client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`, plus these owned files.
Frozen export: `/Users/acfrazier/experiments/274bot/.worktrees/t_42719b42-src-a4157243`
(git archive of that host + client archive + owned overlay; frozen catalog
inputs linked read-only). Isolated empty target:
`/Users/acfrazier/experiments/274bot/.worktrees/t_42719b42-target-a4157243`
(`isolated_build=true`). Shared campaign target was not used. Frozen host
`a4157243` already contains the inventory use-on identity commit; this
overlay did not need a further useOn change.

- `cargo test -p scenario` — 85 passed.
- `cargo test -p host-play --features memory-profile --test catalog_boundary_live`
  — 19 passed, 1 ignored (`LIVE` cell).
- `cargo clippy -p scenario -- -D warnings` — passed.
- `cargo clippy -p host-play --features memory-profile --test catalog_boundary_live -- -D warnings`
  — passed.
- `cargo fmt --check -p scenario` — clean.

No LIVE or fixture process was launched. Root owns the twenty catalog x
revision cells after review.
