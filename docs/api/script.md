# Scripts (`crates/script`)

Script **kernel** for the bot host. Compiled cards are rust-first on 274bot
`api`. Catalog and external TypeScript/JavaScript bots load through a thin
compatibility shim; **behavior, configuration, and fail-closed helpers are
owned in Rust**. The shim does coercion and marshaling only. The only
JS ↔ Rust isolate wire is **FlatBuffers** — there is no extra JSON host wire
and no foreign JS policy runtime in-tree.

WalkTo is **host nav** (panel picker / TUI map), not a script card.
`Script::on_random` is a rising-edge knock (`RandomClaim::Host` default).
Catalog cards come from an external `$RS2B0T` / `--catalog` checkout
(upstream `rs2b2t/rs2b0t` layout: `src/bot/scripts`), not a copy in this
tree. `$RS2B0T` wins over the persisted root (`~/.274bot/rs2b0t-path`,
written on the first successful catalog parse). Scripts run on whatever
world the bound **server profile** logged into (see [README.md](../../README.md)).

Compatibility is **partial**. Unsupported helpers and options fail
explicitly. Do not treat runner PASS, a single live gold, or a historical
inventory count as “all catalog scripts / all options qualified.”

## Two runners

| | Compiled (Browse our cards) | Load / catalog (operator file or `$RS2B0T`) |
| --- | --- | --- |
| When V8 exists | Never | Start of a **JS/TS** picker card only |
| Wake | `host::should_emit_tick` (PLAYER_INFO) | same, posted to the isolate thread |
| House API | Rust `tick(&mut ScriptCtx)` | `export function tick` **or** `defineBot`; explicit `export const apiVersion = 2` is JS API v2 ([js-api-v2.md](js-api-v2.md)) |

Idle = no isolate. Stop tears down V8. Pause / not `is_up` keeps the
instance; `want_run` distinguishes operator Pause from offline.

An offline slot logs in while a login is wanted (auto-login or a Log in) or a
script is running or paused on it (rs2b0t's `autoLogin || scriptActive()`,
re-evaluated on every title-loop pass), unless the operator logged the slot
out. A dropped connection relogged that way pauses a Load script whole: every
await, step machine and task runtime stays, their clocks and the `Execution`
wait clock stop, and the script resumes on the relogged session's first
tick. The slot then re-sends the script walk it was following, and the last
eight walk, walk-near and abort-walk requests the script queued that never
reached the dropped connection. Other unsent requests are dropped; their
owners retry on their own timeouts. An operator or idle logout, Stop or slot
removal ends the session instead: the in-flight machine rows and task
runtimes end (`aborted`, `reset`). Compiled scripts end their live step at
either boundary.

### File Load and catalog cards

- **Load** registers a picker card tagged **File** from an absolute/relative
  path (`~/.274bot/js-scripts.json` remembers `{name, path}`). Same path
  overwrites; different paths stay distinct even when stems match.
  Compiled names are **reserved**.
- Catalog cards are **Catalog** source: static parse of
  `src/bot/scripts/index.ts` (no V8 at registration). Browse fills both
  panel and TUI pickers.
- Isolate is its own OS thread; a non-yielding execution has a **600 ms
  runaway horizon** measured from that execution's own start; **64 MB V8 heap
  cap** (`heap_limits(0, 64 MiB)`). A confirmed watchdog, Pause-deadline, or
  session-reset cut is logged and recreates the script runtime (still paused
  after an operator Pause). Three cuts within five minutes stop the script
  with an error instead of restarting forever. The isolate starts small and
  grows with the live set — not 64 MB reserved per card. Over the heap cap,
  the isolate is terminated. Extra RSS (OS thread, deno/rustyscript, code
  space) sits on top of JS heap and is **unmeasured** at the 50-slot wall —
  `rss_ladder` is Null/draw-off clients, not Started JS.

### Content-addressed transpile cache

Origin bytes (TS or JS) are hashed with **SHA-256**. Hits under
`~/.274bot/js-cache/objects/<hex>.js` skip transpile; misses transpile `.ts`
(plain `.js` stored verbatim) and update `manifest.json`. Reload that sees
unchanged origin bytes reports nothing changed and does not re-transpile.
Sibling/import dependencies participate in the prepared-card fingerprint
used by manual reload.

### Per-profile assignment and settings

A successful **Start** persists `ProfileSettings.script_assignment` (source,
path/name identity, optional `unavailable` reason) and merges
`script_settings` for that card identity into the vault profile. Focus
restore prefers a pending Browse selection, then the saved assignment.
Parameters **Edit** writes the focused profile’s bag (typed editors honour
`showIf` / `group`; File Load parses `export const SETTINGS` with no V8)
and persists it under that card identity. Values a script reads only at
**Start** take effect on the next Start. Live edits push into a matching
**running or paused** isolate without restart (next relevant tick path) —
there is no separate settings API beyond the panel/TUI editors and the
vault bag.

Missing catalog/file sources stay visible with `unavailable` rather than
silently dropping the assignment.

## Operator controls

Browse / Start / Pause / Stop / Load are wired in both operator panels
(`panel-play` script chrome and `tui-play` script pane) over the same
`host_play::Play` dispatch. The native panel also exposes Reload, Refresh
catalog and the MultiBox bulk script controls described below.

| Control | Behavior |
| --- | --- |
| **Browse…** | Pick a compiled or loaded/catalog card for the focused profile (pending until Start). |
| **Load** | Open a file picker; register/transpile a File card. Disabled while a script is active on the focus. |
| **Reload** | Hash current card origin (+ siblings). Unchanged → “Nothing changed; nothing to reload”. Changed with no running/paused owners → apply. Changed with owners → **Confirm** / **Cancel reload**: running bots that still match the warned generation **restart**; named **paused** bots are **Stopped** (not left half-reloaded). |
| **Start / Pause / Resume / Stop** | Focused profile only. |
| **Start all / Stop all** | MultiBox rail bulk script controls (panel). Separate from **Login all / Logout all**. Start all skips already running/paused/stopping members; Stop all stops running and paused across wall members and live slots. |
| **Refresh catalog** | Re-scan `$RS2B0T` / catalog root. Unchanged scan → “Nothing changed.” Changed with owners → confirm; same restart/stop policy as manual reload. |

Script paint (`ScriptPaint`) draws over the Game chatbox in the panel and
replaces the chat pane in the TUI (`p` toggles back to the game ring).

**Live gold example:** `panel-play --live script_bone_burier` (and its TUI
twin) can start the real `$RS2B0T` BoneBurier card on a unique minted
account when the catalog and engine are present — a supported-path proof,
not catalog-wide qualification. `scenario::ScenarioSettings.start_script`
names the card; the host fills the catalog and dispatches
`script_start_load` at live boot.

External raw-TypeScript smoke for the suite runner is a separate
`external_watch` / `--external-ts` path ([../e2e-suite.md](../e2e-suite.md));
ordinary operator Load is the File/catalog flow above.

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
`vendor/fr-client-rust`. No extra JSON host wire. Never Fairy-Ring.
