# Revision 289 host/script delta

Date: 2026-09-07
Scope: bounded, read-only reassessment for the post-memory compatibility campaign. This is an evidence ledger, not an implementation plan or a claim of 289 behavioral compatibility.

## Sources and confidence

Primary 289 source: `/Users/acfrazier/experiments/FR-vault/research/deob/289`, RuneWiki/openrs2-nonfree branch 289, pinned `0c00ef249546fada67b1f6eb8bbe01ea7c250c95`. Inventory correction: the existing separate `vendor/client-java-289` checkout is additional local evidence; the absence of a vault `research/deob/289` directory was not evidence that all 289 material was missing. Inventory anchor: `/Users/acfrazier/experiments/FR-vault/docs/research/deob-289-source-inventory.md:3-15`.

Script baseline: `/Users/acfrazier/experiments/rs2b0t`, baseline `100adccc037d9f6898080e1cad58fcfc43364775`, revision-289 head `8e7d965be2071d6ec65c3265e12af797082d720a`. The exact comparison was made with `git diff` in that checkout. Current host evidence is from this checkout's Rust API/client sources. No build, live run, network action, cache update, vault edit, or product implementation was performed.

Evidence grades used here:

- VERIFIED: directly present in a named source or exact commit diff.
- PARTIAL: a related host seam exists, but the 289 semantic/packet equivalence is not proven.
- UNKNOWN: the named sources do not establish the required behavior.
- BLOCKED: a compatibility requirement must not be filled by a JS clone; a Rust host capability must be specified first.

## Minimum viable 289 bothost boundary

The smallest defensible boundary is a revision-selected client protocol/handshake layer plus a host snapshot/action adapter. It must keep protocol IDs, packet lengths, smart/short encodings, cache/archive loading, and lifecycle state in the Rust client/host, while scripts receive named snapshots and queue named host verbs. Do not port Java policy, tables, walkers, or routers into JavaScript.

Required boundary, in sequence:

1. Revision profile: client version, inbound/outbound packet tables and encodings, RSA/ISAAC/session handshake, and cache/archive index layout.
2. Lifecycle: attached -> login requested -> login response/error -> world/player update -> scene ready -> logout/reset. Snapshot publication must not expose `ingame` or `scene_ready` as equivalent merely because a socket is attached.
3. Read model: generation-stamped player, NPC, inventory, bank, widget/modal, chat, scene/collision, world and camera families.
4. Action driver: typed opcode-family actions (`npc`, `loc`, ground object, player, held item, widget/button, walk, side-tab, dialog/count, login/logout), with target identity and scene guards.
5. Settlement: tick/frame polling and explicit evidence predicates for arrival, item delta, XP, modal/dialog and server refusal.

Everything beyond this cut (quest policy, tutorial policy, guardian/random-event decisions, item/spell/location tables, transport policy and high-level banking/walking routers) remains script/application policy and is not a missing Java runtime to recreate.

## 289 inbound, outbound, login, cache and lifecycle seams

| Seam | 289 primary anchor | Current 274 host seam | Assessment for 289 |
|---|---|---|---|
| Inbound packet framing/dispatch | `client.java:6132-6142` flushes the stream; `client.java:6166-6178` rejects a player-packet size mismatch after decoding | `crates/api/src/prot.rs:1-6` and `:113-140` define typed outbound `Out`; current client has packet tests and server protocol tables | PARTIAL. A strict consume-length invariant is the right boundary, but 289 packet IDs/sizes/encodings must be versioned, not assumed from 274. |
| Player/NPC/world update | `client.java:6150-6175` runs player update methods and validates the final packet position; reset path `client.java:8457-8503` clears actor/scene/interface state | `crates/api/src/snapshot.rs` exposes generation-stamped `NpcView`, `PlayerView`, `WorldStateView`, `SceneView`; `crates/api/src/interact.rs:741-1175` rechecks snapshots before dispatch | PARTIAL. The family shape is reusable, but 289 actor slots, update masks, coordinate encodings and lifecycle reset semantics need a revision fixture. |
| Login response/lifecycle | `client.java:8504-8563` maps response codes 3-21 to invalid credentials, already logged in, full/offline/session/world/member/update outcomes; code 15 at `:8540-8553` clears stream buffers/state and returns to login | `crates/api/src/interact.rs:314-338` has login/logout primitives; `crates/api/src/interact.rs:1097-1114` guards login on attached/not-ingame; `crates/host/src/login_queue.rs:1-13` documents engine-side production limits | PARTIAL. Response classification and reset must be 289-specific. Host login FIFO is Lost City throttling, not proof of RuneScape 289 login compatibility. |
| Outbound interaction packet families | 289 menu/update code is visible in `client.java:6182-6220`; typed current table includes OP object/NPC/loc/player/held and IF button/count/close/walk rows at `crates/api/src/prot.rs:28-111` | `crates/api/src/interact.rs:357-470` defines `OpTarget`, `WireCommand`, refusal reasons and 5-slot operation families; `:741-1175` applies target/scene/precondition checks | PARTIAL. Keep the typed family model, but map each 289 opcode and payload from the primary client. Numeric operation slots are revision/UI data, not stable script semantics. |
| Cache/archive loading | `client.java:3694-3720` routes repeated on-demand failures to `method114("ondemand")`; `client.java:8462-8469` clears four 104x104 scene layers on reset | `vendor/fr-client-rust/crates/client/src/io/ondemand.rs`, `io/jagfile.rs`, `unpack/mod.rs`, and cache tests exist; `crates/nav/src/pack.rs` is a packed nav format (`274V`, version 8) | PARTIAL/BLOCKED for 289. Cache presence and nav-pack availability are not equivalent. Add a 289 cache/archive manifest and loader seam before any script assumes item/NPC/loc/widget definitions. |
| Scene readiness/navigation | 289 reset clears scene arrays at `client.java:8462-8469`; scene/map update sequencing is not established by the bounded anchors above | `crates/api/src/snapshot.rs` has `SceneView`; `crates/api/src/interact.rs:1038-1077` refuses unavailable/out-of-scene walks; `crates/nav/src/router.rs`/`traveller.rs` provide host navigation | PARTIAL. Walk is a host verb; the 289 collision/map build and plane/base coordinate rules remain UNKNOWN. |
| Tick/lifecycle settlement | `client.java:6105-6137` runs cycle/stream flush work; `client.java:6166-6175` validates a decoded player update before accepting it | `crates/api/src/settle.rs:133-321` supplies arrival/item/xp/modal/chat/scene predicates; snapshot tick is documented in `crates/api/src/snapshot.rs` | PARTIAL. Need a 289 capture proving which packet/frame edge advances the published tick and when scene/player state becomes usable. |

## Script-facing ABI comparison: baseline 100 vs revision 289 head

The exact `packages/rs2b0t-api/index.d.ts` comparison is VERIFIED unchanged (`git diff --quiet ... -- packages/rs2b0t-api/index.d.ts` returned exit 0). Therefore the exported spelling and declared TypeScript shapes did not change between baseline and `8e7d965`; that does not prove behavior compatibility.

The revision-289 commit changed 33 files: 1,122 insertions and 251 deletions. The relevant changes are behavioral/script dependency changes, not an API declaration migration:

| Area | Exact 289-head change | Compatibility consequence |
|---|---|---|
| Banking | `src/bot/scripts/BankFletcher/BankFletcher.ts` now imports `Banking`, removes leash-radius settings, and calls `Banking.open({ stand, boothName, boothOp, log })` (diff hunks around lines 6-20, 462-510) | API spelling is unchanged at the declaration level, but runtime depends on nearest-booth resolution and fallback stand semantics. Current host has `Interactions::open_nearest_booth` at `crates/api/src/interact.rs:896-926`; exact booth operation/arrival semantics are only PARTIAL. |
| Inventory packet shape | `src/client/shell/Client.ts` changes inventory size from `g1` to `g2` and changed slot decoding from `g1` to `gsmart` (diff hunk around lines 6466-6504) | This is a concrete protocol/behavior delta hidden behind unchanged script types. Current `ItemView`/inventory snapshot is usable only after a 289-specific packet decoder and fixture. Numeric item IDs and slot IDs must remain revision data. |
| BankFletcher modes | `BankFletcher.ts` adds `mode` settings (`auto`, `cut`, `string`, `cut+string`) and a cut-first then string phase; `BankFletcherLogic.ts` adds `FletchMode`, `keepNames`, `needsRestock`, and `hasFletchWork` branches | Scripts depend on inventory counts, bank list settling, item identity and action labels. Current host has bank/inventory families and `BankBudget`-style guards, but the two-phase policy is application logic, not a host capability. |
| Fire-staff selection | `SuperheaterLogic.ts` changes from one `FIRE_STAFF` to `FIRE_STAVES`/`pickFireStaff`; `Superheater.ts` checks inventory/bank candidates and waits for Wield/Wear actions after closing bank | The ABI names remain usable, but object-name catalogs and widget/menu action labels are behavior dependencies. Current host `ItemView` has IDs/actions/component IDs; exact 289 definitions are UNKNOWN until cache is loaded and verified. |
| Packet tables | `src/client/io/ClientProt.ts` and `ServerProt.ts` both changed substantially (85/92 added/deleted lines in the exact diff), plus config decoding changes | This is the strongest evidence against “scripts stay compatible” as a blanket statement. Script calls may spell the same, while the client sends/reads different packets. A revision profile is prerequisite. |
| Strict packet checks | `src/client/io/packetGuard.ts` added and `Client.ts` added opt-in strict packet consumption | The current direction matches 289's own player-packet size check, but current strictness is test/e2e policy and is not a proof of 289 framing. |

### Representative API shape/dependency matrix

| Representative slice | Shape status baseline -> 289 | Semantic status against current Rust host |
|---|---|---|
| Login | Declared API unchanged; `Game.ingame()`/driver login spelling unchanged | PARTIAL: current `Interactions::login` exists, but 289 response codes, handshake, RSA/ISAAC and reset are UNKNOWN. |
| Tick/frame | Declared API unchanged; `Execution` and state reads unchanged | PARTIAL: current settle/snapshot tick exists; 289 tick edge and frame-vs-server-tick timing UNKNOWN. |
| Player info | Declared API unchanged (`Game`, `Player`, `Players`) | PARTIAL: current player snapshot includes slot/name/tile/target/combat fields; 289 update masks and exact target/index decoding UNKNOWN. |
| Inventory | Declared API unchanged (`Inventory`, `InvItem`) | PARTIAL/BLOCKED until 289 packet width/smart-slot and cache object definitions are wired; commit proves those encodings can change under same ABI. |
| Bank | Declared API unchanged (`Bank`, `Banking`, `withdrawOp`) | PARTIAL: current bank views/actions and nearest-booth verb exist; list settling, item count semantics and 289 widgets need proof. |
| Navigation | Declared API unchanged (`Traversal`, `DirectNavigator`, `Tile`, nav flags) | PARTIAL/BLOCKED: host `walk`/packed nav exists, but 289 map base/plane/collision and teleport/catalog policy are not established. |
| Numeric/object/widget IDs | Declared field shapes unchanged, but the 289 commit changes packet/config decoding and uses component IDs in scripts | UNKNOWN for cross-revision identity. Never carry 274 IDs into 289 without cache/config provenance; widget IDs are interface/cache data, not ABI constants. |
| Packet actions | Declared `.interact(action)` shape unchanged | PARTIAL: current host maps named actions to 1..5 operation slots, but 289 menu action order/opcodes may differ. |
| Cache/world | No public ABI declaration change | BLOCKED: no script-side cache/world clone. Host must publish a 289 loader/manifest or scripts must fail closed. |
| Tutorial/guardians/random events | No relevant declaration change established | BLOCKED/UNKNOWN: these are policy and lifecycle surfaces. Do not infer support from labels, TypeScript compilation, or a passing isolate. |

## What is verified, unknown, and missing

Verified:

- The 289 source inventory and pin are primary/provenance evidence, including the oldlocal/vendor correction.
- rs2b0t API declarations are bytewise unchanged between baseline `100adccc` and 289 head `8e7d965`.
- The 289 head changes packet tables and concrete packet decoding (`g2` inventory size; `gsmart` slot), while changing high-level scripts to use nearest-booth and multi-staff behavior.
- Current Rust host has typed outbound rows, snapshot families, interaction preconditions/identity checks, navigation guards, and settlement predicates.
- 289 directly validates at least one decoded player packet's final position (`client.java:6166-6168`), supporting strict framing as a required boundary invariant.

Unknown or missing host capability:

- 289 login handshake bytes, RSA/session/ISAAC sequencing, and complete response-code state machine.
- Complete 289 ClientProt/ServerProt ID/length/encoding map and fixtures for variable packets.
- 289 cache/archive index and config schema sufficient to resolve object, NPC, loc, sequence, widget and spell IDs.
- 289 scene build base/plane/collision timing and a proof that `scene_ready` is the correct action gate.
- Stable 289 widget component IDs/action order for bank, dialog, tutorial and combat controls.
- 289 semantics for player/NPC target indexes, local-player slot, inventory/bank slot/count updates, and world-region rebuilds.
- Guardian/random-event claim/hold lifecycle and tutorial-specific transitions.
- A 289 live or replay proof connecting the unchanged script ABI to the 289 host behavior.

Fail-closed rule: any implementer needing one of these absent verbs or data surfaces must report `BLOCKED: missing <op>` and must not recreate it in JS. The host should expose `not impl`/refusal until the Rust capability is explicitly scoped and reviewed.

## Sequencing recommendation after memory

1. Freeze a 289 source/cache manifest and revision profile; do not start with script ports.
2. Add offline packet fixtures for login, player-info, inventory-full/delta, region/scene, widget/interface and logout/reset. Verify declared lengths and consumed positions.
3. Implement the 289 client handshake/lifecycle adapter and generation stamps, then validate attached/login/ingame/scene-ready/reset transitions.
4. Reuse the current Rust snapshot/action abstractions only where the 289 fixtures confirm their semantics; keep numeric IDs and action slots sourced from 289 cache/config.
5. Add the minimum action cut (walk, NPC/loc/object/player/held/widget, bank open/withdraw/deposit, dialog/count, login/logout), with target and scene guards.
6. Add script name maps only after the host verbs exist. Keep banking/fletching/superheating/navigation/tutorial/guardian policy in scripts or Rust application code as authorized, never in a foreign-runtime clone.
7. Run representative replay proofs (login/tick/player-info/inventory/bank/navigation) and mark each independently PASS, FAIL, or BLOCKED. TypeScript compile and isolate startup are not behavioral proof.

## Follow-up questions/source gaps

1. Which 289 cache/archive artifact is authoritative for this host run, and where is its checksum/manifest? The source inventory names the jar and cache artifacts but does not establish a host-loadable 289 cache contract.
2. Which server endpoint/revision handshake should the 289 bothost target? The deob source gives client behavior and response text, not an operator-approved endpoint or credential/test fixture.
3. Is the existing `vendor/client-java-289` checkout authoritative for additional 289 behavior, or only historical context? Its pin is recorded, but current availability of its old upstream ref is not asserted.
4. Should the first implementation cut support only login/player-info/inventory/bank/navigation, with tutorial/guardian/random-event surfaces explicitly `not impl`? This is the recommended minimum; policy expansion should wait for a separate capability brief.
5. What replay/fixture format will be accepted for packet and lifecycle proofs before any live test is authorized?

Conclusion: “scripts stay compatible” is true only as a narrow declaration-level statement. The exact 289 diff proves unchanged names can conceal changed packet encodings, config IDs and behavior. Treat compatibility as PARTIAL until the revision-specific Rust boundary and representative replay proofs exist; fail closed on missing host capabilities.
