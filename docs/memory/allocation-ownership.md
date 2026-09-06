# Allocation ownership investigation — 2026-09-06

The bounded runner-sharing and explicit-Stop fixes are now implemented. See
[reassessment](shared-nav-stop-reassessment.md) for tests, native cardinality
verification, clean measurements and the unresolved CPU/work variation. The
findings below describe the original capture.

The largest apparent per-bot memory cost is **benchmark overhead**: each scenario
runner decodes and retains its own navigation pack. The live stack report groups
2,362,141,824 allocated bytes under the 32 `ScenarioRunner::new →
NavWorld::load_pack` call paths: **70.397 MiB per runner**. The application host
also loads one navigation world and shares that world across its slots.

The prior ~90 MiB idle and ~103–105 MiB active finite differences include this
per-runner ownership. They must not be presented as production client costs.
Their raw RSS and workload qualifications remain historical evidence of the
recorded configuration, but the harness needs correction before further baseline
or optimization comparisons. Do not subtract malloc bytes from RSS and call the
result a corrected measurement.

## Native evidence and ownership

Diagnostic `20260906T115411Z_panel_n32_seeded-idle`, saved seeded-idle System
binary `33ec5346…f13f8`, with `MallocStackLogging=1`. At setup, `malloc_history
-allBySize` captured live allocation stacks (the local man page specifies live
malloc and anonymous VM allocations, not cumulative allocation traffic).

| Scenario-runner call-path group | Calls | Reported live bytes |
|:---|---:|---:|
| Collision walk-plane allocation | 32 | 2,084,569,088 |
| Collision blocked-plane allocation | 32 | 260,571,136 |
| Other pack decode/index allocations, 13 groups | — | 17,001,600 |
| All 15 identified runner navigation groups | — | 2,362,141,824 |

These groups are disjoint records within the same native report, not a sum of
RSS/V8/GPU metrics. Allocator-reported block bytes may differ from requested
capacities. A later settled `heap -s --noContent` capture independently shows
**33 allocations of 65,142,784 bytes** in the navigation allocation class, plus
the blocked/index class. The report also contains a distinct `Play::new →
NavWorld::load_pack` stack with one allocation of 65,142,784 bytes.

Code matches the native stacks:

- `crates/host-play/src/memory.rs`, `Run::prepare`: constructs one
  `ScenarioRunner::new` per seeded account, stores each in static `SEEDS`.
- `crates/scenario/src/runner.rs`, `ScenarioRunner::new`: unconditionally loads
  the standard navigation pack and wraps that newly decoded world in an Arc.
- The runner retains `nav_world` and its final `GameSnapshot` after reaching
  `Phase::Done`; `tick_with_hold` returns without freeing those fields.
- `Play::new` separately loads its world. `Play::world` and ordinary bot routing
  already share that host-owned Arc.

An Arc inside each runner does not make separate constructor loads share data.
Dropping navigation entirely is not behavior-preserving: even the seed gate
uses its bounds, in addition to route steps that require it.

Full stack enumeration briefly suspended the process during setup. The later
heap capture occurred with all 32 clients ready. This run was intentionally
terminated after collecting ownership evidence (exit -15); it is **not a
completed scale, baseline or latency result**. Raw reports, metadata and the
termination note are retained under its diagnostic directory. The tracked
`allocation-ownership.json` includes all 15 navigation stack groups.

## Retention after activity

A separate one-bot active diagnostic, `20260906T120024Z_panel_n1_active`, used the
same saved build with stack logging. It completed 120s warmup, a requested 180s
observation (179.137s sampled span), and 60s teardown, exit 0. Both qualification
boundaries were saved and the bot gained 21 steals. Profiling overhead makes this
ownership evidence only, not a new CPU/latency baseline.

The active native stack report contains:

- One 524,288-byte FlatBuffer builder allocation through
  `IsolateBuf::encode_snapshot_delta` (512 KiB).
- One 339,840-byte `SceneEntityFp` vector through
  `SnapshotFingerprint::from_input` (331.875 KiB), plus smaller fingerprint
  allocations.

`SlotScript::stop` joins/drops the isolate and compiled instance and changes run
state, but leaves `last_snapshot`, `last_world_id`, and `ipc` untouched. The
fingerprint owns vectors/strings; `ipc` owns a FlatBuffer builder. This is a
concrete retained-owner finding from code. It is not evidence that these owners
explain the full 202–243 MiB fleet RSS difference after Stop.

Both `malloc_history -forkCorpse` captures hit the 55s tool limit. The active
report contains allocation groups down to one-byte groups and a binary-image
footer, but the command's timeout is retained in the receipt. The post-Stop
allocation report is empty: **no paired native allocation-stack total is
claimed**. The complete VM summaries and sampler show no live isolate or
in-flight snapshots after Stop. Rounded VM malloc-zone allocated bytes were
331.7M during activity and 330.8M after Stop; those are not RSS or V8 ownership
measurements and were captured at different times from the stack snapshots.

Other real allocations visible in the 32-idle heap include client construction,
scene squares/ground data, API snapshot vectors and renderer share-light data.
For example, native stacks identify 32 client-construction blocks of 4,325,376
bytes each. Their exact fields and further savings are not established here.
Do not redesign rendering or entity tables based on this coarse attribution.

## Recommended bounded changes

1. **Correct the benchmark first.** Load navigation once for the benchmark's
   runner set and inject cloned Arcs through existing
   `ScenarioRunner::with_world`. A later integration can reuse `Play::world`
   directly. Preserve per-runner snapshots, travellers, mutable state, missing
   pack handling, seed bounds and proof behavior. Avoid a process-global cache
   that could hide path changes across runs. Sharing among runners alone removes
   31 redundant owners, representing 2,182.317 MiB of reported allocations in
   this capture; that is an ownership estimate, not a promised RSS saving.
   Test shared pointer identity, independent runner progress and unchanged
   missing-pack behavior, then rerun qualified 1/32 controls with native live
   allocation counts to confirm one runner-set pack rather than 32.
2. **Release explicit-Stop storage independently.** After isolate teardown,
   release fingerprint, world identity and encoder backing allocation. Keep
   pause/resume storage, logs and errors as specified in the memory plan. Test
   Stop/restart keyframes, pause retention and actual allocation release. Use a
   successful paired allocation capture to quantify this change independently.
3. **Reassess remaining owners after those corrections.** Completed seed-runner
   snapshot retention is also harness-owned, but changing runner teardown must
   preserve proof/diagnostic data. Investigate client/API allocations and longer
   lifecycle retention with the corrected harness before proposing other work.

No runtime code changed in this investigation. No optimization, percentage saving,
or final whole-branch grok-4.6 approval is claimed. The controls and three-run
reports now carry an ownership correction so these findings do not get lost.
