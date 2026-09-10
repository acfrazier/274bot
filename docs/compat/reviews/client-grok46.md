# Whole-client integration review (plan step 2)

Reviewer: Hermes profile `branchreviewer`, model `grok-4.6`, provider `xai-oauth`.
Session: `20260910_100829_e357e9`. Kanban `t_60bd2a51` run 1081.
Start: 2026-09-10T14:08:56Z. End: 2026-09-10T14:16:26Z.
Kind: independent whole-client review of the frozen reconciled candidate
plus the named host fixture correction. Not host profile/live acceptance,
not 377, not campaign release.

Read once: current host `AGENTS.md`, `docs/execution.md`, plan
`docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md` architecture
and step 2, `docs/compat/architecture-review.md`, and frozen manifest
`docs/compat/evidence/client-milestone/frozen-review-inputs.json`
(SHA-256 `a975118db49f01361b3a085ec4708ff6073ea14bcc3b134b60ac769084b58724`,
matches the contract). Did not use dated client campaign state as
instructions. Did not open `docs/compat/reviews/client-task-grok45.json`
or any other trial report as a findings source. Parent-card completion
metadata was visible in the dispatcher context and was treated as a claim,
not as this review.

Work was read-only except this report path. No merge, push, branch change,
or product edit. Suites were not rerun; counts were recomputed from frozen
raw logs. Single-query mode blocked `execute_code` and Python heredocs;
inspection used `git`, `shasum`, `read_file`, and `search_files`. Record
those as timing confounders. Wall time is not a model ranking.

## Verdict

**APPROVE** the frozen client candidate for plan step 2 source/regression
preservation, with the named host fixture correction accepted in the same
bounded sense.

No newly introduced blocking defects were found in the complete published-base
to candidate behavior. Native Linux/Windows execution, host 289 session
binding, direct host writers, live/public acceptance, mixed fleets,
tutorial/audio parity, and 377 qualification remain **explicitly outstanding**
and are not approved by implication.

## Inspected refs

| Role | Exact commit |
|---|---|
| Host campaign HEAD / frozen host candidate | `391726831b3dc97d3614bb8ad8ddb48f75c1e85d` |
| Host base | `b2bd5023489ab2e0b6ba690f228217e8984ac91b` |
| Client published base | `9b41e6e06b9fd42dc2247fc813a303c3fcb92941` |
| Frozen client candidate (submodule working tree) | `8b1a80918f87089e4eb339f1d0e216c1b163348d` |
| Client product | `cf386ae5c82071ef661f4a1eb4d38e22b41718d3` |
| Source 289 | `c18f3a1148e9caee73426e162677328ca64d1a83` |
| Shared ancestor | `4f2048ea10f75b3bb92ff45610b35ba7313b0308` (confirmed `merge-base`) |

Host `HEAD` gitlink still names published client `9b41e6e0`. Root owns Git
hygiene. This review inspected the frozen candidate checked out at
`vendor/fr-client-rust` (`8b1a8091`), product `cf386ae5`, not a moving
uncommitted product HEAD. Candidate vs product is docs/evidence only.
`AGENTS.md` is absent from the client merge (deliberate).

Host candidate vs base also includes `crates/script/examples/catalog_inventory.rs`
and campaign docs. Those files were not part of this product inspection.

## Ownership

Client crate dependencies remain wgpu/protocol/render only (`crates/client/Cargo.toml`).
No host crate or foreign script runtime dependency. Packet maps live in
`io/client_prot_289.rs` / `io/revision.rs`; 289 outbound is fail-closed
(`map_client_prot` panics on unmapped 274 ids). Inbound 289 dispatch does
not fall through to the 274 handler (`client.rs` `handle_packet`).

Host still constructs implicit 274 (`PlayOptions` / `prepare_client`). That
is step 3/4, not a step-2 miss. Direct 274 writers on a future 289 socket
remain a known host seam (`IF_BUTTON` 274 id 9 vs 289 id 86;
`MOVE_MINIMAPCLICK` 289 id 236). Do not treat client `map_client_prot` as
host-send proof.

`ClientRevision` is `{R274, R289}` with `as_i32` only. Acceptable here if
later host parsing is an explicit `274|289` match. Do not encode
`if !is_274()` as 289.

## GPU overlay / freeze (root RGBA contract)

Root selected last-upload RGBA/coverage comparison instead of a private
overlay epoch. Candidate matches that:

- No `overlay_epoch` / `overlay_upload_needed` / `note_overlay_signature`
  symbols remain.
- `finish` (`gpu.rs` 1480-1527): `overlay_changed` from `scene_overlay_changed`
  against last uploaded RGBA/coverage; `punch_minimap = minimap_live ||
  (overlay_changed && !chrome_upload_pending && minimap_held)`. Overlay-only
  freeze uploads keep the hole and `minimap_held`. Full chrome pending still
  seals when not live.
- Side/chat modals retain GPU atlas force; main modal/overlay are observed
  through the sealed-window compare (`gpu.rs` 1375-1379). When
  `scene_ready` is false, comparison uses opaque alpha 255 (`gpu.rs` 1681-1685),
  preserving sealed-modal RGB upload without HUD flags.
- Freeze `scene` still draws overlays without rebuilding the mesh
  (`gpu.rs` 1213-1221). `GPU_SCENE_TEST_LOCK` retained for test render.
- 274 `viewport_overlay_moves_and_clears_without_chrome_redraw` and
  `sealed_scene_window_uploads_modal_rgb_without_chrome_redraw` still
  `expect` a device. Added `gpu_finish_composes_changed_overlay_and_skips_unchanged_upload`
  returns on `try_new` failure (289-style skip). It is extra coverage, not a
  conversion of the 274 require-tests. On this macOS receipt it actually ran
  (`... ok` in workspace log).

Coupled production-hint + frozen minimap: ignored test
`npc_hint_production_gpu_blink_move_clear_cache_and_freeze`
(`gpu_overlay_tests.rs`) seeds a distinctive live minimap (`0x00aa11`),
poisons chrome at the minimap pixel, freezes `scene_state==1`, draws a real
NPC hint, and asserts held minimap + no scene rebuild + lazy follow-up.

- Red `03-combined-freeze-red.log`: fail at line 222, left `13378048`
  (`0xcc2200` poison) vs right `43537` (`0x00aa11`). SHA matches manifest.
- Green `04-combined-freeze-green.log` and root explicit GPU both pass the
  same test, with the proof eprintln on the green filter run.

289 `tut_com_message` forces `redraw_chat` in shared `CpuBackend::begin`
(289-gated). That is chrome dirty, so it correctly takes the full-chrome
path rather than overlay-only punch. 274 presentation is unchanged
(`legacy_274_keeps_base_interface_order_and_no_pending_frame_dirtiness`).

## Other required seams

Appearance boxing: 274 `client.rs` still `Box::new(Packet::new(data))`.
289 `Appearance::apply` returns `Packet::new(self.raw)` (owned copy from
`bytes()`), stored `Box::new` at mask and New-movement writers
(`actor_289.rs` 202, 463-464, 582). Independent of reused `r#in`.

Constructors: `new` / `from_shared` default `R274`;
`new_with_revision` / `from_shared_with_revision` bind immutable revision;
`cache_from_shared` + Arc identity for cache/ifaces/ifaces_mut
(`from_shared.rs` `shared_constructors_preserve_revision_and_arc_identity`).

Adopt copies revision with stream/ISAAC/frame (`adopt_from` 2452-2474).
Cold login (response 2) resets appearance buffer and entities; reconnect
grant (response 15) keeps sim state (`client.rs` 2690-2706). Login version
word is `revision.as_i32()`.

Framed reads: `set_frame_end(psize)` after a complete payload; 289 handlers
use `inbound_end`. `SERVER_TIMEOUT` remains 15s. No invented tick-end
opcode. No new `World::clone` on read in the product delta.

`bot_target`: `CLIENT_UNPACK_DIR` wins when nonempty; default unpack uses
`operator_home()` (Windows explicit HOME including empty, else USERPROFILE).
Local cache stays engine pack. Tests cover override, empty override, and
home selection.

274-only files unchanged vs published base: `anim_frame.rs` (`Arc<AnimBase>`
private share, public `delay`), `client_stream.rs` (`raw_socket` / `WSAPoll`),
`core/world.rs` / `render/world.rs` dynamic sprites, GPU atlas/profiling.
`Allocation::new` still accounts vertex growth.

289 presentation (`in_multizone` headicon, tutorial pending text, mouse
recorder / ground-trace opt-in) is `is_289()` gated. Diagnostic ground
trace is env-opt-in, not a production overlay policy.

## Host fixture `mint_live_names`

Scoped to `crates/host-play/src/lib.rs` (helper + extended existing test).
Old PID+serial `chars().take(max_token)` dropped the changing serial
(`live-name-red.log`: collision on `live11089_0`, exit 101). Candidate uses
a process `OnceLock` counter seeded by `OsRng`, encoding low base-36 digits
right-to-left so increment always mutates the token inside the 12-char
budget (`live` + token + `_` + slot). Prefix, slot suffix, and limit
unchanged. Callers (`panel` session, `tui`, `e2e`, `host-play` memory)
keep the same signature; no frontend/harness contract change. Isolation is
per-process invocation, not PID uniqueness across processes.

Extended test mints 24 rounds of fleet sizes 1/2/32/128 into one set.
That is fixture maintenance for later live proofs, not live acceptance.

## Independent receipt recount

Spot-checked SHA-256 values against the frozen manifest (manifest file,
workspace/GPU/host/baseline logs, ctor/tutorial/clippy/fmt, host lib,
milestone doc). Matches.

| Receipt | Recomputed |
|---|---|
| Published-base workspace `client-baseline-274/cargo-test.log` | 774 passed, 0 failed, 0 ignored (67 result lines) |
| Candidate workspace `14-cargo-test-workspace.log` | 990 passed, 0 failed, 2 ignored (76 result lines) |
| Workspace ignored names | `npc_hint_production_gpu_blink_move_clear_cache_and_freeze`; `gpu_ground_input_composed` |
| Explicit `--ignored` GPU (`15-…` and `root-explicit-gpu.log`) | 2 passed, 0 failed: overlay freeze proof **and** `gpu_ground_input_composed` actually ran (`... ok`) |
| Combined freeze red/green | 0/1 fail then 1/0 pass on the overlay test |
| Shared constructor filter | 1 passed |
| Tutorial CPU | 5 passed (includes 274 control) |
| Clippy `13-cargo-clippy-pass.log` | Finished clean, 205 bytes |
| Fmt `07-cargo-fmt-check.log` | empty (SHA of empty file) |
| Host after name fix | 441 passed, 0 failed, 7 ignored |
| Host ignored (later live gates) | `live_channel_ladder_client_wall`, `live_channel_walk_focuses_every_slot`, `login_one`, `login_two`, `live_draw_off_never_paints`, `live_rss_ladder_all_null`, `wall_login_fifo_logout_all` |

Historical GPU shade / CRC timing failures on other platforms were not
reproduced here and stay historical. Current Linux/Windows execution is
unavailable in this session.

Old standalone 289 branch reviews are not this candidate's approval.

## Findings

None blocking for this milestone.

Non-blocking observations (do not reopen step 2):

1. Extra GPU unit test skips when `try_new` fails. 274 freeze/sealed tests
   still require a device. Do not weaken those expects.
2. Host gitlink vs submodule working tree still diverge; root hygiene.
3. `ClientRevision` has two variants; host must not treat every non-274
   client as 289 (step 3).

## Scope limits (not approved)

- Immutable process profile, 289 host sessions, cache/nav identity, frontends
- Direct host writer adaptation / legal-send mapping
- Full 377, native rewrites, tutorial/audio parity, mixed fleets
- Public-live smoke, performance budgets, catalog option proofs
- Linux/Windows native client execution

## Receipt

| Field | Value |
|---|---|
| Task | `t_60bd2a51` |
| Reviewer profile | `branchreviewer` |
| Model / provider | `grok-4.6` / `xai-oauth` |
| Session | `20260910_100829_e357e9` |
| Product code edited | none |
| Other trial report read | no (none present besides frozen grok45 JSON, unread) |
| Suites rerun | no |
| Tool restrictions | single-query blocked Python `execute_code`/heredoc |
| Deliverable | `docs/compat/reviews/client-grok46.md` |
| Verdict | APPROVE source/client regression milestone; later host/live/native/377 gates outstanding |
