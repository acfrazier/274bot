# Architecture review: 274/289 reconciliation and 377 seams

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10.
Kind: bounded pre-implementation design review of plan
`docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`
(architecture contract and steps 1–4). Not functional acceptance and not
final campaign approval.

Read once: current `AGENTS.md`, `docs/execution.md`, and the plan contract
above. Git objects were inspected read-only. No product code was edited,
merged, or built.

## Verdict

**Proceed to client reconciliation (plan step 2)** under the required
corrections below.

The one-process server-profile contract, client-owned packet maps, and
host-owned interaction policy are sound and implementable on the current
pins. The three overlapping client files are real merge hotspots, not
theoretical ones. Naive adoption of the 289 GPU overlay path would
regress the 274 last-FBO/minimap freeze. That is a design constraint for
the merge, not a reason to delay it.

Do not import the 377 host or client trees. Reuse their constructor and
fail-closed send *shapes* only. Do not claim measured memory savings or
377 gameplay support.

## Inspected commits

| Role | Path / ref | Exact commit | Tip subject |
|---|---|---|---|
| Host campaign | this checkout `codex/rs2b0t-multirevision` | `b2bd5023489ab2e0b6ba690f228217e8984ac91b` | Document proportionate verification for hobby-project work |
| Client pin / 274 line | `vendor/fr-client-rust` `codex/bothost-274-289` | `9b41e6e06b9fd42dc2247fc813a303c3fcb92941` | Resolve selected-code lint without changing render behavior |
| 289 source | fetched into the isolated client | `c18f3a1148e9caee73426e162677328ca64d1a83` | docs: record reviewed native 289 ground displacement proof |
| Shared client ancestor | merge-base of pin and 289 | `4f2048ea10f75b3bb92ff45610b35ba7313b0308` | fix(gpu): hold minimap composite across scene_state==1 freeze |
| 377 host (read-only) | `/Users/acfrazier/experiments/FR-vault/.worktrees/r377-rust-host` `codex/r377-host-foundation` | `a802ce1094f35d1257abc5f8b497f17675af3dde` | feat(host): revision construction and fail-closed 377 send guards |
| 377 client (read-only) | `/Users/acfrazier/experiments/FR-vault/.worktrees/r377-rust-client` `codex/r377-client-foundation` | `a172f29b5d0ac512abd33345f37d8d3f2fd24032` | [verified] Implement R377 native fullscreen rendering |

Confirmed: `4f2048ea` is the merge-base of both `9b41e6e` and `c18f3a1`.
289 is based on that ancestor and therefore predates the current 274 pin.
377 client `a172f29` is a 36-file / +7209-line delta from `c18f3a1`.
377 host `a802ce1` sits on the older memory-campaign host line
(`codex/memory-per-bot` ancestry, parent `4fa85bd`), not on `b2bd5023`.

## Architecture contract

The plan's ownership split is the right one and should be treated as
binding:

- One immutable process server profile, resolved before assets or
  sockets. It binds server, revision, target, endpoints, RSA/CRC inputs,
  cache identity, nav/content identity, and catalog source. Protocol
  revision and local/public target are distinct attributes validated
  together. All slots inherit it. Changing it requires process restart.
- Client owns framing, encoders, ISAAC/login, world state, rendering, and
  revision-selected packet definitions.
- Host owns interactions, navigation, guardians, compatibility services,
  and the refusal policy for unsupported or mismatched profiles.
- JavaScript stays script logic plus thin name/shape maps. No second
  packet table in the host. No bot action API inside the client.
- Share resources across slots on that one profile. Reject
  cache/nav/profile mismatch before Start. Never fall back to 274 assets
  for a 289 session.
- Keep command order, script tick edges, Pause/Stop, guardian holds,
  stale-target checks, existing timeouts, snapshot buffer ownership, and
  no extra deep world copies.
- Client revision support and host-qualified selectable profiles are
  distinct. 0.1.7 qualifies 274 and 289 only. The fork must not assume
  every non-274 client is 289, and must not hard-limit the client to two
  revisions.

Current host code does not yet implement that contract. `PlayOptions`
(`crates/host-play/src/lib.rs`) is `{host, port, cache_dir, lowmem,
mainland}` with no revision. `prepare_client` always calls
`Client::from_shared` (implicit 274). `load_template` unpacks
`{cache_dir}/config` and `{cache_dir}/interface` with no identity check.
Panel and TUI default `client::cache_dir()`. Direct writers in
`crates/api/src/{prot,interact}.rs` emit public 274 `ClientProt` ids.

The 289 client already supplies the construction seam the host should
use: `new_with_revision` / `from_shared_with_revision` plus read-only
`Client::revision()`. Default `from_shared` stays 274.

## Line map: 274 pin vs 289 vs ancestor

### Overlap (must reconcile deliberately)

Only three files changed on both `4f2048ea..9b41e6e` and
`4f2048ea..c18f3a1`:

1. `crates/client/src/bot_target.rs`
2. `crates/client/src/client/client.rs`
3. `crates/client/src/render/backend/gpu.rs`

**bot_target.rs.** 274 adds `operator_home()`: Unix `HOME` unchanged;
Windows uses explicit `HOME` (including empty) and `USERPROFILE` only
when `HOME` is absent or non-Unicode. `engine_dir` and `unpack_dir` both
switch to it. 289 leaves `engine_dir` on `env::var("HOME")` and adds
`$CLIENT_UNPACK_DIR` override plus `cache_dir_for_with_unpack` so prod
unpack is overridable while local still uses the engine pack. These
compose: 289 override first, then `operator_home()` for the default
unpack/engine paths. Do not keep 289's raw `HOME` read on Windows.

**client.rs.** 274 is a two-hunk memory fix:
`player_appearance_buffer: Vec<Option<Box<Packet>>>` with boxed
`Packet::new` on receive. 289 is the large revision bind (~1584/350):
immutable `ClientRevision`, `from_shared_with_revision`, login version
word `revision.as_i32()`, `map_client_prot` on outbound, 289 inbound
dispatch, packet-bounded reads, socket-adopt carrying revision. 289 does
not touch `player_appearance_buffer`. Keep both.

**gpu.rs.** This is the preservation hotspot. Ancestor already freezes
the last scene texture while `scene_state==1` and composites a held
minimap (`punch_minimap = minimap_live`; hold via `minimap_held`).

- 274 (pin) detects viewport overlay motion/removal by comparing
  `draw_area` RGBA/coverage to the last upload (`scene_overlay_changed`)
  *without* setting `chrome_upload_pending`. Punch becomes
  `minimap_live || (overlay_changed && !chrome_upload_pending && minimap_held)`.
  Overlay-only freeze frames keep the minimap hole. Also: sealed
  scene-window modal RGB upload when `scene_ready` is false; GPU
  allocation accounting (`profiling::Allocation`).
- 289 sets `chrome_upload_pending |= overlay_upload_needed(epoch)` and
  leaves `punch_minimap = minimap_live`. An overlay epoch bump during
  freeze therefore uploads chrome, does not punch, and clears
  `minimap_held`. 289 also forces `atlas_dirty` for `main_modal_id` /
  `main_overlay_id`, adds `GPU_SCENE_TEST_LOCK`, `note_overlay_signature`,
  and overlay tests that skip when `try_new` fails.

Required merge: keep 289's overlay epoch (hint blink/motion without HUD
flags) and test lock, keep 274's freeze punch/hold and sealed-modal
compare, and keep 274 profiling fields. Overlay invalidation must not
travel the full `chrome_upload_pending` path while a minimap is held.
274 GPU tests `expect` a device; 289 tests skip. Report platform GPU
absence separately; do not weaken the freeze assertions.

### 274-only (apply on top of 289; no file overlap)

Preserve these; 289 does not replace them:

| Area | Pin delta | Risk if dropped |
|---|---|---|
| Windows sockets | `io/client_stream.rs`: `raw_socket` / `WSAPoll` | Windows reader wait regresses |
| Animation sharing | `dash3d/anim_frame.rs`: private `Arc<AnimBase>` per unpack; public `get` still deep-materializes; `SeqType` uses `AnimFrame::delay` | extra copies / lock leakage |
| Sprite recycling | `core/world.rs` + `render/world.rs`: dynamic sprite free-list, bounded arena | sprite arena growth |
| GPU atlas / profile | `gpu_atlas.rs`, `profiling.rs`, `gpu_memory_profile.rs` | accounting / texture path |
| Appearance boxing | `client.rs` as above | appearance buffer size |

Public `AnimFrame::get` still clones the base. The saving is the private
store sharing one Arc across frames of one unpack. Do not "optimize"
further by returning Arcs into host/script.

### 289-only (take; keep 274 defaults)

Revision enum and size tables (`io/revision.rs`,
`server_prot_sizes_289.inc.rs`), outbound map (`client_prot_289.rs`
`map_client_prot`: 274 identity, 289 remap, unmapped panics), inbound
decoders (`actor_289`, `zone_289`, `misc_289`, `outbound_289`),
`cache_289.rs` offline fixture/manifest helpers (not a second `Cache`
type), `Packet::frame_end` so handlers cannot see leftover bytes in a
reused allocation, game-shell 289 telemetry/focus/mouse recorder,
tutorial/chat presentation (`tut_com_message` forces `redraw_chat` in
shared `CpuBackend::begin` so GPU chrome sees it), primary packet
fixtures and diagnostic opt-ins.

`ClientRevision` on 289 is `{R274, R289}` with `as_i32` only. That is
acceptable for this campaign if host profile parsing is an explicit
`274|289` match that refuses everything else. Do not write host policy as
`if !is_274() { /* 289 */ }`. 377 already shows the additive variant
shape (`R377` + own size table + named opcodes).

`ClientConfig` is still `{host, port, cache_dir, members, lowmem}`.
Revision is a session field, not a config field. Keep it that way: the
process profile owns the binding and passes it into
`from_shared_with_revision`.

### Host protocol seam (step 4, but it constrains step 2/3)

Public 274 ids currently hardcoded:

| Writer | 274 id | 289 mapped id |
|---|---|---|
| `Send::if_button` / `LEGAL_SEND` IF_BUTTON | 9 | 86 |
| `Send::close_modal` | 51 | 93 |
| `Send::count_dialog` | 102 | 180 |
| `cheat` `CLIENT_CHEAT` | 224 | 34 |
| `WalkStep` `MOVE_GAMECLICK` | 207 | 234 |

274 `MOVE_MINIMAPCLICK` is also id 86, which is 289 `IF_BUTTON`. Mixing
the public 274 table with a 289 socket is not a near-miss; ids collide.

`press` / `walk` / `op_loc` go through `doAction` / `tryMove` and will
pick up `client_opcode` once the 289 client is linked. `close_modal`,
`answer_count`, `cheat`, and door-step `WalkStep` will not. Opcode
selection alone cannot fix those host sends. Payload lengths still need
independent checks (count is `p4`, cheat is `p1(len)+pjstr`, walk is
control+abs x/z). Do not add a second host opcode table; route through
the client's map.

`RUN_ORB_IFACE = 153` is 274 control overlay content, not a packet id.
Treat content IDs as profile/nav identity (step 5), not as revision
enums.

## 377 seams (reuse shape, do not import trees)

### Host `a802ce1`

Reusable:

- `prepare_client` remains R274; additive
  `prepare_client_with_revision(..., ClientRevision)` uses
  `from_shared_with_revision` and keeps `cache_from_shared` / Arc equality.
- `Driver::revision()` (stubs default R274).
- Split policy: mapped client gameplay path vs direct 274 serializers.
- Tests that a refused send leaves `out.pos`, ISAAC, menu slot, and route
  unchanged; R274 walk/close/set_run bytes stay identical.

Do not copy:

- The product host at `a802ce1`. It is an older memory-campaign baseline
  plus a documented `client::profiling` compile hole. Current
  `b2bd5023` already has `CLIENT_TICK`. Importing 377 host would reintroduce
  superseded memory-campaign code.
- `client_gameplay_send_ok = !rev.is_377()`. That allows any non-377
  revision onto `doAction`. Unmapped revisions panic inside
  `map_client_prot`. Allow only revisions with a complete mapped gameplay
  path (274 identity, 289 map). Refuse everything else, including 377,
  before menu/ISAAC/route mutation.
- Selecting or constructing R377 in 0.1.7 frontends. Enum presence is not
  host support.

377 host correctly refuses direct 274 serializers on R289. For 289 that
is only an interim: `map_client_prot` already maps close/count/cheat/walk.
Step 4 should emit those through the client map, not keep a permanent
289 refuse. 377 outbound remains partial (MAP_BUILD_COMPLETE, NO_TIMEOUT,
CLOSE_MODAL, IF_BUTTON, RESUME_PAUSEBUTTON; else panic). Host must keep
refusing 377 gameplay.

### Client `a172f29`

Reusable later, not in this merge:

- Additive `ClientRevision::R377` and `SERVER_PROT_SIZES_377`.
- Fail-closed 377 inbound: never fall through into the 274 dispatcher.
- Partial 377 outbound map with panic on unmapped.

Do not take now:

- Fullscreen GPU early-returns (`fullscreen_active` skips scene/chrome and
  drops minimap hold). That fights both 274 freeze and 289 overlay epoch.
- 377 `client.rs` (+1133), `packet.rs` (+189), `profiling.rs` (81-line
  delta vs the 274 pin's 152-line profiler), `draw.rs`, `cpu.rs`.
- Any claim of login→scene→movement→dialogue parity. The 377 foundation
  does not establish that.

Later shared-file conflicts with the 274 pin, if 377 is adopted after
this campaign: `client.rs`, `lib.rs`, `profiling.rs`, `gpu.rs`.

## Priority findings

P0. GPU overlay invalidation vs last-FBO/minimap freeze. 289 epoch →
`chrome_upload_pending` + `punch_minimap = minimap_live` drops a held
minimap. 274 overlay-compare path exists specifically to avoid that.
Consolidate before calling the client candidate preserved.

P0. Host direct 274 writers. After the client merge, 289 `doAction` is
mapped; close/count/cheat/WalkStep still emit 274 ids unless step 4
routes them. Fail-closed refuse is acceptable until those adapters exist;
silent 274 ids on a 289 socket are not.

P1. `bot_target` Windows home + unpack override must compose. 289
`$CLIENT_UNPACK_DIR` is the right explicit override; 274 `operator_home`
is the right default resolver.

P1. Process profile is missing. Construction, cache unpack, panel/TUI
defaults, and CLI have no revision/resource identity. Bind and reject
before Start; no 274 fallback for 289.

P1. Extensibility. Keep `match revision` / named maps. Do not encode
two-revision forks in host policy. Do not enable 377 selection.

P2. 289 `Packet::frame_end` is required for reused inbound buffers. Keep
it when boxing appearances (274) so a boxed appearance packet is an
independent `Packet::new`, not a sliced view of `r#in`.

P2. 289 GPU tests skip without a device; 274 overlay tests require one.
Keep the 274 freeze/modal tests as GPU-required; keep 289 epoch unit
tests device-free; do not convert requires into skips.

P2. Content constants (run orbs, banks, stun heuristic) are not packet
maps. Mismatch is a step-5 world-capability defect, not a reason to put
content tables in the client.

P3. 289 `is_289()` presentation branches (tutorial continue, multiway
headicon, mouse latch) must stay 289-only so 274 pixels do not change.
Unknown revisions must not take the 289 presentation path.

## Required corrections / design constraints

1. Reconcile GPU `finish` so overlay-only updates during `scene_state==1`
   keep `minimap_held` and continue punching the minimap hole. Prefer
   289 epoch as the dirty signal and 274 punch/hold as the composite
   rule. Do not set full chrome pending for overlay-only freeze frames.
2. Keep 274 sealed-modal RGB upload (`scene_ready == false`) together
   with 289 `main_modal_id` / `main_overlay_id` atlas dirty. Lazy
   unchanged chrome must still skip upload.
3. Keep NPC hint blink/motion, tutorial `tut_com_message` chat redraw,
   and 289 multiway icon without forcing 274 HUD redraw every frame.
4. Compose `operator_home` + `CLIENT_UNPACK_DIR`. Local cache stays the
   engine pack; prod unpack is overridable; Windows home is 274's rule.
5. Keep Windows `ClientStream` reader-handle/`WSAPoll` behavior.
6. Keep anim-base Arc sharing, public get detach, dynamic sprite
   free-list, appearance boxing, and GPU profiling allocations.
7. Bind revision at `from_shared_with_revision`. Default constructors
   remain 274. Socket adopt copies revision with ISAAC/frame state.
8. Host `LEGAL_SEND` / `Send` / `WalkStep` stay the public 274 view.
   Multi-revision writes use the client map. No host-side 289/377 table.
9. Reuse 377 host constructor/guard *shape* on current `b2bd5023`, not
   the 377 host tree. Refuse unmapped revisions before mutation. Login
   handshake is not a gameplay send.
10. One process profile; all slots inherit; reject per-slot mismatch
    before mutation. Namespace 289 cache/unpack/nav/vault; keep 274
    paths and explicit overrides.
11. Snapshot families keep rebuilding from dirty gens; do not clone
    `World` on read.
12. Preserve tick order: observe before `client_frame`; mainloop drains
    the socket first; Pause/Stop/guardian hold/stale-target/timeouts
    unchanged. No invented tick-end opcode.
13. Public profiles require a known revision/asset pair. Fixture cheats
    stay local-only (`cheat_allowed` = `BotTarget::Local`).
14. Serialise edits to `gpu.rs`, `client.rs`, `bot_target.rs`, and host
    dispatch. Old 289 branch reviews do not cover the merge.

## Expected architectural benefit

Unmeasured, qualitative only:

- One shared unpacked cache and iface template per process profile,
  already the `from_shared` path, instead of per-slot unpack or a
  mixed-revision scheduler.
- One packet vocabulary in the client, so host interaction policy stays
  revision-agnostic except for refuse/allow.
- 274 memory/render work (appearance boxing, anim Arc, sprite reuse,
  lazy chrome, freeze) survives 289 protocol support.
- A third revision can be added later as another enum variant, size
  table, and map arm without rewriting host orchestration — after its
  own qualification.

This is not a savings claim.

## Bounded falsification / regression tests

Run these as the merge gate. Prefer existing tests; add only where
behavior would otherwise be unobserved. No heavy unrelated builds in
this review; implementers run the named suites.

Client (inside `vendor/fr-client-rust`; host `cargo test` does not run
these):

- Default `Client::new` / `from_shared` → `revision() == R274`, 274
  size table, 274 login version word.
- `from_shared_with_revision(R289)` shares the same `Arc<Cache>`
  (`ptr_eq`), sets `cache_from_shared`, does not unpack.
- Explicit 274 construction equals default construction for opcode ids
  used by IF_BUTTON / CLOSE_MODAL / MOVE_GAMECLICK / CLIENT_CHEAT.
- `map_client_prot(R289, IF_BUTTON).id == 86` and 274 id 9 is unchanged.
- Unmapped 289 opcode panics in the map (fail-closed), never emits 274.
- GPU freeze: scene_state 1 retains last scene; overlay move/clear
  uploads; minimap pixel at the held composite stays (274
  `viewport_overlay_moves_and_clears_without_chrome_redraw`).
- GPU sealed modal RGB uploads without HUD flags (274
  `sealed_scene_window_uploads_modal_rgb_without_chrome_redraw`).
- Overlay epoch: blink/coverage change uploads; uncovered pixels and
  unchanged epoch do not (289 overlay tests). Unchanged overlay stays
  lazy.
- Tutorial pending message sets `redraw_chat` in begin (289 cpu path)
  without CLS of frozen scene overlays (`should_cls_scene_overlays`
  false at scene_state 1).
- `operator_home` Windows/Unix tests plus `CLIENT_UNPACK_DIR` override
  tests, together.
- Dynamic sprite reuse bounded; anim private Arc shared within unpack
  and detached on public get.
- `Packet::frame_end` rejects reads past the frame; appearance boxing
  still stores an independent packet.
- Windows reader-handle test remains, cfg-gated.
- Existing 289 stage/outbound/actor fixtures still pass on the merged
  tree. Reproduce known GPU/CRC platform failures and report them;
  they do not excuse a new freeze or opcode failure.

Host (after the client is linked; not a step-2 merge blocker):

- `prepare_client` stays R274; `prepare_client_with_revision` selects
  274/289 without re-unpack.
- R289 `press`/`walk` emit mapped opcodes; R274 bytes for set_run/walk
  close stay identical to current fixtures.
- Direct close/count/cheat/WalkStep either map through the client or
  refuse with zero `out.pos` and unchanged ISAAC. Never 274 ids on 289.
- Unsupported revision and cache/nav mismatch refuse before connect.
- Snapshot rebuild does not `World::clone`.
- Observe still runs before paint; guardian hold still gates script
  tick.

Falsifiers: merged GPU overlay test loses the freeze minimap pixel;
default construction becomes 289; 289 session unpacks a second cache;
Windows home ignores `USERPROFILE` when `HOME` is missing; 289 close
writes id 51; snapshot path clones the world; 377 becomes selectable.

## What this review does not approve

- Merged client candidate (needs a fresh whole-client Grok 4.6 review
  after step 2).
- Host profile/frontend work (step 3) before the GPU freeze overlay
  contract is in the reconciled client.
- Catalog/live acceptance, native rewrites, mixed fleets, public live
  smoke, performance budgets, or 377 qualification.
- Import or submodule switch to either 377 worktree.
- Any measured RSS/CPU saving.

## Receipt

| Field | Value |
|---|---|
| Task | `t_192621de` architecture review |
| Reviewer profile | `grok46` |
| Model / provider | `grok-4.6` / `xai-oauth` |
| Host HEAD inspected | `b2bd5023489ab2e0b6ba690f228217e8984ac91b` |
| Client pin inspected | `9b41e6e06b9fd42dc2247fc813a303c3fcb92941` |
| 289 source inspected | `c18f3a1148e9caee73426e162677328ca64d1a83` |
| Shared ancestor | `4f2048ea10f75b3bb92ff45610b35ba7313b0308` |
| 377 host / client | `a802ce1094f35d1257abc5f8b497f17675af3dde` / `a172f29b5d0ac512abd33345f37d8d3f2fd24032` |
| Product code edited | none |
| Deliverable | `docs/compat/architecture-review.md` |
| Verdict | Proceed to reconciliation with P0 GPU freeze/overlay and host-map constraints |
