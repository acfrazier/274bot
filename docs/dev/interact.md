# Interact: acting through `Driver` and `Interactions`

> **Developer reference.** This page is for people who change 274bot.
> To use 274bot, start at the [README](../../README.md); to find another page, see the [docs index](../README.md).

Actions go through `api::interact`. The send boundary is the `Driver` trait;
`Interactions` is the orchestration layer above it (target identity,
label→opcode resolution, refusal reasons). A `true` return means the driver
**accepted the send**, not that the server applied it — confirm effect via
snapshot + settle evidence.

## `Driver` (the dispatch boundary)

`Client` implements `Driver` over `doAction`/`tryMove`/`out`; tests use a
recording stub. The kernel never injects a raw opcode that skips ISAAC.

| Helper | Mechanism |
| --- | --- |
| `press(driver, iface_id)` | menu slot 0 = `IF_BUTTON` + `doAction(0)` |
| `set_run(driver, on)` | `press` 153 (on) / 152 (off) |
| `walk(driver, x, z)` | `tryMove` from the local route, type 0 |
| `walk_nearest(driver, x, z)` | `tryMove` with a nearest-reachable fallback when the tile itself is solid |
| `op_loc(driver, x, z, loc_id)` | `OP_LOC1` via `set_menu`/`do_action` |
| `interact(driver, slot)` | `doAction(slot)` on a prepared slot |
| `close_modal(driver)` | `CLOSE_MODAL` via `Out` |
| `answer_count(driver, n)` | `RESUME_P_COUNTDIALOG` via `Out` |
| `login(driver, u, p, reconnect)` | `Client::login` handshake |
| `logout(driver, ifaces)` | `IF_BUTTON` on the `CC_LOGOUT` iface (client code 205) via `doAction`; missing iface is `false`, no panic |
| `cheat(driver, cmd)` | `CLIENT_CHEAT` via the driver's `send_cheat` (body as given — `tele`/`setvar` have no `~`; debugprocs are `~name`); refused unless the profile admits cheats (local profiles only) |
| `mainland_hop(driver)` | two `cheat`s: `tele` courtyard + `setvar tutorial 1000`; returns without sending unless `cheat_allowed(driver.cheat_admission())` |
| `seed_at(driver, level, x, z)` | `setvar tutorial 1000` then `tele level,mx,mz,lx,lz` |

`Driver::click_side_tab(tab)` flips the client's local active side icon
(no packet). `Driver::loc_typecode(sx, sz)` reads the packed wall/decor/
scenery/ground-decor typecode at a scene tile.

## `Interactions` (the orchestration layer)

`Interactions<'a>` holds `&'a GameSnapshot` + `&'a mut dyn Driver` and
returns `SendResult` (a `WireCommand`, or a `SendReason` refusal). Its 26
methods: `apply_amount_key`, `interact(target, action)`, `wear(id)`,
`unequip(id)`, `use_item_on(item, target)`, `use_widget_on(widget, target)`,
`press(widget)`, `if_button(widget)`, `continue_dialog(component_id)`,
`open_nearest_booth()`, `open_booth_at(tile, expected_id)`,
`open_named_booth_at(tile, expected_id, expected_name, expected_action)`,
`answer_choice(option)`, `close_modal()`, `answer_count(value)`,
`shop_buy(name, chunk)`, `shop_sell(name, chunk)`, `walk(tile)`,
`walk_nearest(tile)`, `pending_door_step(tile)`, `click_side_tab(tab)`,
`login(user, pass)`, `clear_local_modal(component_id)`, `set_run(on)`,
`set_retaliate(on)`, `set_note_mode(on)`. Each does: precondition (attached/ingame/scene/
count-dialog) → target identity (`still_present`) → label/opcode resolve
(`operation_of`/`offers_operation`) → dispatch a `WireCommand` through
`Driver`.

- `OpTarget` is an enum of view refs: `Npc`/`Player`/`Loc`/`GroundItem`/
  `Item`.
- `WireCommand` has 13 kinds: `Op`, `UseItem`, `UseWidget`, `Button`,
  `Continue`, `Close`, `Count`, `Walk`, `WalkNearest`, `DoorStep`,
  `SideTab`, `Login`, `ClearLocalModal`.
- `SendReason` has 21 refusal reasons (`NotAttached` … `DriverRejected`).
- `ActionResolution::operation_of`/`offers_operation` scan the target's
  actions for a label (trimmed, case-insensitive) or a 1..5 operation
  number. `TargetIdentity::still_present` checks per-kind identity (npc by
  index+id, player by index+name, loc by typecode+layer+tile, ground item
  by id+tile, item by id+slot+component within its container).
- `close_modal`/`clear_local_modal` dispatch via `CLOSE_BUTTON` do_action so
  the client's local modal state is cleared (not just the wire write).

## Server-ordered input dispatch

`api::interact::server_ordered::ServerOrderedInputs` is the shared dispatcher
for selected-content requests and advanced typed packet batches. This mechanism
has no product consumer yet. It does not add a script declaration, JavaScript
binding or new Gatherer, bank or dialogue behavior.

Native content uses `ActionContext::emit_server_ordered`. A request uses one
owner reservation, one request id and one completion. Its receipt contains
`request_id`, `dispatch_tick`, five fixed row results, `row_count`,
`dispatched_prefix`, `wire_events` and an optional typed terminal error.
The existing completion envelope retains run and observation identity.
`HostQueued` means that the normal encrypted client sink accepted the bytes.
It does not prove that the server applied them.

The dispatcher checks the full request before the first packet. The knife
request checks the selected `knife.knife` and `knife.logs` item identities,
inventory generations and distinct slots. These identities come from the
selected content extractor, not item names or a hand-written id table.
The selected NPC transform proofs currently admit no numeric NPC operation.
The native request surface cannot construct an advanced envelope or its permit.
The private advanced surface uses the same dispatcher, packet validation and
cheat authorization as the named content path.

The client sink reports complete packets without copying packet bodies.
Slot input state keeps a monotonic input count, per-observed-tick category
counts, a USER_EVENT input epoch and an owned content expectation. Advanced
input invalidates the expectation even when its packet is a CLIENT_EVENT.
Ordinary housekeeping packets do not count as dispatched input. Accepted
zero-wire local prompt edits also invalidate the expectation.

Counts never refuse input or reserve engine capacity. Only a newly observed
tick resets per-tick counts. Native batch charging applies to that runtime's
own emission frame; it does not exclude other producers in the drain tick.
The server alone defers input that exceeds its decode-pass limits.


## `LEGAL_SEND` / `ClientProt`

`api::prot::LEGAL_SEND` is the complete outbound table (opcode id + fixed
length, `-1` variable). Typed builders (`api::prot::Send`) construct the
kernel's sends. Key opcodes on 274: `IF_BUTTON` 9, `CLOSE_MODAL` 51,
`RESUME_P_COUNTDIALOG` 102; 289 remaps them (86 / 93 / 180 — see
`client_prot_289.rs`). `OPNPC1..5`/`OPLOC1..5`/`OPOBJ1..5`/
`OPPLAYER1..5`/`INV_BUTTON1..5` per the table in `api::prot`.

All 82 named revision-289 rows have a closed `Send` builder. `Send` fields are
private. Builders reject integers outside their wire domains, invalid base-37
names, invalid abuse reasons and incomplete variable shapes. `PacketError`
describes each refusal. Count resumes keep their signed `i32` domain.

Use `Driver::send_packet` to emit a typed packet with the driver's bound
revision. `Send::write_for_revision` also accepts the concrete client `Packet`
and checks the full body before it changes the output cursor or ISAAC state.
Neither method accepts an arbitrary opcode or a caller-supplied encoded body.
Fixed packets and movement packets need no heap allocation.

`Driver` has the same required and provided methods with every feature set.
Every implementation supplies `send_packet`, including recording drivers.
It has no raw sink accessor. The separate `api::prot::Out` recorder trait is
available only to API unit tests or with `api/test-support`.
Recorder crates enable that feature in dev-dependencies. Harnesses compiled
outside `cfg(test)` forward it through their own `test-support` feature.
In particular, `host-play/test-support` enables `api/test-support` for its
Linux harness, including frontend test builds. Product builds enable neither
feature and do not expose the raw recorder trait.
The client fork still exposes its public `Client::out` field. The native
source fence rejects advanced permit construction and emission from native
machines. External script bindings are not activated by this mechanism.

The typed driver path remains available in product builds:

```no_run
use api::interact::Driver;
use api::prot::{PacketError, Send};

fn close(driver: &mut impl Driver) -> Result<(), PacketError> {
    driver.send_packet(&Send::close_modal())
}
```

These examples must fail against a product build without `api/test-support`:

```compile_fail
use api::prot::Out;
```

```compile_fail
use api::interact::Driver;

fn raw_write(driver: &mut impl Driver) {
    driver.out().p1_enc(1);
}
```

Movement builders accept at most 24 signed offset pairs. The minimap builder
requires a `MinimapTrailer`. Mouse telemetry accepts complete typed records.
Chat builders use the client `WordPack` codec. They refuse unsupported
characters and text over its 80-character limit instead of losing text.
Variable framing permits at most 255 body bytes. The framing validator also
checks the engine's 1600-byte VAR_SHORT limit, although 289 defines no such row.

Use `Base37Name` for every social, private-message and report name.
`AbuseReason` contains the twelve report reasons. `prot::effects::effects`
accepts a completed `Send`, never a revision-selected opcode row. Each effect
is `Effect::Yes`, `Effect::No`, or `Effect::DependsOnContent`. A `No` requires
handler evidence. Content-dependent effects remain unknown, not false.
These classifications do not prove that a particular request succeeded.

The `CLIENT_CHEAT` builder requires the selected debug catalog and validates
the command with its formatter. Its catalog revision must match the session.
This packet value does not grant cheat authority. Dispatch must still enforce
the concrete driver's Local/loopback and live-session admission. The existing
checked cheat helper retains that boundary and its byte encoding.

## Server-ordered requests

Named content requests and advanced typed packets use one host drain owner,
one request ID and one completion. The dispatcher checks every row before
writing the first packet. It checks live ownership again before each row.
Cancellation retains the committed prefix; it does not replay that prefix.
The receipt records host dispatch, not server acceptance.

`ServerOrderedInputState` holds the slot's one content expectation and bounded
continue exclusion. A resume needs the exact run, predecessor, phase, input
epoch and session proof. Reset needs complete closure, a new owned Talk-to
and its newly observed initial page. A timeout or repeated old page cannot
release the exclusion.

The encrypted sink is the only packet-counting point. USER_EVENT advances
the input epoch; CLIENT_EVENT counts as input without advancing that epoch.
Ordinary housekeeping does neither. Any advanced packet invalidates the
content expectation. Accepted foreign local edits invalidate it even when
they write no packet. Counts can exceed five and never control admission.
The engine owns input deferral. The native emission frame still charges its
own existing batch allowance, not another producer's slot-wide traffic.
The client completes any open packet observation before its socket flush.
The drain also completes a previous producer's packet before changing lineage
or capturing a request's input proof. Each row counts only its own commits.
These observation boundaries do not change packet bytes or cipher steps.

Native content needs no external declaration. The advanced Rust envelope
requires a private, run-bound permit. No production permit issuer, script
binding or native consumer is activated here.

On 64-bit macOS, the permanent dispatch state adds 256 Rust live bytes per
active native slot: 208 bytes in `SlotInput`, 32 bytes for its outbound
observer and 16 bytes in the existing native ledger. `Packet` remains
2,128 bytes and `Client` remains 28,872 bytes. The observer allocates once;
replacement, socket adoption and packet commits reuse its state. Inbound,
sound and retained appearance packets acquire no observer storage.

A completed ordered request separately retains one 72-byte completion
envelope. Its lazy box preserves run/session evidence without adding the
whole envelope to every native ledger. The warmed native-seat test remains
below its existing 8,448-byte heap limit. Default native runs allocate no
advanced packet buffer.
