# Scripts (`crates/script`)

Campaign 5 **kernel**. Panel Browse/Start/Pause/Stop drive a per-uid
`SlotScript` on `Play`. Smoke **ports** (BoneBurier, live `--live script_*`)
are a follow-on plan.

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

## WalkTo

The port exists under `crates/script/src/ported/walk_to.rs` for tests.
`factory(WalkTo)` is **`None`** until `ScriptCtx.walk` is wired to the
existing traveller — Start reports `not ported` (no lying Start). Lean
`here` is `None` until a real player tile is decoded (build origin is not
the player).

## Hard no

No dummy tick-end opcode. No `Arc<World>` on extras. No bot action API in
`vendor/fr-client-rust`. Never Fairy-Ring.
