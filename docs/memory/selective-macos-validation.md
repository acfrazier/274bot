# Selected macOS functional checks

Frozen code: host `9527cc636187fdfaaca29c3e4af14819dd40dbe9`, client
`daccb4ba3ff5f8d1fce5b3b9487a1f0576ad50ac`. Later host `c6d748bd` changes
product documentation only. Immutable system-allocator TUI/panel binaries and
SHA256s are recorded in `binaries/selected-build.json`.

## Automated checks

- Selected client library:75 passed. Separate client integration targets:
  100 passed (`seq_delay`, `player_info`, `login`, `inject`, `zone`, `iface_model`,
  `gpu_texture`, `gpu_memory_profile`, `render_backend`, `gpu_backend`). These
  preceded a formatting-only client commit; no semantic client edits followed.
- Host library:124 passed, including focus/reconnect and socket/Stop wake tests.
- Combined API/nav/host-play/script/scenario/panel/TUI memory-feature suite:
  1410 passed across47 reported suites,0 failed,9 existing ignored live/opt-in.
- System-allocator harness unit tests:30 passed. Ordinary, counting and direct
  system-allocator release TUI/panel builds passed. Full selected rustfmt applied.
- A fresh lockfile regeneration attempted to resolve unrelated newest offline
  dependencies and hit the existing pinned serde conflict. The actual selected
  lockfile was instead updated from the existing pinned baseline by Cargo check:
  only four dependency entries added, with all locked builds/tests passing.

## Fresh pack and doors

Selected `nav-pack` baked483 mapsquares into dense collision,2143 transports and
68 bank stands. Fresh pack SHA256 `c9dca67bcdc8d0c8a598fb316279851658c1113e56bbb8dd5d81356cdf773f17`;
flags `92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb`.
Inputs: existing Server content maps/door configs and engine config cache.
The campaign bank-return checker was temporarily compiled against selected nav;
it rejected the obsolete Door1530 long crossing and found a bank-return route.
The temporary example was removed from selected source.

Real contested-door test passed normal login order in57.34s and reverse order
in55.32s. Both had two actual ingame/scene2 slots and reached exact(2817,3443,0),
with the closer active and the original180s terminal bound. The reverse run used
the same pre-format code binary; code semantics are unchanged in final selection.
Logs: `nav-bake.log`, `nav-pack-check-corrected.log`, `nav-door-{normal,reverse}.log`.

## Retained frontend harness

Direct product TUI runs used the saved selected system-allocator executable,
real120x40 Unix PTY, shared selected pack, normal600ms server, actual catalog
`100adccc037d9f6898080e1cad58fcfc43364775`, and explicit sustained fixture.
The campaign launcher only transports/records the retained binary. Its legacy
checkout SHA fields name the campaign, not the binary; the immutable binary hash
and selected-build manifest establish the actual selected source.

N1: `../diagnostics/20260910T024101Z_tui_n1_active`, exit0. Warmup30/observe120/
teardown60. Progress grew4 to14 steals during observation; inventory moved3 to
exactly22 and bank closed. Observation ended during the return walk, so this N1
cell alone is incomplete return evidence.

N32: `../diagnostics/20260910T024517Z_tui_n32_active`, exit0. Warmup30/observe240/
teardown60. All32 actual slots established readiness/proof and stayed active
through observation. Per-slot diagnostic inventory shows exactly22 food, then
bank closed, then later increased steals for every32/32 slot. No script errors.
Full per-slot times/positions: `tui-n32-functional-analysis.json`.

These are functional diagnostic runs, not accepted performance comparisons:
some builds overlapped, and one separate idle panel ran late in N32. The clean
common-driver comparisons have separate folders and eligibility results.
Spot animation245 occurred on all32, but the excluded detailed navigation log
means this does not establish Traveller's bounded stun recovery path triggered.
No live claim is made for that inferred path. Focus/reconnect priority has
specific deterministic queue/Play tests; these fleets establish functioning
mainland reconnects, not an independently timed live priority-order proof.

## Native macOS panel inspection

The first standalone CLI panel launch was stopped solely because the UI tool
could not select an executable without an app bundle identifier. A disposable
app bundle reused the byte-identical saved panel binary; no product code changed.
The replacement N1 seeded-idle run
`../diagnostics/20260910T025136Z_panel_n1_seeded-idle` exited0 after its normal
10/180/60 lifecycle. It overlapped the diagnostic N32 run and is not a benchmark.

Root actually read native window screenshots through the UI tool. The native
Ardougne scene, inventory and minimap were visible at ingame/scene2. Through
General config/render controls, the same logged-in client switched GPU to CPU
and back to GPU; the scene/minimap remained visible. Configuration controls
responded, and the client stayed in game. Those visual observations are in the
interaction record; no separate PNG was saved before the bounded run closed.
An attempted later F12 capture targeted the already-closed window and is not
screenshot evidence. No new live modal or scene1-timed freeze capture is claimed;
the selected renderer readback/integration regressions cover those invariants.

Windows native panel captures are retained separately and were also read by root.
They establish actual scene/minimap rendering in the selected binary, not a
modal-open screenshot, a captured freeze interval or universal GPU support.

## Closing lint and comparison

The ordinary host and separate client workspace/all-target Clippy checks passed
with warnings denied, as did the system-allocator harness feature check. Initial
lint failures and correction logs are retained. Final code is host `c5159d38`,
client `9b41e6e`: differences from the live/native freeze are mechanical lint
cleanup (test-module placement, Copy/Option idioms, equivalent minimap range
checks, a Default implementation, and default-settings initialization).
No protocol, timing, lifecycle or feature behavior changed. Both workspaces pass
format checks. The affected client library/render/modal and host API/Play tests
were rerun after the primary lint changes (104 client, 151 API/Play); logs are
`lint-{client,host}-regressions.log`. The final two harness cleanup edits received
35 passing focused tests in `final-harness-lint-tests.log`.
Native/live binaries remain explicitly pinned to the earlier semantic freeze;
they are not mislabeled as rebuilt final-SHA executables.

The [single bounded comparison](selective-comparison-results.md) qualified both
N1 cells. Selected mean current RSS was 29.29 MiB lower in that short pair,
with different XP productivity and no timing-percentile claim. Main N32 passed;
selected N32 failed before observation during a Maze teleport. The failed cell
and bounded diagnosis remain visible. No N32 performance result is accepted and
no favorable retry replaces it.
