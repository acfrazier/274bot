# Current candidate N1/N16 allocation attribution

The current captures identify growing per-client construction and snapshot allocations while shared navigation, animation, sound and unpack totals stay fixed. The next bounded proposal is to box occupied player-appearance packets rather than reserve full `Packet` storage in every empty appearance slot. This report does not claim RSS savings or authorize a new acceptance result.

## Evidence and limits

Batch `diagnostics/incremental-owner-attribution-20260907T095008Z/batch.json`: frozen candidate TUI SHA `b94bef2b437b649418dd3285f657ff77838e16d0696ceff8146aa38f37b27340`, client source commit `451759f2a7df9c57895657d5b8d506172860cee1`. N1 and N16 real TUI active sustain, profiles OFF, no input probes, stack logging **lite**, 30s warmup/180s observation, existing normal teardown. One `vmmap -summary` and `malloc_history -allBySize` capture per PID after a fixed 60s observation delay; both commands finished within observe with exit0 and recorded before/after samples and SHA256. Both frontends completed exit0 and independently bound workload qualification (`n1-bound.json`, `n16-bound.json`); minimum steal gains 28/9. Raw runs `20260907T095117Z_tui_n1_active` and `20260907T095653Z_tui_n16_active`.

These are perturbed allocation diagnostics. Native tools, capture watcher, campaign audit/review activity and unrelated user/OS activity are explicitly allowed in this batch metadata; capture helpers are separately recorded outside managed six-role series. No timings or resident values from this batch are clean comparison results. All source builds for the queued TUI fix were held until capture completed. No new candidate binary was built for this attribution.

Reproduce stack grouping with `python3 docs/memory/diagnostics/incremental-owner-attribution-20260907T095008Z/analyze_owners.py`. It verifies capture hashes and observe membership, parses both singular `call` and plural `calls`, normalizes only addresses, and retains full raw stacks in `owner-differential.json`. Exclusive family rules separate VM/thread mappings first, avoid inferring interface ownership merely from an `IfType` generic argument to `Client::from_shared`, and avoid attributing all descendants of `client_frame` to snapshots. Family totals remain stack-filter evidence requiring source interpretation.

## Allocation families (MiB, not RSS)

| Family | N1 | N16 | N16 minus N1 |
|---|---:|---:|---:|
| Client construction, excluding other identified families | 5.727 | 90.469 | 84.742 |
| Client world/build paths | 8.298 | 66.432 | 58.134 |
| Snapshot WidgetView | 3.235 | 53.878 | 50.643 |
| Snapshot LocView | 1.875 | 28.594 | 26.719 |
| Other explicit API snapshot paths | 0.629 | 10.487 | 9.858 |
| V8 non-mmap stacks | 1.251 | 16.997 | 15.747 |
| Other heap stacks | 11.183 | 57.819 | 46.636 |
| Navigation pack | 70.494 | 70.494 | 0 |
| Animation frame | 26.521 | 26.521 | 0 |
| Sound JagFX | 13.301 | 13.301 | 0 |
| Interface templates | 8.565 | 8.565 | 0 |
| Other shared client unpack | 5.290 | 5.290 | 0 |

The largest individual non-VM constructor group is 4,719,536 bytes/6 allocations at N1 and 75,512,576 bytes/96 at N16 (exact 16x). Navigation has one 62.141 MiB decode group plus one 7.781 MiB sibling in each process; shared-nav ownership persists. These captures do not support multiplying animation unpack storage by bot count.

VM/thread mapping stack totals increase by 1213.188 MiB but are **not RSS** and are excluded from the heap-owner ranking. vmmap reports the same 75.4M resident `MALLOC_LARGE` and 70.0M `MALLOC_LARGE (empty)` rows in both captures; the empty region is not attributed to a live allocation owner here. Malloc zone total resident is 199.6M/600.5M and allocated 156.4M/448.1M in vmmap display units. These zone and footprint numbers are diagnostic, do not sum to process RSS, and are not subtracted from clean resource results. Retained free pages/fragmentation remain a separate unproven opportunity.

## One bounded proposal: sparse appearance packet ownership

`Client::player_appearance_buffer` currently has 2,048 `Option<Packet>` entries (`client/client.rs:356,939`). `Packet` contains `random: Option<Isaac>` inline (`io/packet.rs:50`); `Isaac` contains two 256-element i32 arrays (`io/isaac.rs:5`). Even a `None` appearance slot reserves full inline Packet layout.

The batch includes `appearance_layout.rs`, emitted `appearance_layout.ll` and `appearance-layout.json`. `rustc 1.98.0` compiled `size_of` against actual cached client rlib `libclient-30b7668a1d9e764e.rlib` (SHA `fc2121cd25e163df8b4797825354b14666fa76c449c7c59f1ce74b8e8dd098f6`) without linking or running another client. On this arm64 layout: `Isaac=2064`, `Packet=2112`, `Option<Packet>=2112`, `Option<Box<Packet>>=8` bytes. The 2,048-slot empty table therefore reserves 4,325,376 bytes versus 16,384 bytes when boxed: **4,308,992 bytes (4.109375 MiB) structural difference per client**. This explains most of the large constructor allocation group, but individual live addresses were not captured to isolate that field within its grouped stack. This is not a measured 4.109 MiB RSS win; populated packet boxes and allocator overhead are additional and actual occupancy was not measured.

Proposed scope: `Vec<Option<Box<Packet>>>` for this one table; box a received appearance once, keep the existing take/update/restore behavior and index semantics, and drop entries at existing login/reset points. Keep `Packet`/ISAAC representation and wire crypto behavior unchanged. Touch only necessary client call sites and fixtures (not host action APIs or client routers). The table remains 2,048 slots; populated entries retain independent ownership. Fully populated worst case adds pointer/box allocator overhead compared with inline storage, so sparse workload qualification matters. This is a proposal pending independent review and implementation, not a committed representation change.

Verification for the implementation: retain real PLAYER_INFO appearance decoding, cached appearance reapplication when a player re-enters, packet cursor/reset behavior, and login reset/drop semantics. Existing client tests `player_info`, `login`, `server_packets`, `gens` must run separately from host tests; add a meaningful cached-appearance remove/re-entry regression if absent. Run affected host crate tests and the required live qualification. Freeze matched pre/post binaries with common instrumentation and measure actual clean N1/N16 RSS/CPU before keep/park. Do not substitute compiler layout arithmetic for resident savings. The anticipated scale cannot close the N16 target gap alone.

The TUI input-origin instrumentation task `t_3ec8a043` is independent. Existing shared-nav acceptance remains unproven on latency; this diagnostic does not erase those failed classifications or remove final Linux/panel/lifecycle/whole-branch requirements.
