# Direct live per-bot owner capture — bounded design

Status: DESIGN ONLY, task t_360cbcc9. No capture, optimization, default change,
navigation disposition, or performance acceptance is released by this document.
Phase A is complete; this is not another structural-layout exercise.

## 1. Decision and question

Implement one default-off, in-thread logical-payload census, then ask root to
release ONE N1 active TUI diagnostic on the existing Linux measurement host.
Measure the live World allocation tree, selected Client tables/COW overlays,
both actual snapshot shells, and the retained script fingerprint. Observe natural
encoded buffers by metadata only. Do not collect allocator stacks.

The historical c0709ab/3456 diagnostic has N1/N16 steady RSS 176.322/520.779 MiB,
22.964 MiB/additional active bot versus 16 MiB, and N16 CPU 0.561746 versus
0.5 core. Those observations motivate the question; they are not owner sizes.
A direct N1 census can establish whether a private owner is large enough to be a
plausible contributor in this workload, without inferring it from an intercept.
It cannot establish the distribution across sixteen clients or nonlinear growth.
No N/N+x pair is needed for this discriminator. A later savings claim still
requires matched N/N+x measurements under performance-finish-plan §§3–5.

Choose N1 rather than adding a new harness scale or repeating N16. Compare its
private COW storage directly with the existing shared template, not with a
second account. If this scene does not discriminate, report that result; no
automatic N2/N16 escalation or repeated live attempt follows.

The tiled navigation candidate is a separate shared fixed owner. Its measured
cold-load result failed the original gate, but the operator accepted the measured
~344 ms startup tradeoff at 0036143; root provisionally retains the candidate.
CPU, latency, Stage B and all other gates remain unchanged. This design neither
changes navigation nor releases a capture or optimization. Frozen-source staging
below does not revert navigation in the campaign checkout.

## 2. Source authority and evidence, not runtime assumptions

All H anchors below mean host `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`;
C anchors mean client `3456edc8dabf7b25ada78110ffa56327af9f67a4` under
`vendor/fr-client-rust`. Paths beginning `client/` below abbreviate
`vendor/fr-client-rust/crates/client/src/`. Line numbers refer to those objects.

Source audit for this design used named `git show` objects (including H host-play
and snapshot_dedup, and C core/world), with numbered reads of corresponding
files after diffing them against H/C. H host-play lib/memory changes are only
later test hunks; the production spans cited here retain their frozen line
numbers. H host lib, script slot/isolate_fb, api snapshot/snapshot_dedup/lib/
obj_names, relevant manifests, and C client.rs/core/world/dash3d/if_type/packet
have no inspected-source delta. This is source-reading lineage, NOT permission
to compile moving current path dependencies. The old Phase A preflight is not
a new full-tree admission.

Reuse these evidence snapshots without re-parsing whole Heaptrack files:

- `current-per-bot-owner-ledger.md` and `.json`: completed Phase A, native type
  layouts and explicit missing occupancy. Use `size_of` in the admitted capture
  target, not assumed Mac ABI constants.
- `current-tui-n1-n16-attribution-report.md` and evidence/review companions:
  qualified-but-not-final observation, missing raw resource provenance and
  unmeasured overhead.
- `current-owner-allocation-capture-result.md`: sole global trace hit its 2 GiB
  output guard; no full observe/Stop epochs. Qualified saved-prefix callpath
  groups remain leads, not retained field ownership. Do not relabel global
  startup navigation/input overlap as per-client storage.

### Reachability and proposed hooks

Every API named **new** below is proposed, not already implemented.

| Owner | Verified source boundary | Minimal diagnostic addition and lifetime |
|---|---|---|
| Client and World | H `crates/host/src/lib.rs:362–398`: `Host::client_tick` owns `&mut Client` on the slot thread; observe runs before client_frame. C `client/client.rs:339–396` exposes Client tables; `core/world.rs:41–104` hides World vectors (`pub(crate)` or private). | Immediately BEFORE the existing observe call at H:378, inspect `&Client` and `&slot.snapshot`. Add **new** C feature-gated `World::owner_payload(&self, budget)` in core/world.rs, returning only bounded counters. No host attempt to access private World fields, no transmute, no model materialization. |
| Host snapshot | H host `SlotLoop` private `snapshot` at 897–900, rebuilt by `after_drain` at 989 onward. | Same pre-observe hook has a valid shared borrow. Add **new** `GameSnapshot::owner_payload(&self, budget)` inside api snapshot.rs, where private Vec capacities/gens are accessible. Slice getters alone cannot recover capacity. Borrow ends before observe. |
| nav_snapshot | H host-play `lib.rs:3865–3877`: owned by the move observe closure; rebuild at 3945, consumers at 4006–4040. | At closure ENTRY, before memory::client_frame/slot_frame, inspect the actual retained `nav_snapshot`, not a rebuilt surrogate. Join its scalar result with this frame's pre-observe host row. Both shells coexist; neither has been mutated between those reads. Keep each shell's actual tick/gens separate. This measures retained pre-observe state, not rebuild peak. |
| COW template | H host-play `lib.rs:3061,3160–3170,3578,3710,3765`: Play owns `ifaces_mut_template`, passed into slot construction. C client.rs:304–315 gives outer and inner Arcs. | In the existing slot closure, borrow its already-held template and Client overlay after construction. Compare corresponding Arc identities with `Arc::ptr_eq`; no additional strong template copy for the observer and no COW write. Count template storage separately once. |
| Script/fingerprint | H host-play `lib.rs:611–618,695–703`: ScriptWall resolves a per-slot mutex; script_observe already locks that slot. H script `slot.rs:27–51` has private `last_snapshot` and `ipc`. | Add **new** `SlotScript::owner_payload(&self, budget)`; call inside the EXISTING slot guard immediately after `on_is_up`, before encoding/tick. No new lock acquisition and no lock-order change. Fingerprint and host builder live on the caller side, not inside V8. Emit scalar fragment after releasing the guard. |
| Encoded Vec | H host-play `lib.rs:724–738`: return Vec exists immediately before post_snapshot moves it. H script `isolate_fb.rs:2264–2266` copies finished_data into it. | Record len/capacity scalars at this ownership point, without cloning, retaining, decoding, or delaying its move. Record once for natural initial keyframe and once for first subsequent delta; separately record the first natural post in each active census window. Never force a post/keyframe. |
| Builder | H script `isolate_fb.rs:2242–2266`: private `FlatBufferBuilder<'static>`; only capacity seam is cfg(test) consuming `into_backing_capacity(self)` via collapse. Dependency pinned to flatbuffers 25.12.19 in H script Cargo.toml. | Live backing capacity and builder scratch allocations remain **unknown** in this minimal design. `finished_data().len()` is only encoded length. Do not consume/collapse/reset/reconstruct/replace a live builder to measure it, use private-layout casts, or assert a non-consuming capacity API exists. No dependency patch in this follow-up. |

The pre-observe hook and closure communicate through a **new diagnostic-only
thread-local scalar staging slot** in host, not a thread-local Client/snapshot
reference. API accounting has no host dependency; script returns its own scalar
accounting type. Host-play combines them. Register one opaque slot-instance token
and one preallocated output mailbox before login. At pre-observe entry clear any
previous partial staging row and increment a diagnostic frame serial; consume
only matching token/request/frame. No username-keyed global owner registry.
Do not store raw references, upgrade Weak owners, or keep product owners alive.

For the COW comparison, capture a borrowed `&ifaces_mut_template` into the inner
observe closure, not the Arc by value: it already lives in the enclosing slot
thread around prepare_client and the login loop (H host-play:3760–3789).
H host `run_client` bounds at 243–246 impose FnMut, not `'static`, so this borrow
can last exactly that call and end before the next login round. No template
reference goes into staging/mailboxes. Build the scratch identity index from
the borrowed template and overlay in bounded passes, not an unbounded quadratic
scan across every pair of components; identity-table probe work consumes the
same visit/deadline budget.

These are sequential reads in an unchanged program order, not an atomic census
of every thread. Client+host+nav pre-observe rows can be combined as a coexisting
retained population. Fingerprint read later under the normal script guard has its
own time interval and may have changed due to operator Stop. Never sum a previous
script fragment into a later frame. Record source ticks, generation fields,
scene/base/plane, ingame, draw, loop_cycle, script state and monotonic begin/end
for each fragment; serialize no names, chat contents or account credentials.

Exact host/nav content equality remains **unknown**: the current observe ABI
receives Client, name, run_sends and RandomStatus, not `&slot.snapshot`. The chosen
scalar bridge deliberately does not change that ABI or retain a snapshot borrow.
Equal gens, lengths, pointers seen at different times, or a digest do not establish
cross-shell content equality. We can rank two measured private capacities but
cannot select snapshot dedup on redundancy proof from this capture. That would
need a separately reviewed borrowed comparison seam; do not smuggle it in here.

## 3. Accounting contract

Emit schema `direct-owner-v1`. Each fixed family row has slot-instance token,
request/frame, owner/field enum, source epoch, `complete`, reason enum, element
len/capacity/occupied counts, occupied element bytes, capacity bytes, nested
capacity bytes, box count/bytes, and elapsed observer ns. Unknown is null plus
reason; never zero. Include coverage declarations for omitted owners.

Definitions, using the running binary's sizes:

- `V(v:T)` = `v.capacity() * size_of::<T>()`; occupied element storage uses
  `v.len()` instead. `E(v)` = V(v) plus nested storage of initialized elements
  only. Spare capacity is allocated element storage, not occupied values.
- `S(s)` = String capacity; logical content bytes are its len. Optional strings
  contribute zero iff absent, not their inline Option header a second time.
- For actions `Vec<Option<String>>`, E means V<Option<String>> plus S for
  each Some; for Vec<String>, V<String> plus each S. All arithmetic is checked;
  overflow invalidates the row rather than saturating into a plausible total.
- `B(Some(Box<T>))` = `size_of::<T>()` plus that object's nested storage. None
  contributes zero. Count the containing pointer slot only in its parent.
- Inline structs/arrays/Options/Vec and String headers are already in their
  containing allocation/header. Report `size_of::<Client>()`, World and shell
  structural headers separately; do not add World again on top of Client or
  report stack headers as separately allocated heap blocks.
- These are directly observed Rust logical requested-storage formulas, not
  allocator usable bytes, allocator metadata, total malloc live bytes or RSS.
  No `malloc_usable_size`, purge, page attribution, or subtracting these totals
  from RSS. Arc control-block/alignment overhead is a separate estimate/unknown,
  not exact payload. V8 used/total, native isolate/runtime and GPU remain separate.

### Client / World field formulas

C core/world.rs:35–104 fixes the types; walk the actual live vectors, not dimensions
from a serialized scene. A grid `Vec<Vec<Vec<T>>>` contributes outer capacity ×
`size_of<Vec<Vec<T>>>`, each initialized level capacity × `size_of<Vec<T>>`, and
each initialized row capacity × `size_of<T>`, plus initialized T descendants.

| Field | Exact covered formula / occupancy |
|---|---|
| World.squares | Grid formula for `Option<Box<Square>>`, plus B for every occupied grid Square and every owned `linked_square` chain. C dash3d/square.rs:11–55. Iteratively follow linked chains, do not recurse the stack unboundedly. |
| Square descendants | B separately for ground, overlay_stamp, wall, decor, ground_decor, ground_object. Ground has inline fixed arrays (dash3d/ground.rs:61–85), stamp is scalar input state; wall/decor/ground-decor/object are scalar placement/descriptors, not decoded models. quick_ground and five sprite indices are inline in Square; no additional allocation charge. Record mesh presence even with draw=false; do not clear it to fit expectations. |
| World.sprites | V for `Option<Sprite>`; count Some entries. Sprite is inline scalar data (dash3d/sprite.rs:13–41), no Box charge. Square arena indices and dynamic_sprites indices are references, not additional Sprite allocations. |
| World.free_dynamic_sprites / dynamic_sprites | V for usize / Option<usize>, actual len/capacity and occupancy; do not confuse `Option<usize>` with pointer size. |
| World.occluders / occlusion_cycle | V for Option<Occlude> / i32, occupied occluders separately. Do not add sizeof(Occlude) again for inline Some values. |
| World.groundh and Client.groundh | Separate grid formulas for i32, both actually borrowed. C client.rs:360–363 and World:49. No equality or shared storage assumed from mirrored contents. |
| Client.mapl / collision | Grid formula for u8; four collision.flags Vecs use capacity × `size_of<[i32;104]>()`, NOT a Vec-per-row formula (dash3d/collision_map.rs:20–25). CollisionMap headers are inline Client members. |
| players / npc | V for `Option<Box<ClientPlayer>>` / `Option<Box<ClientNpc>>`, occupied B payload plus entity descendants. player_ids, npc_ids, entity_removal_ids, entity_update_ids use V<i32>. Counts may differ from len; retain both. |
| Entity descendants | C dash3d/client_entity.rs:13–68: route_x/route_z V<i32>, route_run V<bool> (ordinary Rust Vec<bool>, not C++ bit-vector), optional chat_message S. Player additionally S(name); local_player is inline Option, so only descendants, no Box charge. `loc_model` presence is counted but its model graph is unknown; never lazily decode it. C client_player.rs:59–85; client_npc.rs:7–14. |
| player_appearance_buffer | V for `Option<Box<Packet>>`, B(Packet) per occupied slot plus data capacity. C io/packet.rs:50–77 exposes only a slice/length, not capacity: add **new** feature-gated `Packet::owner_data_capacity(&self)` in that module. pos/bit_pos/Isaac are inline; packet caches/stream queues are not covered by this row. |
| stat_base_level / stat_effective_level / stat_xp / var / var_serv | V<i32> for each, C client.rs:390–396. |
| ifaces_mut outer | Arc<Vec<Option<Arc<IfTypeMut>>>>: record outer data identity and V<Option<Arc<IfTypeMut>>>; count once as shared-template when ptr_eq(template), otherwise slot-private. Its Vec header is one Arc payload, not one per holder. |
| ifaces_mut entries | At each index compare actual entry with template entry. Shared identity is counted in template only. Diverged Some contributes sizeof(IfTypeMut) + optional link_obj_type/link_obj_number V<i32> + S(text) + S(graphic_name), C config/if_type.rs:156–178. If duplicate inner identities occur at several indices, count one allocation and report holder count. Do not use strong_count as client count. |

For inner COW identity de-dup use a fixed-capacity scratch table of borrowed
addresses within that one read; drop/clear it before returning. Table exhaustion
invalidates COW completeness. Compare against the live template by index and
also detect any alias to another template entry before calling it private.
Identity does not survive an epoch: allocator address reuse must never imply
retention. No allocation-address history is written to disk. Cache and immutable
ifaces Arc identities are reported shared, their nested payload stays unmeasured
and outside the private subtotal. This is not a full Client census:
ground_obj/LinkLists, stream queues, packet thread-local pools, audio, ancillary
maps/strings and loc_model descendants remain named unknowns. Large covered
families can select a lead; a small covered subtotal cannot exonerate Client.

### Both GameSnapshot shells

Add the accounting method inside api snapshot.rs so all actual capacities are
visible. Enumerate every heap-bearing field in H:536–687. The method returns a
fixed family array, not a cloned GameSnapshot or serde round-trip.

- npc: E<Vec<NpcView>>, nested optional name/overhead_text and actions. players
  and optional local player: their ActorView's same fields (H:85–206).
- stats: V<StatView> + S(name) per entry; inv: V<(i32,i32)>; chat: optional S;
  varps: V<VarpView>; scene.collision_flags: V<i32> (H:208–225,486–497).
- loc: V<LocView> + optional S(name/description) + E(actions).
- ground_item: V<GroundItemView> + item def optional name + E(actions).
- inventory/equipment/bank/bank_side, trade.my_offer/their_offer/side_pack,
  shop.stock: E<ItemView>; nested def.name and actions only. trade.partner S.
  ItemDefView's remaining fields are scalar (api obj_names.rs:17–26).
- widgets: V<WidgetView>, five optional text strings, optional scripts outer
  V<Option<Vec<i32>>> and each present V<i32>, optional comparator/operand
  V<i32>, varp_bindings V<WidgetVarpBindingView>, E(actions), E(items).
- side_tabs: V<SideTabView> + E(tab.widgets) for each actual nested vector.
- chat_lines: V<ChatLineView> + optional username S + text S; chat_options:
  V<ChatOptionView> + text S; make_products: V<MakeProductView> + name S +
  V<MakeButtonView>; quest_statuses: V<QuestStatusView> + name S.
- menu_entries/main_modal_texts/chat_modal_texts: V<String> + each S;
  login_message S. Other scalar/gate fields are included in shell header only.

Reuse the source-visible payload algorithms, not their misleading scope:
H api `snapshot_dedup.rs:527–595` has widgets_payload_bytes_vec,
side_tabs_payload_bytes_vec and locs_payload_bytes_vec, callable without enabling
dedup (api lib.rs:15–18). However `actions_bytes` at 520–524 uses slice LEN, not
Vec capacity. Exact accounting requires the spare-capacity term for EVERY
Widget.actions, nested Item.actions and Loc.actions. Implement a bounded
capacity-aware diagnostic walker using those verified field lists; tests compare
it to the existing helpers plus these explicit corrections. Do not silently
change existing dedup accounting or call its allocating registry census.
The old helpers also have no traversal budget, so are test oracles for admitted
small fixtures, not unguarded live calls. No whole snapshot fixture is counted
twice: host and nav each supply their own live values/capacities.

### Script fingerprint and buffers

H isolate_fb.rs:1315–1448 supplies the complete owned fingerprint field list.
Use E for each actual vector, with these element descendants:

- ItemRowFp: optional name S + V<String>(ops) + S per op. Applies to inv, bank,
  bank_side, equipment, trade_mine/theirs/side, shop_stock.
- SceneEntityFp: optional name S + V<String>(actions) + S per action. Applies
  to npcs/locs/players/ground. Do not confuse these with GameSnapshot LocView.
- stats and chat_lines tuples: S of their String; BankStandFp: S(name/kind)
  and optional choose S; CombatStyleFp: label S (combat_styles/spell_buttons).
- MakeProductFp: name S + V<MakeButtonFp>; nearest_booth: name/op S.
- chat_options V<String> + each S; chat_text/my_name/trade_partner optional S.
- booths, varps, side_tab_ifaces use V of TileInput, VarpInput,
  SideTabIfaceInput respectively; their fields are inline scalar rows.

Fingerprint Option/header is inline SlotScript; only its descendants enter the
heap subtotal. Also record pending_logs V<String> + each S and last_error S.
Compiled trait-object/native isolate internals are unknown. On Stop,
slot.rs:174–191 clears last_snapshot/world identity and replaces ipc with a NEW
IsolateBuf; do not claim the remaining builder owns zero capacity. Record null
builder-capacity after Stop too. Existing V8/inflight metrics are separate gauges.
The natural posted Vec's length/capacity is a transient single-buffer observation,
not simultaneous retained channel capacity. Once moved to LoadIsolate it cannot
be borrowed by this host hook; queued/in-flight Rust buffer capacities and the
isolate-side interact/paint builder remain unknown. No retention instrumentation
in load.rs and no new ACK, synchronization or changed timeout in this design.

## 4. Bounded observation and phase wiring

New feature name: `memory-owner-capture`, absent from all defaults. TUI forwards
to host-play; host-play forwards to host/api/script/client and requires its
existing memory-profile-no-alloc. Host forwards api/client as needed. Script
feature includes load. Client feature only exposes the World/Packet diagnostic
methods. Enabling snapshot-dedup simultaneously is rejected for this v1 capture.
System allocator, all existing scheduling/responsiveness/render profiles OFF,
RasterOff, unchanged memory mode/audio/guardian/packet behavior.

New runtime switch `BOT_MEMORY_OWNER_CAPTURE=1` is read once at initialization,
not per frame. It is valid only for the declared N1 active diagnostic. No hook
work when off. Immutable configuration and preallocated mailboxes live only in
the diagnostic fixture. No product barrier: memory::Run::poll publishes scalar
requests at phase offsets; slots answer on their next existing loop. Neither
Run::poll nor the slot waits for the other. A missing reply fails capture after
its deadline without postponing the observe-end Stop.

The feature-only wiring in memory.rs is necessary because the profile-off
fixture has no owner census requests and no sampled slot-join barrier. It must
be independently reviewed with generated tests proving original Stop timing and
packet/action/tick order. Do NOT turn on cohort shutdown: H memory.rs:1067–1078
calls script_stop and starts teardown while clients survive. Request the
post-Stop row during that existing teardown, not after slot joins. No
allocator purge, pause, drain extension or restart to obtain a cleaner row.

Hard observer limits (instrumentation admission requirements, not measured facts):

- Fixed family-row enum; at most 128 aggregate field rows per fragment.
- At most 262144 visited entries/owned nodes per fragment; check deadline every
  256 visits and before each linked-square step. No content hashing/string-byte
  scans; len/capacity only. Five milliseconds per fragment is the failure budget,
  checked cooperatively; record overshoot and suppress remaining census work on
  failure. This is not a hard real-time guarantee.
- One COW scratch address table, at most 16384 identities, at most 512 KiB;
  preallocate before login. Overflow is explicit incomplete, not a truncated
  subtotal advertised as total. Scalar mailboxes/staging/output row storage
  together at most 256 KiB. No new observer worker thread or unbounded channel.
- A publisher uses nonblocking mailbox access; a busy/full mailbox fails that
  request rather than blocking the client. All owner borrows end before output
  serialization/file I/O, which runs in the existing harness poll path.
- At most three owner requests and four natural-buffer metadata rows (initial
  keyframe, first delta, one first post per active request window). No per-tick
  logs, retained history of owners, payload serialization or deep copies.
- Owner JSONL maximum 256 KiB; all run output maximum 64 MiB; supervisor rejects
  growth beyond the cap. Keep timing/cap failure receipts even if rows fail.

Observer accounting reports reserved scratch/mailbox bytes, touched bytes if
measured, row/file bytes, visits, maximum and sum elapsed hook time, and misses.
Sum of spans is observer wall work, not per-owner process CPU. Native generated
qualification must measure incremental observer CPU with thread CPU timing and
allocation counters isolated to the test process. The live run reports its own
process CPU/RSS as perturbed context only. Before any future clean comparison,
review actual observer overhead and use capture OFF in clean binaries/cells;
never subtract a guessed constant from the old N1/N16 RSS or CPU.

## 5. ONE later live procedure (root release required)

1. Tool implementation and generated/offline qualification first, separate review.
   Root then admits exact original H/C blobs plus ONLY the reviewed instrumentation
   overlay, lockfiles, feature graph, compiler/target, tools and binary hashes.
   Build an isolated source tree from Git objects, not this checkout's moving
   dependencies. No new production optimization or tiled-nav adaptation. The
   artifact is a diagnostic derivative, NOT the old a0c6eb0b executable.
2. Native Linux, existing Concord host, real 120x40 PTY, N=1, active Thiever
   fixture, 30s established warmup / 120s observation / 60s after-script-Stop
   teardown. This is a deliberately short diagnostic, not the historical
   120/600s acceptance-shaped sample. Reuse the existing reviewed N1 controller
   selection extension e707e2d and existing sustain behavior; no new script.
3. Identity references are under
   `diagnostics/current-tui-n1-1727/current-tui-n1-1727-final/`:
   `inputs/freeze.json`, `inputs/build-manifest.json`,
   `n1-controller-e707e2d/install-manifest.json`,
   `20260908T172715Z_tui_n1_active/metadata.json`, and
   `managed/result.json.cells/result/{cell_spec,cache-provenance}.json`.
   Bind the same cache snapshot version `2faf336eeb0462ed`, all cache file hashes,
   catalog and nav flags/pack hashes in those receipts. Nav pack SHA is
   `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`.
   Operator clarification approved 2026-09-10: preserve the existing N1
   account-selection and seed contract. The diagnostic mints one disposable
   account after frontend start and creates its throwaway vault. Before launch,
   root privately binds the exact fixture recipe, settings, N=1 active workload,
   source/binary, cache and live server identities. Prelaunch account/population
   counts admit that one-slot fixture; they do not claim a named save already
   exists, is logged in, or has qualified. After launch, establish the actual
   account from qualification slots[].name and client/workload observations,
   including ingame && scene_state==2, before accepting capture. Do not predict
   names, reuse a prior run's name, equate the recipe hash with account identity,
   or invent a hash-to-name join. Keep names/passwords out of the owner ledger;
   do not substitute the operator's highmem vault. Preserve all workload, frame,
   lifecycle, resource, cleanup and single-attempt requirements. Server/config/
   cache identities remain freshly observed; old PID726 is NOT a live identity.
   No cache rebuild, account reset, server restart, installation or permission
   change is authorized.
4. Root fresh preflight: no conflicting owned test/build/profiler/frontend,
   server healthy and separately measured, no swap/OOM, MemAvailable >=768 MiB,
   output filesystem free >=256 MiB. Runtime guard polls every 0.5s: stop owned
   diagnostic if MemAvailable <256 MiB, frontend RSS >512 MiB, output >64 MiB,
   or wall duration >360s from frontend launch. Limits are one-attempt ceilings,
   not permission to consume unrelated process resources. No AS cap guessed for
   V8 reservations. If preflight cannot admit these limits, no launch.
5. Ready means ingame && scene_state==2 with populated scene base/local player;
   active means Running, seeded and XP-proved per existing harness, not merely
   process-up. Preserve observe-start/end qualification and per-slot progress.
   Existing frame counters must advance and active fixture steals/XP increase
   across the 120s window; no script errors/disconnects. Record actual scene/base,
   guardian hold, draw=false and renderer absent. A capture during scene_state!=2
   is invalid for the planned steady census, not silently retried next scene.
6. Request A at observe+30s, B at observe+90s; take first complete pre-observe
   frame within 1s of each request. Script fragment must belong to that frame;
   capture one natural post metadata row within 2s, without forcing a tick.
   Record both actual shells and their epochs even if not equal. Initial
   keyframe/first-delta metadata are armed once before Start. No peak claim.
7. Existing observe-end path calls script_stop normally. Record its begin/end
   receipt and arm C at teardown+30s, reply within 1s, while Client/host/nav
   remain alive. Script row must be Idle with fingerprint absent; ipc is a new
   builder of unknown capacity. Require existing isolate/inflight gauges zero
   and ready=1 at final teardown. Finish normal exit at teardown end. This is
   after-script-Stop, NEVER post-slot-join or an allocator-retention proof.
8. Preserve metadata, source/overlay/tool/binary admissions, raw rows, qualification,
   resource series, bounds and cleanup receipts in a fresh root-chosen directory.
   Supervisor owns only its launched tree/PTY; on failure stop that tree, reap it,
   verify no owned descendants, preserve artifacts. Never signal the game server
   or unrelated helpers. No second attempt, extended window or alternate N.

Success requires complete A/B/C mandatory families, matching slot/frame/phase,
no bound/overflow/missing/duplicate row, natural buffer rows, workload/lifecycle
qualification, admitted identity and clean owned-process exit. Planned opaque
builder/native/client-ancillary unknowns remain null; they do not invalidate
covered-family measurement, but prevent a whole-process reconciliation. Any
unexpected missing family, unsupported sharing, stale frame, mesh/model-dependent
unknown in a claimed complete subtotal, identity drift or cap breach makes that
subtotal incomplete and the capture unqualified. Preserve partial facts without
turning them into a passing result. No automatic rerun.

### Discriminating outcome, not savings acceptance

Rank only complete private requested-storage families at A and B. Predeclare
1 MiB private capacity in BOTH active rows as a material lead; compare its scale
with the historical 6.964 MiB incremental target gap, without equating domains.
A family below 1 MiB in both rows is deprioritized for THIS N1 scene only, not
proved globally irrelevant. World square/descendant storage, spare vector
capacity, or private interface overlays above this threshold select one bounded
representation investigation with its actual consumers/lifetime. Unchanged
large capacity at C is evidence of still-live storage, not allocator retention.
Large snapshot families select an ownership investigation, NOT dedup acceptance:
exact equality and replacement costs remain unmeasured. Large fingerprint storage
that disappears at C selects script-retained payload as a lead; builder and
in-flight capacities remain unresolved. Choose the largest qualifying complete
family; ties remain explicit, no simultaneous optimizations. If none qualify or
unknown coverage dominates, report precisely the missing capability. Do not
attribute the residual to V8/allocator or call the campaign complete.

## 6. Bounded follow-up implementation and tests

This design card edits only this document. The later implementation card owns:

- C `crates/client/Cargo.toml`, `src/core/world.rs`, `src/io/packet.rs`:
  diagnostic feature, borrowed World counters and Packet capacity accessor;
  unit tests adjacent and new `crates/client/tests/direct_owner_capture.rs`.
  No other client behavior/render/model changes.
- H Cargo.toml feature wiring ONLY in crates/{api,host,host-play,script,tui}.
  `crates/api/src/snapshot.rs` accounting method and feature-gated new
  `crates/api/src/owner_capture.rs` scalar/budget helpers; api lib.rs module export.
  `crates/host/src/lib.rs` pre-observe seam and feature-gated new
  `crates/host/src/owner_capture.rs` scalar staging/mailbox interface.
  `crates/host-play/src/lib.rs` closure/script/COW seams; memory.rs request/publish
  wiring; new `crates/host-play/src/owner_capture.rs` Client/COW accounting/output.
  `crates/script/src/slot.rs` borrowed method and natural-buffer counters;
  `crates/script/src/isolate_fb.rs` fingerprint walker. No builder dependency edit.
- New `crates/api/tests/direct_owner_capture.rs`,
  `crates/host-play/tests/direct_owner_capture.rs`,
  `crates/script/tests/direct_owner_capture.rs`; adjacent host seam tests and
  existing `crates/host-play/tests/snapshot_frame_equivalence.rs` regression.
- A compact offline `docs/memory/validate_direct_owner_capture.py` and
  `docs/memory/test_validate_direct_owner_capture.py` for schema/count/coverage/
  phase/identity/cap/admission validation only. No broad parser or controller
  rewrite. Reuse the managed launcher/resource/cleanup machinery; an unsupported
  launch guard is a specific prerequisite to report, not permission to weaken it.

Required generated tests before root launch: empty/nonempty/spare capacities;
nested action capacity exceeding len (including side-tab items); linked squares;
sparse sprites counted once despite multiple tile indices; inline Option versus
Box; both heightmaps; independent/equal/different snapshot epochs; all fingerprint
field families; shared outer/inner COW identities, divergence and aliasing;
Packet capacity > length; disabled feature/runtime switch no observation work;
no snapshots/Arcs retained; bounds, deadline, mailbox-full and stale frame failure;
Stop clears fingerprint but builder capacity stays unknown; natural keyframe/
delta/bank-force/restart behavior unchanged; unknown != zero and no residual-RSS
inference. Exercise malformed/slow scripts, guardian hold, pause/resume, bank
publication, retained packet lifetimes and scene-change regressions already named
in performance-finish-plan §5. Reject launch on missing proof, not synthetic loops.

Run affected api/host/script/host-play/tui tests both with diagnostic feature and
feature off, retaining load and memory-profile-no-alloc where required. Run
client unit AND integration tests separately with its diagnostic feature and
without it; host cargo tests do not run submodule integration tests. Generated
Linux observer allocation/CPU/guard qualification and admitted-source manifest
checks are required before live release, not executed by this design task.

Routine same-card reviewer reviews this design/commit first. Root owns the later
card/release freeze, actual launches, evidence review, candidate decisions and
final whole-branch Grok 4.6. This plan neither changes STATE nor creates a release.
