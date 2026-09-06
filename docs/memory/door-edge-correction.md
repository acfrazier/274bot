# Adjacent door destinations — 2026-09-06

The captured bank-return bug was an invalid baked transport edge: Door1530 at
(2656,3292,0) jumped west to (2651,3292,0), skipping blocked counter, plant and
bench footprints. `door_far_side` now accepts only an adjacent standable tile.
Web footprint traversal retains the previous helper behavior. No Traveller,
script API, wire-format, or client rendering changes are included.

Implementation commits: `b9f40fe` and `7992f68`. The first reproducer failed on
old code, returning the five-tile landing. The final suite passes 253 nav library
tests and three nav-pack tests. Tests cover blocked neighbors, boundaries,
face-only wall flags, derived adjacent destinations, and retained web walk-out.
Grok-4.5 (`reviewer`) initially requested documentation/test corrections and
approved the revised source scope. Review receipts accompany this report;
whole-branch grok-4.6 review remains separate.

## Pack and binary provenance

Baking is explicit; a new binary does not invalidate an existing v8 pack.
`check_bank_return` fails against the original default pack and succeeds against
the rebuilt candidate, also proving the destination remains routable.
The diagnostic launcher now records the selected pack path/hash and flags path.

Candidate artifacts, relative to this checkout:

- `docs/memory/diagnostics/door-edge-build/274bot.navpack`
  SHA-256 `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`
- `docs/memory/diagnostics/door-edge-build/274bot.navflags`
- `docs/memory/diagnostics/door-edge-build/panel-play-system`
  SHA-256 `511881bd27af6a5784bc236721f8e234993578a6911bf0a4fedb48fd1a9d1384`

Panel was built from the implementation before committing; its title embeds
`4231362-dirty`. The launcher records source and binary hashes, rather than
relying on that title. The follow-up commit changes tests/comments only.
The shared `~/.274bot` pack is unchanged. Future tests must set `NAV_PACK` and
`NAV_FLAGS` to the candidate paths above to exercise this correction.

## Live findings

Captured fleet run: `20260906T011739Z_panel_n32_active`, 32 active Thiever bots,
one drawing slot, System allocator, diagnostics/debug/captures enabled,
one-second warmup and 300-second observation, then 60-second teardown.
This is functional diagnostics, not a memory baseline or CPU comparison.
Closer tests and review overlapped portions of this run, further excluding it
from performance acceptance.

Focused bot `live48e70_0` returned from the bank with generation 3: a single
47-tile walking leg from (2655,3286,0) to (2659,3304,0), with no door transport.
It started at tick214 and arrived at tick237, within radius2 of the requested
(2661,3306,0), then resumed thieving. The reduced trace is in
`bank-return-door-fixed-evidence.json`. The return-start PNG was visually
inspected; its pixels represent capture time, while its JSON preserves the
triggering event snapshot. Do not infer exact event position from delayed pixels.

The one-tick Catherby door closer FAILED on both candidate and original packs:
tick133, at (2816,3438,0), aiming (2816,3439,0), `Expired`, one try reported.
Both slots reached ingame/scene2 first. The two packs have identical door edges
at this location. Original-pack debug trace repeatedly submits Open and
walk-through, but the walker stays on the near tile. This is evidence against
the new pack causing this particular failure, not a passing closer result.
The raw logs are retained in `door-edge-build/closer-{candidate,original}.log`.

The bank-return false edge is corrected and exercised. Broader door stability
remains unaccepted. Investigate the closer's action/movement processing order
with bounded traces before another scheduling comparison; do not increase retry
budgets merely to hide the failure. No shared-pack replacement or memory saving
is claimed here.

## Final run qualification and additional blocker

The process exited 0. All 32 remained ready/active through observation, each
recorded positive steal progress (1–41; focused bot31), and teardown reached
zero active scripts, live isolates, V8 used bytes and in-flight snapshot bytes.
The existing qualification helper returns true, but its positive-progress rule
is insufficient evidence of sustained workload: slot18 gained only one steal.

Slot18 reached its return destination at tick280 (single Walk leg, no door),
but remained in “returning from the bank” until the final observation, with no
new requests and no in-flight snapshot. It had withdrawn28 lobsters despite a
22 target (Withdraw X19 was followed by Withdraw-10). This is a separate
script/completion or banking-state investigation; the traces do not establish
its cause. See `bank-return-script-stall-evidence.json`. Do not accept this as a
stable CPU/memory baseline merely because the current harness says qualified.

Next bounded work: trace the return completion delivery for slot18 and the
withdrawal fallback, and instrument the closer's server-side processing order.
Keep those changes separate from the reviewed door-edge fix. Scheduling
comparison remains pending these stability findings.
