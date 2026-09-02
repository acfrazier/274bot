# Scripts (`crates/script`)

Alpha **kernel**. Compiled cards are meant to be **rust-first rewrites**
on 274bot `api`, not rs2b0t file-ports. This tag does **not** ship honest
skilling/farming bots. The Load/`@rs2b0t` shim is for out-of-tree /
non-technical authors. WalkTo is **host nav** (panel picker / TUI map), not a
script card. `Script::on_random` is a rising-edge knock (`RandomClaim::Host`
default); `Handle` is unnamed in-tree. The 0.1.5 shim loads listed scripts
from `$RS2B0T/src/bot/scripts` (upstream `rs2b2t/rs2b0t`), not a copy in
this tree. `$RS2B0T` wins over the persisted root
(`~/.274bot/rs2b0t-path`, written on the first successful catalog parse).
Scripts run on whatever world the client logs into — **local engine by
default**; `BOT_TARGET=prod|live` or `host-play --prod` switches the login
host to `w1.rs2b2t.com:43594` with the baked public RSA (Cargo `TARGET` is
the rustc triple, not a world switch; not Jagex, not a hosted wall, no w1 CI).

## Two runners

| | Compiled (Browse our cards) | Load (operator file) |
| --- | --- | --- |
| When V8 exists | Never | Start of a **JS** picker card only |
| Wake | `host::should_emit_tick` (PLAYER_INFO) | same, posted to the isolate thread |
| House API | Rust `tick(&mut ScriptCtx)` | `export function tick` **or** `defineBot` |

Idle = no isolate. Stop tears down V8. Pause / not `is_up` keeps the
instance; `want_run` distinguishes operator Pause from offline.

Load registers a picker card tagged **JS** (`~/.274bot/js-scripts.json`
`{name, path}`). Same JS name overwrites. Compiled names are **reserved**.
Isolate is its own OS thread; ~50 ms budget; 64 MB heap cap. A `while(true)`
tick is interrupted via `terminate_execution`; Stop join is bounded.

Browse/Start/Pause/Stop are wired in **both** operator panels: the native
`panel-play` script chrome and the headless `tui-play` script pane (the
same `host_play::Play` dispatch; `$RS2B0T` catalog cards fill both
pickers). Script paint (`ScriptPaint`, the rs2b0t dock shape) draws over
the Game chatbox in the panel and replaces the chat pane in the TUI
(`p` toggles back to the game ring).

**Live gold:** `panel-play --live script_bone_burier` (and its TUI twin)
starts the **real `$RS2B0T` BoneBurier card** on a unique minted account
and PASSes on the server's "You bury the bones." chat line — the shim's
`Inventory.first`/held-item `interact` and the `reader.inventorySize()`
gate are exercised against the live engine. `scenario::ScenarioSettings
.start_script` names the card; the host fills the catalog (register/Load)
and dispatches `script_start_load` at live boot.

## ScriptCtx read surface

```rust
pub struct ScriptCtx<'a> {
    pub driver: &'a mut dyn Driver,
    pub tick: u64,
    pub here: Option<(i32, i32, i32)>,           // local player world tile
    pub walk: Option<&'a mut dyn FnMut(i32, i32, i32) -> bool>, // arm find + follow
    pub inv: Option<&'a [(i32, i32)]>,            // (real obj id, count)
    pub obj_names: Option<&'a ObjNames>,          // id -> name table
}
impl ScriptCtx<'_> { pub fn has_item(&self, name: &str) -> bool; }
```

`has_item` resolves real obj ids case-insensitively. Inventory ids are real
(`stored - 1`), matching `ItemDefView.id` and `ObjNames`.

## Nav vs scripts

WalkTo stays the panel **WalkTo** button + traveller. `ctx.walk(x, z, level)`
arms `nav::router::find` on the shared whole-world `NavWorld` and the slot
pump drives `nav::traveller::Traveller::follow`; `SlotStatus.walk_{x,z,level}`
mirrors the armed dest and clears on arrival. The nav `find` runs off-pump
(a short-lived worker); `follow` steps on the slot pump, one send per tick.

## Hard no

No dummy tick-end opcode. No `Arc<World>` on extras. No bot action API in
`vendor/fr-client-rust`. Never Fairy-Ring.
