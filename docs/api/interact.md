# Interact: acting through `Driver` and `Interactions`

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
| `op_loc(driver, x, z, loc_id)` | `OP_LOC1` via `set_menu`/`do_action` |
| `interact(driver, slot)` | `doAction(slot)` on a prepared slot |
| `close_modal(driver)` | `CLOSE_MODAL` via `Out` |
| `answer_count(driver, n)` | `RESUME_P_COUNTDIALOG` via `Out` |
| `login(driver, u, p, reconnect)` | `Client::login` handshake |
| `cheat(driver, cmd)` | `CLIENT_CHEAT` via `Out` |
| `mainland_hop(driver)` | two `cheat`s: tele courtyard + `setvar tutorial 1000` |
| `seed_at(driver, level, x, z)` | skip tutorial + `::tele` to an absolute tile |

`Driver::click_side_tab(tab)` flips the client's local active side icon
(no packet). `Driver::loc_typecode(sx, sz)` reads the packed wall/decor/
scenery/ground-decor typecode at a scene tile.

## `Interactions` (the orchestration layer)

`Interactions<'a>` holds `&'a GameSnapshot` + `&'a mut dyn Driver` and
returns `SendResult` (a `WireCommand`, or a `SendReason` refusal). Its 13
methods: `interact(target, action)`, `use_item_on(item, target)`,
`use_widget_on(widget, target)`, `press(widget)`, `continue_dialog`,
`close_modal`, `answer_count(value)`, `walk(tile)`, `click_side_tab(tab)`,
`login(user, pass)`, `clear_local_modal(component_id)`, `set_run(on)`,
`set_retaliate(on)`. Each does: precondition (attached/ingame/scene/
count-dialog) → target identity (`still_present`) → label/opcode resolve
(`operation_of`/`offers_operation`) → dispatch a `WireCommand` through
`Driver`.

- `OpTarget` is an enum of view refs: `Npc`/`Player`/`Loc`/`GroundItem`/
  `Item`.
- `WireCommand` has 11 kinds: `Op`, `UseItem`, `UseWidget`, `Button`,
  `Continue`, `Close`, `Count`, `Walk`, `SideTab`, `Login`,
  `ClearLocalModal`.
- `SendReason` has 20 refusal reasons (`NotAttached` … `DriverRejected`).
- `ActionResolution::operation_of`/`offers_operation` scan the target's
  actions for a label (trimmed, case-insensitive) or a 1..5 operation
  number. `TargetIdentity::still_present` checks per-kind identity (npc by
  index+id, player by index+name, loc by typecode+layer+tile, ground item
  by id+tile, item by id+slot+component within its container).
- `close_modal`/`clear_local_modal` dispatch via `CLOSE_BUTTON` do_action so
  the client's local modal state is cleared (not just the wire write).

## `LEGAL_SEND` / `ClientProt`

`api::prot::LEGAL_SEND` is the complete outbound table (opcode id + fixed
length, `-1` variable). Typed builders (`api::prot::Send`) construct the
kernel's sends. Key opcodes: `IF_BUTTON` 9, `CLOSE_MODAL` 51,
`RESUME_P_COUNTDIALOG` 102, `OPNPC1..5`/`OPLOC1..5`/`OPOBJ1..5`/
`OPPLAYER1..5`/`INV_BUTTON1..5` per the table in `api::prot`.
