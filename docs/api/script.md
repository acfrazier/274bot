# Scripts (`crates/script`)

Campaign 5 **kernel**. Compiled cards are **rust-first rewrites** on
274bot `api`, not rs2b0t ports. The Load/`@rs2b0t` shim is for
out-of-tree / non-technical authors. WalkTo is **host nav** (panel picker),
not a script card.

## Two runners

| | Compiled (Browse our cards) | Load (operator file) |
| --- | --- | --- |
| When V8 exists | Never | Start of a **JS** picker card only |
| Wake | `host::should_emit_tick` (PLAYER_INFO) / lean `LeanSnapshot.tick` | same, posted to the isolate thread |
| House API | Rust `tick(&mut ScriptCtx)` | `export function tick` **or** `defineBot` |

Idle = no isolate. Stop tears down V8. Pause / not `is_up` keeps the
instance; `want_run` distinguishes operator Pause from offline.

Load registers a picker card tagged **JS** (`~/.274bot/js-scripts.json`
`{name, path}`). Same JS name overwrites. Compiled names (e.g. WalkTo) are
**reserved**. Isolate is its own OS thread; ~50 ms budget; 64 MB heap cap.
A `while(true)` tick is interrupted via `terminate_execution`; Stop join is
bounded so the panel cannot hang forever.

## Nav vs scripts

WalkTo stays the panel **WalkTo** button + traveller. `factory(WalkTo)` is
not a picker id. Lean `here` is `None` until a real player tile is decoded.

**Thread split:** one baked pack + `nav::router::find` at **host** scope
(A* can run on a short-lived worker so a long path does not hitch the 20 ms
pump). **Traveller** is one per uid, ticked on that slot’s pump with the
`Driver` — it goes Idle when Arrived; it does **not** own an OS thread.

## Hard no

No dummy tick-end opcode. No `Arc<World>` on extras. No bot action API in
`vendor/fr-client-rust`. Never Fairy-Ring.
