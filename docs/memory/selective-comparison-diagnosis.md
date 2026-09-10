# Selected N32 comparison: bounded diagnosis

September 10, 2026. Read-only source/evidence investigation; no rerun, driver
change, production edit, or timeout/predicate change.

**The best-supported explanation is a real Maze random-event teleport interrupting
the common driver's workload qualification. This is not evidence of an established
selected-build regression, but the failed run remains unqualified.** The records
do not establish transition duration, guardian recovery or resumed thieving.

## Evidence

Selected host `9527cc63`, client `daccb4b`; main host `54cfcf8`, client `4f2048e`.
The same common driver source hash was used in both processes.

- Selected reached 32 ready actors at 64.764 seconds, then failed at 152.300
  seconds with `actor readiness lost before observation`. It never emitted
  `observation_start`; there is no selected N32 active measurement window.
- All 32 scenario runners had Passed the original XP proof. All had null
  script/start errors. Their latest player-update observations were 0.126–0.172
  seconds old. This rules out a captured long fleet-wide packet stall.
- `live13d71_27` was the only Paused script. Its row recorded 250 player-generation
  observations, XP 101520, and an update age of 0.171558 seconds. Its retained
  proof had passed after 18.952 seconds at `(2661,3309,0)` with ten lobsters.
  That evidence is historical, not its current position/inventory at failure.
- The engine's later saved file for the same freshly minted actor has a valid
  v7 checksum and position **`(2891,4597,0)`**. Its mtime is
  `2026-09-10T03:10:56.022236Z`, about 30 seconds after client Stop. Save layout
  was verified against `Player.save` and `PlayerLoading.load`; the file hash,
  checksum and decoded coordinates are preserved in
  `selective-integration-evidence/compare-selected-n32/actor27-saved-state.json`.
  This is later saved state, not a timestamped server event log.
- That coordinate is precisely the NW Maze spawn: unchanged
  `crates/host/src/random/maze.rs:27` and the server's
  `content/scripts/macro events/configs/macro_events.enum:76`
  (`val=0,0_45_71_11_53`). The server's `start_macro_maze` procedure chooses this
  table and calls `p_telejump`. Two comparison actors inspected as controls
  saved normally near Ardougne: actor28 `(2658,3311,0)`, actor0 `(2658,3312,0)`.
- Main's N32 run did complete a 60-second observation with all 32 Running and
  individual XP/player-update progress. Its successful run does not reproduce
  the selected actor's random event or prove that main would avoid the same gate.

Source records: `compare-selected-n32/{metadata.json,markers.jsonl,run.log}` and
`compare-baseline-n32/markers.jsonl`, under `selective-integration-evidence`.
No readily available timestamped server event log was present beside these
artifacts or in the engine's usual log directories. The saved actor file was the
available server-side evidence; no broader log archaeology was performed.

## Predicate and source assessment

The common driver's `ready()` requires exact membership, ingame, scene_state 2,
and no login error. It checks this once immediately after all XP proofs pass and
aborts on failure. Its failure rows unfortunately omit the actual ingame,
scene_state, login error, tile and guardian fields read by the predicate.

The unchanged host publishes Client ingame/scene_state and passes `is_up()` to
`SlotScript::on_is_up`. With no operator Pause in this test, a Paused isolate
indicates its last presence gate was false. Guardian hold alone does not put the
slot into RunState::Paused. Scene loading after a teleport can do so, and is a
normal client lifecycle: map rebuild sets scene_state 1, then returns to 2.
Thus the later Maze save and contemporaneous Paused row fit a real scene transition.
The lack of an exact rejected status prevents a definitive assertion that this
was only a brief rebuild, or exclusion of a disconnect occurring around it.

Relevant differences do not provide a specific fault mechanism:

- The driver never calls focus/prefer_login. LoginQueue's preferred uid starts
  None, so the selected persistent focused-reconnect priority is inactive here.
- `SlotScript::on_is_up`, its lifecycle transitions, and host presence publication
  are unchanged from main. Selected loadout delivery and Stop cleanup do not
  themselves explain an unrequested pause during the run.
- Host random/maze implementation is unchanged. Selected terminal scenario
  snapshot cleanup releases runner-owned data after evidence; it does not change
  the live Client or the script gate. All terminal proofs remain intact.
- Selected food/routing behavior can change what the actors do and when server
  random events occur; this is not proof that either caused a bug. The driver
  does not record enough current gameplay state to attribute the event further.

Classification: **workload qualification interrupted by a strongly evidenced
random-event transition; recovery and exact failure instant unresolved.** Do not
label it a proven benign race, a thieving-stun recovery failure, a packet stall,
or a regression caused by scalar delay. A Maze teleport is a different event
from the spot-animation 245 thieving stun path.

## Proportionate options for the orchestrator/operator

1. Preserve this failed artifact and the missing N32 paired result in the final
   acceptance review. The selected run supports initial 32-actor XP correctness,
   not sustained N32 performance or Maze recovery. If the bounded release is
   accepted with that limitation, make it an explicit scope/acceptance decision.
2. If resolving the failure is required before acceptance, make one bounded
   diagnostic follow-up whose purpose is to observe the transition/recovery:
   capture the exact rejected SlotStatus plus the existing guardian status and
   the last presence transition for each actor. Keep all existing scene, XP,
   timeout and actor-count gates. Use an existing controlled Maze exercise if
   available to check pause/rebuild/guardian return; that is functional evidence,
   not a replacement matched performance window. Define the reproduction and
   stopping condition before launch; do not repeat random comparisons until green.
3. If a controlled later performance comparison is chosen, qualify and report
   identical external conditions on both full builds. Any deliberate suppression
   of random events would be a newly specified workload and must be disclosed;
   it cannot retroactively qualify this failed ordinary-session pair. Idle data
   would also retain its idle label and cannot close active scalar timing.

No selected fix should be rolled back solely on this evidence. Conversely,
existing unit/review success should not be used to waive the missing sustained
N32 measurement or claim the interrupted actor recovered after the process ended.

Server saved-state provenance: source
`/Users/acfrazier/experiments/Server/engine/data/players/main/live13d71_27.sav`,
SHA256 `825ee544d1441e114d56033091369dc34a76d7bc63f4d66f440a380ef81dd5bc`.
The report records derived coordinates/checksum only; the save itself was not
modified. `git diff 54cfcf8 9527cc63 -- crates/host/src/random.rs
crates/host/src/random/maze.rs` is empty; the script presence gate also has no
diff in that range. Final Grok acceptance can assess the selected release with
the explicit missing N32 paired performance result, without retrying this run or
claiming Maze recovery.
