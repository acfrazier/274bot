Retain the selected allocation improvements, gameplay/rendering fixes and reusable preservation harness from the memory campaign. Clients reuse completed sprite slots, avoid animation-delay clones, share private animation bases and store sparse appearances compactly. Explicit Stop releases snapshot storage, harness seeds share Play's navigation world, and panel CPU upload storage is allocated only when needed while retaining pixels across GPU/CPU switches.

Preserve the corrected Rust banking/loadout/food and routing behavior, exact adjacent door crossing, focused reconnect priority, modal/overlay/minimap restoration, and Windows socket/home/resource support. Unsupported compatibility helpers remain explicit errors. The opt-in TUI/panel fleet harness ships with real readiness/proof checks, terminal evidence and basic accounting. Experimental snapshot sharing, borrowed fingerprints, tiled navigation, advanced capture and campaign controllers do not ship. Product documentation describes the retained surface and requires rebaking existing v8 navigation packs.

Validation:
- macOS: 1,410 combined affected tests passed; selected client 75 library +100 separate integration tests; host124. Ordinary/counting/system-allocator frontends built. Final host/client all-target Clippy, harness-feature Clippy and formatting passed; post-lint client104 and API/Play151 tests passed; final harness35 passed.
- Windows/MSVC and Linux: affected native tests and ordinary/harness builds passed except the existing GPU shade-boundary test (9/10, expected223/observed255). Linux binaries were built in the existing Hyper-V VM, hash-verified and tested on Concord. The historical Windows unreachable-CRC deadline remains unresolved.
- Actual TUI smokes on all three OSes and native panels on Windows/macOS exited0. macOS GPU→CPU→GPU switching was visually inspected. Windows Stop/restart cleared isolate/snapshot counters and resumed ticks. No headed Linux desktop or universal backend claim.
- Fresh selected navpack baked and validated; contested-door test passed both login orders. All32 macOS sustained Thiever slots restocked exactly22 food, closed the bank, returned and increased successful steals. Controlled Traveller stun recovery, timed live focus priority and live modal/freeze intervals are not newly proven.
- One ordinary headless Play pair at N1 observed mean current RSS289.88→260.59MiB, with XP234→141; this is a short single-pair observation, not an equal-productivity CPU or general capacity claim. MainN32 qualified; selectedN32 was interrupted before measurement, with server save confirming a Maze destination. The failed artifact is retained; no N32 savings or recovery claim, and no favorable retry.

The required Grok4.6 whole-diff review approved the assembled code; closing review requested operator acceptance of the missing paired N32 performance result and documented limits, with no code change requested. The operator accepted publication and merge on September 10, 2026. Live/native artifacts bind host9527cc63/clientdaccb4ba. The final hostc5159d38/client9b41e6e adds product docs and behavior-equivalent lint cleanup, not relabeled native rebuilds.

Source mapping (selected/reconstructed code, not cumulative campaign merges):

| Retained group | Original campaign SHA(s) | Selected result |
|---|---|---|
| Client sprite reuse | ca0a36f | 4871cd8 |
| Client scalar delay | 12c2061 | fb68bd1 |
| Client overlay + frozen minimap | bc8bb4e,451759f | 037d461,3be5e7a |
| Client sparse appearances + private animation bases | 85266df,e17deab | cfd3d3d,9d6bcca |
| Client Windows sockets/home/lock | 35f1c13,4b35300,2b1af85 | b4ceb4d,56b1bb7,76a0f86 |
| Client modal | 3456edc | a5b2a4e |
| Required basic client profiling | b555f84 | d5f26e3 |
| Client format/lint | integration cleanup | daccb4ba,9b41e6e |
| Explicit Stop snapshot release | e65103b | b7357a07 |
| Focused reconnect | d32b493 | 1687b453 |
| Lazy CPU upload + retained pixels | daa97f7,c90053d | d3e177fc |
| Host Windows sockets/home/current RSS/resources | 9cdd4a2,1e33d28,8ee4bf3,c8b9334 | 2ef82b70,3bc804a7,d998a6fd,751f0853 |
| Rust bank/loadout/food, route ownership/radius/retry, bounded stun | selected a52b12c hunks | d6be8aff |
| Door progress/edges/adjacent packet, delayed inventory | 72804cd,b9f40fe,7992f68,94adac4,1211d0a; comments5e0f3ec | d6be8aff |
| Harness foundation/fixtures/basic counters | e6ec9e6,e1ebd59; selected a52b12c | a1b74051 |
| System allocator, seeded idle, render policy | c4943a7,d3abfb4,9f687af | a1b74051 |
| Shared seed nav, terminal cleanup | 7d026bc,606c93b,122ad32,0e909b9 | a1b74051 |
| Final TUI preparation fixtures | 4b1d0bb,273ecd4,f9675b6 | a1b74051 |
| Honest excluded navigation field | reconstructed selected boundary | ef840612 |
| Basic timing hooks/dependency resolution/docs/lint | selected a52b12c + integration | a5b5b0dd,9527cc63,c6d748bd,9d2a0f65,c5159d38 |

Client gitlink pins the selected history on `acfrazier/FR-client-bothost` branch `r274-bh-modular`. Campaign assessments, scripts, review reports and raw evidence stay on the campaign checkout/branch. This PR is intended for a squash merge onto existing main history; no release tag or history rewrite.
