# Snapshot consumer audit and terminal runner release

The four observed allocation paths correspond to real independent consumers.
They cannot be collapsed into one mutable snapshot without changing observation
boundaries. The first bounded change releases a completed runner's private
working snapshot after the entire terminal tick returns, rather than narrowing active
navigation or script views.

## Consumer findings

| Owner | Readers and publication boundary | Decision |
|:---|:---|:---|
| Panel `Session::nav_states` | Callback rebuilds the snapshot and its `WorldState`; UI routing reads gating facts and bank rows. Walk follow and BankBudget read the published snapshot. | Retain current families pending a narrower consumer contract. |
| Host `Slot::snapshot` | `after_drain` rebuilds dirty families; the guardian reads the host view in `client_frame`. | Preserve host pump ordering and guardian view. |
| Host-play `nav_snapshot` | Rebuilt on script tick edges; feeds script snapshot materialization, navigation, wire dispatch, and capture instrumentation. | Preserve the complete script-visible surface and tick gating. |
| `ScenarioRunner::snapshot` | Read by seeding, predicates, actions, navigation, shot callbacks and terminal evidence. `Done` returns before rebuilding or reading it. | Release after the whole terminal tick returns. |

Panel navigation requires more than inventory/stat gates. `WorldState::from_snapshot`
uses inventory, equipment, stats, varps and quest statuses; Traveller also reads
scene collision, placed locs, stun/tick/chat state, dialogue choices, modals,
widgets and side tabs. In particular spell teleports search the magic tab,
glider transport searches live widgets, and component resolution scans open roots
and side tabs. Removing widget families wholesale would change these behaviors.
The native widget/loc vector totals therefore describe candidates for ownership
work, not wholly redundant bytes.

The script observation snapshot is similarly not just a navigation cache: the
same value supplies the public script snapshot and dispatch paths. It cannot be
narrowed merely because a particular Thiever run does not use every API family.

Relevant source anchors: `crates/panel/src/session.rs` (`nav_states`,
`nav_snapshot_for_follow`, `focused_walk_state`, `focused_walk_bank`);
`crates/nav/src/world_state.rs` (`from_snapshot`);
`crates/nav/src/traveller.rs` (`spell_button`, `find_component`, transport follow);
`crates/host/src/lib.rs` (`Slot`, `after_drain`, `rebuild_dirty`);
`crates/host-play/src/lib.rs` (`nav_snapshot`, `observe_rebuild_snapshot`);
`crates/scenario/src/runner.rs` (`tick_with_hold`, `finish_pass`, `finish_fail`).

## Implemented boundary

Commits `122ad32` and `0e909b9` assign a fresh `GameSnapshot` at the outer
`tick_with_hold` boundary if the completed tick leaves the runner Done. The terminal shot callback runs first with the original
snapshot. Evidence owns its terminal tile, inventory/name rows, stat, chat,
predicate, message, outcome and timing; it does not borrow the working snapshot.

There is no public snapshot accessor or restart method on ScenarioRunner.
Public post-terminal status and evidence reads use the retained evidence.
`armed_route` and `current_aim` use route/traveller state, which is unchanged.
Companion hooks use their client and are unchanged. A follow failure can still be followed by an arm/budget check in the same tick,
including another failure callback. Cleanup waits for this existing sequence to
finish; no early return or error-behavior change is introduced. Subsequent
tick bodies return immediately. No navigation world, route, screenshot sink, evidence
or error is cleared.

This applies to terminal ScenarioRunner instances generally, including the
benchmark's retained seed runners. Production bot slots' snapshots remain
unchanged. It removes a demonstrated harness owner; it is not a production-client
memory saving claim.

## Regression verification

The new `terminal_snapshot_is_released_after_shot_and_evidence` test first failed
at the retained-snapshot assertion with original code. For PASS and FAIL it
checks that the terminal callback sees the original serialized snapshot,
evidence retains terminal tile/scene/run energy and the original failure, and
later held/unheld ticks neither change evidence/status nor fire another shot.
The private snapshot returns to its fresh serialized state. That assertion
checks state; live allocation capture verifies actual retained allocation removal.

All 78 scenario tests and 134 host-play unit tests plus enabled integration tests
passed (`memory-profile-no-alloc`). The saved panel release build succeeded.
Client source is unchanged, so client integration tests are not affected.
No claim of full campaign acceptance or whole-branch grok-4.6 approval is made.

The first code review caught the same-tick follow-failure continuation. A second
regression reproduced the premature cleanup: callback tiles were `[Some(tile),
None]` instead of the existing `[Some(tile), Some(tile)]`. Deferring release until
the outer tick returns restores both callbacks and the final budget evidence.
The initial active live attempt failed an XP startup proof before observation;
it is retained as a failed run and supplies no allocation comparison.

## Live allocation verification

The final code review approved `317b5ec..0e909b9` after checking the same-tick
continuation correction. The new saved binary is `runner-boundary-build/
panel-play-system`, SHA-256
`df54511ef56a2ad31b2a9d3aae76ffda07e774735e1adcf42bb808f9773f74ae`.
Client sources, navigation pack and catalog inputs match the prior targeted run.
Full metadata, capture receipts and selected native groups are retained in
`runner-release-live.json`.

Diagnostic `20260906T161339Z_panel_n32_active` completed with exit 0, qualified
with no errors. All 32 bots stayed ready/active throughout observation and gained
2–17 steals. It uses one drawing slot, 30s warmup, 120s observation and 60s teardown
with lite stack logging. Sampled observation spans 118.838s. Both active and
post-Stop VM/stack captures completed successfully in their intended phases.
The earlier `20260906T160707Z_panel_n32_active` attempt failed the initial XP
proof for one bot before observation; it supplies no passing measurement.

The original active capture had **48,381,952 bytes (46.141MiB)** of WidgetView/
LocView vector allocations on the ScenarioRunner path. The corrected active and
post-Stop captures have **zero matching allocation records** on that path.
The active report contains 21,538 allocation groups overall. Other snapshot
owners remain present and vary with workload; no claim is made that all their
allocations have identical sizes across runs.

This establishes removal of the selected runner-owned vector allocations, not
46.141MiB of RSS savings or the entire size of its former snapshot. It does not
establish production-slot memory savings. Stack logging consumed 14.43 CPU cores
and perturbs allocator layout; a clean comparison is required for RSS and latency
acceptance. The candidate remains unmerged. Broader family narrowing, shared
snapshot publication and other consumer changes were not implemented.
