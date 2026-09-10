# Corrected whole-client integration review (plan step 2 follow-up)

Reviewer: Hermes profile `branchreviewer`, model `grok-4.6`, provider `xai-oauth`.
Session: `20260910_103533_55a2fa`. Kanban `t_4bed943f` run 1085.
Start: 2026-09-10T14:36:25Z. End: 2026-09-10T14:40:28Z.
Kind: required fresh whole-client follow-up after the two confirmed
corrections. Not host profile/live acceptance, not 377, not campaign release.

Read once: current host `AGENTS.md`, `docs/execution.md`, plan
`docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md` architecture
and step 2, `docs/compat/reviews/client-trial-reconciliation.md`, and
`docs/compat/evidence/client-milestone/corrected-review-inputs.json`
(SHA-256 `e36c57a26812d1019462ef4e8d9360d11369c0648b9778d4fe65933ae4779b33`,
matches the contract). Both independent trial reports were submitted and were
read as permitted input. The earlier Grok 4.6 approval of `8b1a8091` is not
this verdict and does not override the reproduced P1. Root accepts the
concrete failure rather than a reviewer vote. Root's additional P2 is
explained and reproduced.

Work was read-only except this report path. No merge, push, branch change,
product edit, commit, or further review dispatch. Suites were not rerun;
counts were recomputed from the named correction receipts. Single-query mode
blocked `execute_code` and Python `-c`; inspection used `git`, `shasum`,
`jq`, `awk`, `read_file`, and `search_files`. Record those as timing
confounders. Wall time is not a model ranking.

## Verdict

**APPROVE** the corrected client candidate for plan step 2 source/regression
preservation against the published base.

The complete product remains acceptable with the two confirmed defects
corrected: stale inbound frame limits are cleared at the login-seed
transition for cold/reconnect/adoption on both revisions, and published 274
report stubs stay idle while 289 retains ordered close/report frames.
Native Linux/Windows execution, host 289 session binding, direct host
writers, live/public acceptance, mixed fleets, tutorial/audio parity, and
377 qualification remain **explicitly outstanding** and are not approved by
implication.

## Inspected refs

| Role | Exact commit |
|---|---|
| Host campaign HEAD / frozen host candidate | `80552d3c215ff0d7722b82bc4d54a03c6482ac6f` |
| Prior host fixture-correction HEAD | `391726831b3dc97d3614bb8ad8ddb48f75c1e85d` |
| Client published base | `9b41e6e06b9fd42dc2247fc813a303c3fcb92941` |
| Previous (defective) candidate | `8b1a80918f87089e4eb339f1d0e216c1b163348d` |
| Corrected candidate (submodule working tree) | `58120f28ee5208553ca07f41cb364f2cf98ea280` |
| Corrected client product | `d14755da758c64d971c1103b2d7703f6fd8379fb` |
| Source 289 | `c18f3a1148e9caee73426e162677328ca64d1a83` |
| Shared ancestor | `4f2048ea10f75b3bb92ff45610b35ba7313b0308` (confirmed `merge-base`) |

Host `HEAD` gitlink still names published client `9b41e6e0`. Root owns Git
hygiene. This review inspected the corrected candidate checked out at
`vendor/fr-client-rust` (`58120f28`), product `d14755da`, not a moving
uncommitted product HEAD. Candidate vs product is docs/evidence only
(`git diff --stat d14755d HEAD -- crates/client` empty). Host
`39172683..80552d3c` is review/docs only (7 files, no product).
`AGENTS.md` remains absent from the client merge (deliberate).

Product delta `8b1a8091..d14755da` is three files:
`crates/client/src/client/client.rs`, `crates/client/tests/lost_con.rs`,
`crates/client/tests/revision_289_outbound.rs`.

## Required findings

### P1 — stale inbound frame limit at login seed

`tcp_in` / `read_packet` still stamps `set_frame_end(psize)` after a complete
payload (`client.rs:4258`). Opcode reads still `clear_frame_end`
(`client.rs:4214`). `g8` still `assert_can_read(8)` (`packet.rs:212-213`,
assertion at `:102`). `adopt_from` still transfers the same inbound Packet
(`client.rs:2467`).

The defect: a completed short game frame left `frame_end = Some(1)` on the
reusable `r#in`. Login then wrote eight seed bytes through `data_mut` and
called bounded `g8` without clearing that limit. A new TCP connection does
not allocate a new Packet; adoption inherits the bound.

Correction: `clear_frame_end` only on the `response == 0` seed path, before
the eight-byte read (`client.rs:2557-2566`). Game-frame bounds are not
weakened.

New socket regression `short_game_frame_does_not_constrain_later_login_seed`
covers both revisions × cold (`logout` + `login(..., false)`), reconnect
(`lost_con`), and adopt (`adopt_from` then `login(..., true)`). It injects a
real one-byte `UPDATE_RUNENERGY` frame (`ServerProt` 83 / `ServerProt289`
mapped opcode, sizes `SERVER_PROT_SIZES[83] == 1` and
`SERVER_PROT_SIZES_289[UPDATE_RUNENERGY] == 1`) with ISAAC unset, waits for
`runenergy == 42`, then performs the handshake. Reconnect/adopt retain
`local_player.y == 77`.

Red receipt `login-bound-red.json` names base `8b1a8091`, exit 101. Log
panics at `crates/client/src/io/packet.rs:102:9` (`assert_can_read`). The
unnamed server thread then fails `lost_con.rs:35` (`n > 0`) because the
client abandoned the socket; that is a consequence of the client panic, not
a second product defect. `00-login-test-compile.log` is a distinct helper
compile error (`ClientConfig::clone` missing) and is not the runtime
failure.

### P2 — preserve incomplete 274 report controls

Published base `9b41e6e0` `client_button` documents 601-613 as slice 6 and
falls through to `false` after accept-design. No `CC_REPORT_MUTE` /
`CC_REPORT_REASON_*` arms. Incoming 289 handlers on `8b1a8091` ran unguarded
(`client.rs:6045` / `:6048`), completing previously idle 274 mute/reason
controls.

Correction gates both arms with `self.revision.is_289()` (`client.rs:6050-6054`).
289 mute still toggles with no packet. 289 reasons still `close_modal` then
`REPORT_ABUSE` / `SEND_SNAPSHOT` (`p8` namehash + `p1` reason + `p1` mute).
Mapped 289 opcodes are `CLOSE_MODAL` id 93 and `SEND_SNAPSHOT` id 94.
274 `CLOSE_MODAL` remains 51 / `REPORT_ABUSE` 137 and is not emitted by the
stub.

New test `report_controls_preserve_274_stubs_and_289_ordered_frames`:
274 mute stays false and `out.pos == 0`; each 601..=612 leaves `out.pos == 0`
and `main_modal_id == 7`. 289 mute becomes true with no packet; each reason
emits `[93, 94, 0, 0, 0, 0, 0, 0, 0, 1, code-601, 1]` and closes the modal.

Red receipt `report-controls-red.json` names base `8b1a8091`, exit 101. Log
fails at the new test line 182: 274 mute became `true` vs expected `false`.

## Prior constraints still in view

Correction did not retouch GPU/minimap/modal, appearance boxing, sharing,
sprite/accounting, Windows/home, or ownership files.

- `gpu.rs` blob `48657cfb…` is identical on `8b1a8091` and `d14755da`.
  `finish` still uses last-upload RGBA/coverage (`overlay_changed` /
  `punch_minimap = minimap_live || (overlay_changed && !chrome_upload_pending
  && minimap_held)`). No `overlay_epoch` / `overlay_upload_needed` /
  `note_overlay_signature` symbols. Explicit GPU receipt actually ran
  `npc_hint_production_gpu_blink_move_clear_cache_and_freeze` and
  `gpu_ground_input_composed` (`… ok`).
- Appearance remains boxed: 274 `Box::new(Packet::new(data))`; 289
  `Box::new(appearance.apply(…))` / `Box::new(raw)` in `actor_289.rs`.
- 274-only `anim_frame.rs`, `io/client_stream.rs`, `core/world.rs`,
  `render/world.rs` are unchanged vs published base `9b41e6e0`.
  `client_stream.rs` still has `raw_socket` / `WSAPoll`. `bot_target.rs`
  still prefers explicit HOME including empty, else USERPROFILE.
- Client `Cargo.toml` remains wgpu/protocol/render only: no host crate, no
  foreign script runtime. `ClientRevision` is `{R274, R289}` with
  `as_i32` only.
- No invented tick-end opcode in the correction. `SERVER_TIMEOUT` path in
  `lost_con.rs` is unchanged. No new `World::clone` on read in the product
  delta.

Host `mint_live_names` was already accepted with the prior fixture
correction; this host HEAD only adds review evidence. This review does not
re-open or expand that helper into profile/live acceptance.

## Independent receipt recount

Every evidence path in `corrected-review-inputs.json` passed
`shasum -a 256 -c`. `summary.json` SHA-256 values match the raw logs.

| Receipt | Recomputed |
|---|---|
| Seven affected suites `focused-green.log` (`lost_con`, `login`, `from_shared`, `revision_289_outbound`, `revision_289_stage1`, `revision_289_stage3`, `packet`) | 7 summaries, 127 passed, 0 failed, 0 ignored. Includes both new tests `… ok`. |
| Full workspace `workspace.log` | 76 summaries, 992 passed, 0 failed, 2 ignored |
| Workspace ignored names | `npc_hint_production_gpu_blink_move_clear_cache_and_freeze`; `gpu_ground_input_composed` |
| Explicit `--ignored` GPU `explicit-gpu.log` | 2 passed, 0 failed; both named tests actually ran |
| Clippy `clippy.log` | `--workspace --all-targets --no-deps -- -D warnings`, Finished clean |
| Fmt `format-check.log` | empty (SHA of empty file), exit 0 in JSON |
| Login-bound red | 0 passed / 1 failed, panic `packet.rs:102` |
| Report-controls red | 0 passed / 1 failed, 274 mute `true` vs `false` |
| `00-login-test-compile.log` | compile error, 0 test summaries |

`workspace.json` and `explicit-gpu.json` name product
`d14755da758c64d971c1103b2d7703f6fd8379fb`. Red JSONs name base `8b1a8091`.
Do not substitute the earlier 774/990/441 counts for these corrected inputs.

Historical GPU shade / CRC timing failures on other platforms were not
reproduced here and stay historical. Current Linux/Windows execution is
unavailable in this session.

## Findings

None blocking for this corrected milestone.

Non-blocking observations (do not reopen the correction):

1. Host gitlink vs submodule working tree still diverge; root hygiene.
2. `ClientRevision` has two variants; host must not treat every non-274
   client as 289 (step 3).
3. Extra GPU unit test still skips when `try_new` fails. 274 freeze/sealed
   tests still require a device.

## Scope limits (not approved)

- Immutable process profile, 289 host sessions, cache/nav identity, frontends
- Direct host writer adaptation / legal-send mapping
- Full 377, native rewrites, tutorial/audio parity, mixed fleets
- Public-live smoke, performance budgets, catalog option proofs
- Linux/Windows native client execution

## Receipt

| Field | Value |
|---|---|
| Task | `t_4bed943f` |
| Reviewer profile | `branchreviewer` |
| Model / provider | `grok-4.6` / `xai-oauth` |
| Session | `20260910_103533_55a2fa` |
| Product code edited | none |
| Other trial reports read | yes (`client-grok46.md`, `client-astra.md`, reconciliation) |
| Suites rerun | no |
| Tool restrictions | single-query blocked Python `execute_code` / `-c` |
| Deliverable | `docs/compat/reviews/client-grok46-corrected.md` |
| Verdict | APPROVE corrected source/client regression milestone; later host/live/native/377 gates outstanding |
