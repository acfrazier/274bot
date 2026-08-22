# Interact: acting through the `doAction` path

All actions go through `api::interact`, which writes to the send-side
through the `Driver` trait. `Client` implements it over
`doAction`/`tryMove`/`out`; tests use a recording stub. A `true` return means
the driver **accepted the send**, not that the server applied it — confirm
effect via snapshot + settle evidence.

## The `doAction` path

The kernel never injects a raw opcode that skips ISAAC. Menu actions are
prepared in the client's menu buffers and dispatched by `Client::doAction`,
which runs the client-code arms (walk-to-tile, anticheat preambles, logout
vetoes) and then writes the opcode + payload through the ISAAC sink.

| Helper | Mechanism | Effect |
| --- | --- | --- |
| `press(driver, iface_id)` | menu slot 0 = `IF_BUTTON` + `doAction(0)` | press interface child |
| `set_run(driver, on)` | `press` 153 if `on`, 152 if off | set run on/off |
| `walk(driver, x, z)` | `tryMove` from the local route, type 0 | plain ground walk |
| `interact(driver, slot)` | `doAction(slot)` on a prepared slot | dispatch prepared menu option |
| `close_modal(driver)` | `CLOSE_MODAL` via `Out` | close open modal |
| `answer_count(driver, amount)` | `RESUME_P_COUNTDIALOG` via `Out` | answer count dialog |
| `login(driver, user, pass, reconnect)` | driver handshake (`Client::login`) | queue a login |
| `cheat(driver, cmd)` | `CLIENT_CHEAT` via `Out` | `::` command without the prefix |
| `mainland_hop(driver)` | two `cheat`s | tele Lumbridge courtyard + `setvar tutorial 1000` |

`set_run(true)` presses **153** (run on); `set_run(false)` presses **152**
(run off). Auto-run only sends 153 when run is off. Run state is
server-echoed, so the caller decides from snapshot state whether to send.

## `LEGAL_SEND` / `ClientProt` table

`api::prot::LEGAL_SEND` is the complete outbound table: one row per
`ClientProt` constant from `client/src/io/client_prot.rs` (opcode id +
fixed length, `-1` = variable). The kernel only ever emits a packet whose
row is in this table, through the `Out` sink (`p1_enc` ISAAC-encrypted
opcode, then plaintext payload).

Typed builders (`api::prot::Send`) are the constructors the kernel uses:
`Send::if_button(child)`, `Send::close_modal()`, `Send::count_dialog(n)`,
each writing via `Out`. Anticheat/event rows are present in the table but
unused by the kernel.

Key opcode ids agents touch directly:

| ClientProt | id | len |
| --- | --- | --- |
| `IF_BUTTON` | 9 | 2 |
| `CLOSE_MODAL` | 51 | 0 |
| `RESUME_P_COUNTDIALOG` | 102 | 4 |
| `MOVE_GAMECLICK` | 207 | -1 |
| `OPNPC1`…`OPNPC5` | 236 / 233 / 223 / 147 / 189 | 2 |
| `OPLOC1`…`OPLOC5` | 215 / 103 / 187 / 157 / 127 | 6 |
| `OPOBJ1`…`OPOBJ5` | 247 / 169 / 108 / 62 / 117 | 6 |
| `OPPLAYER1`…`OPPLAYER5` | 109 / 166 / 196 / 98 / 174 | 2 |
| `INV_BUTTON1`…`INV_BUTTON5` | 74 / 82 / 239 / 179 / 46 | 6 |
| `MESSAGE_PUBLIC` / `MESSAGE_PRIVATE` | 253 / 139 | -1 |

## Menu actions

`Client::doAction` dispatches on `MiniMenuAction` values (`client/src/client/
mini_menu_action.rs`): `OP_*1..5` for npc/loc/obj/player/held, `IF_BUTTON`
(231), `WALK` (718), plus examine/use-item/target arms. `press` uses
`MiniMenuAction::IF_BUTTON`; the kernel maps an action to a slot via
`Driver::set_menu` before `doAction`.
