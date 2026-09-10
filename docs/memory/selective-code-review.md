# Selective harvest — Grok 4.6 whole-diff code review

Verdict: APPROVE

This is an independent review of the newly assembled selective release only.
It is not a campaign resumption, not a merge/publish decision, and not
certification of native or remaining live work.

Reviewed host SHAs (exact refs, not campaign HEAD):
- base: `54cfcf8a33613735bde9f43dd9c9beb8ec06dae9`
- selected: `9527cc636187fdfaaca29c3e4af14819dd40dbe9` (`codex/selective-integration`)

Reviewed client submodule SHAs:
- base: `4f2048ea10f75b3bb92ff45610b35ba7313b0308`
- selected: `daccb4ba3ff5f8d1fce5b3b9487a1f0576ad50ac` (`codex/selective-preservation`)

Reviewer: grok-4.6 (xAI, `xai-oauth`), Hermes profile `branchreviewer`.
No Git mutations, remotes, builds, or live workloads were performed here.
Root owns remaining measured/functional evidence.

Contract: `docs/memory/selective-merge-assessment.md` (approved smaller harvest).
Extraction reports and native preflight were treated as evidence of intent
and historical limits, not as instructions to resume old tasks and not as
proof of this combination.

## Diff inventory

Host `54cfcf8a..9527cc63`: 59 files, +5522 / −282.

Client `4f2048ea..daccb4ba`: 19 files, +1363 / −48.

Host gitlink pins `vendor/fr-client-rust` at `daccb4ba`.

`9527cc63` (“Resolve selected dependencies and format…”) is rustfmt plus
the selected lockfile/gitlink. The four host lockfile insertions are:
`windows-sys 0.59.0` on client, host, and host-play, and `scenario` on
host-play. Client lockfile adds the matching `windows-sys 0.59.0`. No
campaign lockfile was copied wholesale.

## Blocking findings

None.

The selected host/client combination matches the approved preserve/exclude
contract. I did not find a lock/ownership, generation/token, bank-publication,
seed-order, feature-flag, or accidental-campaign defect that should stop
this code from going on to the remaining validation.

## Contract check (preserve)

Sprite reuse — `vendor/fr-client-rust/crates/client/src/core/world.rs`
(`free_dynamic_sprites`, `release_dynamic_sprite`, static indices not
recycled; 1000-frame bound test).

Scalar animation delay — `seq_type.rs` reads `AnimFrame::delay` without
cloning transforms.

Boxed appearances — `Client.player_appearance_buffer: Vec<Option<Box<Packet>>>`;
receive still boxes once, take/apply/restore (`client.rs` ~6909–6914).

Private animation bases — `anim_frame.rs` stores `Arc<AnimBase>` per unpack;
`get` still materializes an owned public clone (no Arc leak to callers).

Stop snapshot release — `crates/script/src/slot.rs:180-198` clears
`last_snapshot` / `last_world_id` / IPC on Stop; Pause keeps the instance;
restart emits a keyframe (`stop_releases_snapshot_storage_and_restart_emits_keyframe`).

Shared seed nav — `crates/host-play/src/memory.rs` `SeedNav::FromPlay`;
panel `session.rs:1307` and TUI `bin.rs:1166-1173` prepare vault, construct
Play, `bind_seed_nav(FromPlay(play.world()))`, then spawn. Tests cover Arc
identity and `FromPlay(None)` with no second decode (`memory.rs:1343-1410`).

Lazy panel CPU uploads with retained pixels — `crates/panel/src/game_view.rs`
starts as a 1×1 placeholder; applet CPU texture/RGBA exist only after a CPU
present; GPU-first unbind restores the placeholder.

Terminal snapshot cleanup after evidence — `crates/scenario/src/runner.rs:329-335`
clears the runner snapshot only after `tick_inner` (callbacks / same-tick
evidence). Tests keep terminal shots and follow-failure evidence, then assert
the snapshot is empty (`runner.rs:900-989`).

Routing publication/retry — `crates/host-play/src/lib.rs` `NavBot`:
one `route_worker` token (`Arc` ptr equality), generation on each request,
latest `pending_route`, stale `publish_route` ignored (`9566-9568`), `NoPath`
keeps the old route and clears `requested_route` so retry works (`9573-9577`).
Exact walks still refuse when a route/worker is live; WalkNear may replace
pending. Tests: `route_publication_rejects_stale_results_and_preserves_route_on_failure`,
`failed_radius_search_can_retry_same_destination_with_old_route_retained`.
Approach tiles skip the occupied target (`approach_candidates_avoid_occupied_target_and_stay_in_radius`).

Bank / loadout / food — Rust withdrawal sequencer (`bank_withdraw.rs`);
JS `Bank.withdrawX` waits on published inventory (4000 ms), not the queued
answer (`shim/bank.js:151-159`); remaining helpers still `notImpl`
(`foodForms`, `isFoodItem`, `foodCount`, `eatAtHpThreshold`,
`nextWithdrawChunk` / missing Withdraw X). Ordinary harness starts with an
empty loadout list; sustain posts the fixture loadout before ticks
(`memory.rs:593-623`). Adjacent bank access does not re-arm walk
(`gold_stubs.rs:365-383`).

Door cardinal step / edge bake — `api::prot::WalkStep` is existing
`MOVE_GAMECLICK` (no new opcode). `pending_door_step` requires same-level
cardinal adjacency (`interact.rs:1056-1076`). Bake `door_far_side` is one
adjacent standable tile; blocked scenery is not skipped; webs keep
multi-tile walk-out (`nav/src/transport.rs`).

Bounded stun inference — spotanim 245 onset + eleven distinct ticks, once
per follow run (`traveller.rs:648-757`). Disconnect clears the wait so
refusal stays reachable. This remains a revision/content inference, not a
universal stunned flag, and is not live-proven here.

Focused reconnect — `LoginQueue.preferred` survives the first grant;
`set_preferred` does not enqueue an online slot (`login_queue.rs:80-294`
and tests). `Play::focus` calls `set_preferred`.

Overlay / minimap / modal — overlay change can upload without a UI redraw
flag; overlay-only freeze keeps the punched minimap; sealed scene windows
re-upload opaque chrome (`gpu.rs` `finish` / `chrome`).

Windows sockets / home / metrics — Unix `poll` body kept under `cfg(unix)`;
Windows `WSAPoll` + loopback wake pair with peer check; stack arrays of
size 2. `operator_home`: explicit HOME (including empty) wins; USERPROFILE
only when HOME is absent. Script uses a local adapter (no production client
dep). Current vs peak RSS are distinct; Windows TCP count is `None`;
`memory-profile-no-alloc` nulls allocator fields rather than inventing zeros.

Reusable harness — `Run`/`Config`/`Sample`, LIVE+local gate, `ingame &&
scene_state==2`, seed/proof failure exit, N16/render policies, system-allocator
mode, test-only TUI `suppress_slot_spawn`. Basic GPU/timing counters come from
selected client profiling (`d5f26e3` / `b555f84` lineage), enabled before
clients are built in `prepare_unseeded`. Diagnostic sidecar sets
`recent_navigation` to JSON `null` because those capture producers were
excluded (`memory_diagnostics.rs:96-97`).

## Contract check (exclude)

No snapshot-dedup registry, borrowed fingerprint comparator, tiled nav
representation, or advanced owner/census/capture controller landed in this
diff. Dense collision remains. `memory_diagnostics` is the approved bounded
optional sidecar (frames/logs/requests), not the excluded capture stack.

## Focused review notes (no change requested)

Host callback lock / teardown
- Script wall lock order is documented: wall before slot
  (`host-play/src/lib.rs:571-575`). Isolate `pump_logs` keeps `logs` then
  `in_flight` to avoid the tick-interrupt deadlock (`load.rs:1241-1244`).
- Seed observe clones the per-name seed Arc, drops `SEEDS`, then locks the
  seed (`memory.rs:310-323`).
- `LoadIsolate::join` is bounded; Stop then clears host snapshot storage.
  Abandoned isolates remain visible to weak counter snapshots until they
  actually drop (`memory_profile.rs:127-128`).
- Scenario snapshot release is after evidence, not before.

Generation / token / pending worker
- Worker loop takes pending under the nav mutex, calculates unlocked,
  publishes only if the token still matches and generation is current, then
  drains or clears `route_worker`. Spawn failure clears only that token’s
  pending. I do not see a stranded-pending race under the mutex.

Bank publication
- Withdraw X does not treat AnswerCount as success. Sequencer fallback is
  blocked until inventory moves or fills.

Cross-platform cfg / features / seed order
- `cfg(unix)` / `cfg(windows)` split for park, wait, stream handles, and
  home. Feature `memory-profile` pulls `dep:scenario` and script counters;
  `memory-profile-no-alloc` keeps GPU/timing and nulls allocator counts.
  Panel/TUI bind nav after Play exists and before slot spawn.

Harness failures / stale tests / campaign leftovers
- Seed/proof failure returns `Err` and can write diagnostics
  (`memory.rs:658-664`). TUI unit fixtures can suppress real network
  workers. No leftover tiled/fingerprint/dedup types in the selected tree.
  Remaining foreign helper stubs still throw `not impl`.

## Validation limits (honest)

This pass reviewed source and tests in the frozen diffs. It did not rerun
cargo, bake, or live sessions (root is running those; competing builds were
avoided).

Do not treat the following as certified by this review:
- Native Windows/MSVC and Linux socket/home/metrics/harness/TUI/panel
  evidence (preflight describes environments; it does not validate this
  build).
- Remaining live checks (bank restock-to-22 + return + progress, reverse
  door login, stun trigger/recovery, Stop/restart, GPU↔CPU view switching,
  N1/N32 comparison).
- Combined memory-feature suite (reported in progress at review time).
- RSS/CPU savings. Allocation avoidance and historical pairs are not
  combined savings for this harvest. Do not infer RSS from allocation
  counts.

Operator-reported checks I am not re-claiming: client 75 lib + 100 targeted
integration (pre-format); host 124 library; API 166 / nav 254 / script 287 /
host-play 113; harness 126 focused; ordinary combined `cargo check`; fresh
selected pack 483 mapsquares; normal contested-door live PASS in 57s. Those
are inputs for later acceptance, not this code verdict.

Unresolved platform issues called out in the assessment (Windows/Linux GPU
shade-boundary, Windows CRC timing) have no shipping fix in this client
ancestry and stay visible.

Stun recovery has unit coverage only. Old v8 packs do not contain the door
edge correction; bake must be from selected code (already done for the
reported door PASS; that does not reuse an old pack as proof of the baker).

## Bottom line

APPROVE the combined selected code for continued validation and later
release acceptance. No code change is required from this review. Root
should attach the remaining measured/functional evidence before calling the
release accepted.
