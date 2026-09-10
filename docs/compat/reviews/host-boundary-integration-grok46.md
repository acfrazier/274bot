# Step-4 host-boundary integration review

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kanban card `t_0ec632f9`.
Kind: independent Grok 4.6 coherent integration review of the combined
host action / publication / lifecycle boundary. Not a same-card Grok 4.5
task review, not live acceptance, not step-5 world/guardian qualification,
not the final whole-campaign `branchreviewer` pass.

Read once: `AGENTS.md`, `docs/execution.md`, plan architecture plus step 4
in `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
brief `15-host-boundary-integration-review.md`, design
`02-host-boundary-design.md`, `02a-host-outbound.md`,
`02b-host-snapshot-reset.md`, `13-host-snapshot-reset.md` top, and the
script-loading policy in `STATE.md` plus `b9cacc6e`. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report. No product edits, commits, stash/reset/checkout, index changes,
client gitlink movement, or live/native launches.

Campaign HEAD at inspection is `5046d610` (docs-only freeze of brief 15 /
STATE after the candidate). Concurrent uncommitted WIP was left untouched
and is **not** part of this candidate: `Cargo.lock`,
`crates/host-play/src/profile.rs`, `crates/nav/**`. Frozen `profile.rs`
was read from `fede3c0d`, not the dirty worktree.

## Verdict

**APPROVE the integrated source candidate** for bounded live
qualification at host product head
`fede3c0dd5d5fbdc7fd7def10f2eb43630b8f16f` with reviewed client pin
`6cb5a0b17aeef74da6b57b205e916681daee4f76`.

This is **source approval** of the combined public-289/local mapping,
named outbound writes, snapshot publication, reconnect watermark,
invalidation accounting, producer/consumer queue order, isolate late-reply
discard, script keyframe lifecycle, revision-agnostic script load/start,
and shared live-harness path. It is **not** live acceptance. Root still
owns the controlled local 274/289 loop: login/scene2, script/host-caused
walk, NPC and loc interaction, and actual IF logout. Production 289 slots
remain gated until world qualification. This milestone does not replace
the final Grok 4.6 whole-branch review.

The superseded binary `b9cacc6e` is not an acceptance candidate.

## Inspected refs

| Role | Exact value |
|---|---|
| Branch | `codex/rs2b0t-multirevision` (not `main`) |
| Host product base (step-3 presentation) | `41d896b3007e476c76a9c01352b64d5afd5ed901` |
| Host product head (scene correction) | `fede3c0dd5d5fbdc7fd7def10f2eb43630b8f16f` |
| Docs freeze after candidate | `5046d61037b225ab860c6ac84589217783827178` |
| Client base (accepted step-3 pin) | `2be1697060e4d2b8b709ad4d5e54d12513b38333` |
| Client head / host gitlink at `fede3c0d` | `6cb5a0b17aeef74da6b57b205e916681daee4f76` |
| Brief SHA-256 | `b4244ff5413d8fae291407b95baf77083e64fc3bbce01d99531999f384782cc8` |
| Design SHA-256 | `bf9cb322dc3555877772a3c1ade4440dc727b3f9492150359b94114dee06cc61` |
| Outbound report SHA-256 | `7e0e508bc7d87fb25e8c61fafc953ba1f0a81efbaa9982acd9e544e9030e5c04` |
| Snapshot report SHA-256 | `02e20787b3b904eff95b4a02fe76f27ada73733e20bf311f03f3331fa8d029cc` |
| Reviewer model/provider | `grok-4.6` / `xai-oauth` |

Confirmed: `git branch --show-current` = `codex/rs2b0t-multirevision`.
`git ls-tree fede3c0d vendor/fr-client-rust` is `6cb5a0b1`. Host
`fede3c0d..5046d610` is docs only. Product diff `41d896b3..fede3c0d`
is the range reviewed. Client `2be16970..6cb5a0b1` is the one client
commit `Expose successful session and packet publication boundaries`.

## Prerequisite actual-model reviews

| Lane | Card / run | Receipt |
|---|---|---|
| Public 289 profile | `t_96dc78da` run 1099 | `evidence/public-profile-correction/review.json`; actual `grok-4.5` / `xai-oauth` session `20260910_133757_d52004`; approved `04d3e02f` |
| Outbound writers | `t_b22abf57` run 1102 | `evidence/host-boundary/outbound/review.json`; actual `grok-4.5` / `xai-oauth` session `20260910_135900_8cf8e7`; approved `e946c2f9` |
| Live harness source | `t_079d478d` run 1101 | `evidence/host-boundary/harness/review.json`; actual `grok-4.5` / `xai-oauth` session `20260910_135359_681e61`; approved `2f1d9709`; live not run |
| Snapshot/reset, including response-15 scene correction | `t_0be9adbf` run 1111 | parent handoff; actual Grok 4.5 corrective review approved `fede3c0d` / client `6cb5a0b1` |

Design contract `02-host-boundary-design.md` (card `t_d6761a15`) remains
the ownership baseline, including the orchestrator correction that R289
legal rows follow named packet identity (OPPLAYER2 id 51 and
EVENT_MOUSE_CLICK id 224 are legitimate).

## Combined boundary

### Public-289 / local revision mapping

Frozen `ServerProfile` accepts `local-274`, `local-289`, and `public-289`.
`public-274` refuses. Public 289 requires `w1.rs2b2t.com:443` on both
game and asset hosts. `require_bot_operation` still returns
`host-boundary-not-qualified` for R289 on spawn, vault start, TUI `run`,
Play cheat, and `queue_wire`. That is the temporary slot/operation gate
awaiting world qualification, not a script-loading allowlist.

### Named outbound writes

Production typed writers use `write_for_revision` /
`map_client_prot(driver.revision(), named)`:

- `close_modal` / `answer_count` → IF_BUTTON payload is not used here;
  CLOSE empty, COUNT `p4`
- `WireCommand::DoorStep` → `WalkStep::write_for_revision`
- `cheat` → CLIENT_CHEAT id 34 on R289, still local-target only
- `press` / `walk` / `op_loc` / named NPC/player/loc/ground/item/widget
  ops remain on `doAction` / `tryMove`

Independent R289 byte oracles in `revision_289_direct_host_writers_match_primary_bytes`
are literal `[86,…]`, `[93]`, `[180,…]`, `[34,…]`, not mapper echoes.
Forged typed `Send` refuses before `out.pos` or ISAAC move. Legal rows
for R289 are named-mapped; OPPLAYER2=51 and EVENT_MOUSE_CLICK=224 are
present as those names. `LEGAL_SEND` remains the R274 table.
`Send::write` / `WalkStep::write` still emit 274 ids as the documented
R274-compatible public path; production call sites do not use them.

### Snapshot publication and reconnect

`host::publish_snapshot` is the shared seam for `SlotLoop`, the
host-play observer, and `revision_boundary_live`. Logout (`!ingame`)
resets and returns without rebuilding leftover client arrays. A
successful grant (`session_changed`) resets to `client.session_start_gens()`
then rebuilds; PLAYER_INFO in the same drain can publish immediately.

Client `6cb5a0b1` adds generic observations only: `gens.session` on
successful response 2/15, `session_start_gens` watermark, dedicated
`player_info`, and `invalidations` on `bump_all_gens`. Failed login and
rejected reconnect do not mint a session. R289 PLAYER_INFO publication
is distinct from ALL-family invalidation (`player_info: false` on the
ALL flag). No new opcode. No bot policy in client.

`family_observed` subtracts explicit invalidations rather than inferring
from equal family deltas. PLAYER_INFO is a dedicated counter, so
REBUILD+another-family cannot invent a tick and equal deltas cannot hide
a real PLAYER_INFO. Scene/Loc/GroundItem opt out of that subtraction:
scene scalars always refresh; the collision grid rebuilds on scene gen
**or** once when the host view is missing and the client is
`ingame && scene_state == 2` (the response-15 correction); locs rebuild
on scene gen or `tile_model_stamp`; ground items stay scene-gen gated.

No deep world copy: families remain generation-gated view copies. The
scene correction copies collision flags once when materializing a missing
host grid. Tick remains `should_emit_tick(player_info)`.

### Producer / consumer order and isolate lifecycle

Observer order on a session boundary: close the published `ingame` gate,
clear cheat/wire/nav/script deferred work, then `publish_snapshot`.
`queue_wire` / `cheat` hold that gate through enqueue. The gate reopens
only when `ingame && scene_state == 2 && local_player`. After
`run_client` returns, the same close-then-clear pair runs again. A new
`Pump` / `GameSnapshot` is constructed for the next connected session;
script tick stays monotonic across the outer login loop.

Isolate `reset_session_work` bumps `work_generation`, drops unread
interacts and in-flight, and sends `ResetSession`. Queued ticks and late
Interact/Completed messages with the old generation are discarded.
Parked waits, the script instance, and operator `want_run` survive.
`last_snapshot = None` forces the next post to be a full keyframe. No
FlatBuffer schema or JS policy change.

### Script loading / starting

Operator clarification is honored. `b9cacc6e` removed catalog-hash
capture, bind-time catalog identity comparison, and
`require_bot_operation` / `validate_catalog` from script load and start.
Panel revision changes and catalog imports do not unload already-loaded
sources. Server/cache/nav identity checks remain process-bound.
Catalog proof is not a loading allowlist. Unsupported capabilities still
refuse at the host/client operation boundary.

### Shared live harness

`revision_boundary_live` constructs through `SharedClientTemplate` /
`prepare_client`, drives `Interactions` on the real `Client`, and
publishes through `host::publish_snapshot`. It does not start Play slots
or load nav. Predicates require scene2 + tile, walk displacement, NPC
dialogue modal text, loc typecode/action change, and logout emptying
actors/inv/bank. Failures print FAIL and `exit(1)`. Timeouts were not
weakened in source. The test remains `#[ignore]` until `LIVE=1`.

## Limits of this approval

1. No controlled live cell has run. This review does not accept 274 or
   289 gameplay.
2. Production 289 `try_spawn_slot`, vault start, TUI `run`, cheat, and
   `queue_wire` still refuse. That gate stays until step 5 world
   qualification.
3. After a response-15 grant, leftover ground items wait for a scene-gen
   move; locs may rehydrate via `tile_model_stamp` if the retained world
   is nonempty. Parent already recorded locs as outside the scene-grid
   correction. Live loc proof after reconnect should not assume a
   scene-gen bump.
4. The harness picks Lumbridge `Hans` / nearby `Door` and seeds
   `3220,3212`. That is fixture coupling, not a 274 content-id hardcoded
   into the driver. A 289 fixture missing those names fails honestly.
5. `Send::write` / `WalkStep::write` remain 274-id public helpers.
   Production must keep using the revision-selected methods.
6. In-tree cargo was not rerun here: concurrent dirty `Cargo.lock`,
   `host-play` profile, and `nav` would not be a frozen candidate. Parent
   Grok 4.5 already executed host lib 136, api snapshot 61, focused
   reconnect/quiet regressions, and clippy `-D warnings` on api/host.
   Outbound/harness/public-profile receipts remain the hashed Grok 4.5
   runs above.
7. Linux/Windows, 289 nav/guardians, catalog cards, public live smoke,
   and performance claims are out of scope.

## Product files in `41d896b3..fede3c0d`

`crates/api/src/{interact,prot,snapshot}.rs`, `crates/api/tests/{interact,snapshot}.rs`,
`crates/host/src/{lib,slot}.rs`, `crates/host-play/src/{lib,main,profile}.rs`,
`crates/host-play/tests/{revision_boundary_live,session_profile}.rs`,
`crates/panel/src/session.rs`, `crates/script/src/{load,slot}.rs`,
`crates/script/tests/load_isolate.rs`, `crates/tui/src/bin.rs`,
client `crates/client/src/client/{client,actor_289}.rs` and
`crates/client/tests/{gens,login,lost_con}.rs`.

## Conclusion

Proceed to the authorized controlled local live proof on isolated 274 and
289 engines using this frozen pair. Do not lift the production 289 slot
gate. Do not treat this document as live or campaign acceptance.
