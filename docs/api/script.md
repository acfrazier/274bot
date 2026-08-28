# Scripts (`crates/script`)

Campaign 5 **kernel**. Compiled cards are **rust-first rewrites** on
274bot `api`, not rs2b0t ports. The Load/`@rs2b0t` shim is for
out-of-tree / non-technical authors. WalkTo is **host nav** (panel picker),
not a script card.

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
