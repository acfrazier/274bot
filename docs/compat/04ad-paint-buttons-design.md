# Native one-shot script paint-button seam

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 11:16 UTC. Kind: bounded read-only design for brief 108.
Not implementation, compilation, LIVE, fixtures, ledger, STATE, cache
cleanup, or a foreign canvas/paintState copy. Root owns acceptance, any
LIVE snapshot, serialized implementation insertion, and implementation
handoff. No routine reviewer card. Do not spawn implementation cards
from this run.

Read once: `AGENTS.md`, `docs/execution.md`, brief 108. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `04d7b1f48180ab9b93bbd6fec45a655df4385233`. Client gitlink
`9d090ed04957e4efc254f073cda97bc5510ca72b`. Concurrent cook / combat /
tick / tools workers must not be touched. Work was read-only except this
report and `docs/compat/evidence/paint-buttons-design/`.

## Verdict

**Root912 paired Air FAIL is the missing `Paint.buttons` member, not a
broken imported NatureCrafter.** Native `Paint.begin` records
title/row/gap/text/bar/cells/end and throws `not impl: Paint.buttons`
via the unused-widget Proxy. NatureCrafter Runner `onPaint` calls
`p.buttons([{id:'gobank', label: goBank ? 'Resume' : 'Go bank'}])` and
toggles script-local `goBank` only if the returned id is `gobank`.
Master does not call `buttons` on the native path: host-owned
`ScriptRunner.paintControls` is a hint row, not foreign Pause/Stop
overlay buttons.

Map the foreign immediate-mode click onto a **host-owned descriptor plus
one-shot isolate command**. Record `{id,label}` on `ScriptPaint`.
Headless returns `null` and still forwards descriptors. Panel overlay
and TUI paint pane must expose **real** controls for advertised ids.
An actual click may reach only the current slot actor, current
`work_generation`, and an id advertised by this paint's `buttons()`
call, consumed once. Do not fabricate a click. Do not treat a label as
gameplay. Existing host Pause/Stop stay host-owned.

| | Hypothesis | Status |
|---|---|---|
| (1) | Imported Runner is broken; dim the card | **Rejected.** Frozen `onPaint` is the documented caller. Shim Proxy throws. Missing host support. |
| (2) | Append a `Go bank` paint line and return `null` | **Rejected.** Brief: rendering an unavailable label and returning null does not qualify. Never fabricate a click. |
| (3) | Port foreign `paintState` / canvas regions / `pointerDown` | **Rejected.** No foreign canvas UI, controller, or input-policy copy. Native `paintLogic.js` is fmt helpers only. |
| (4) | Reuse host Pause/Stop or `WireCmd` | **Rejected.** `goBank` is script-local park (`GoBankPark`), not isolate pause. `WireCmd` is game continue/answer/walk. A label must not imply a packet. |
| (5) | Reuse existing `tabs`/`select` stubs (return current, drop at `end()`) | **Rejected.** Those are display-only and not the throwing member. Do not expand them here. |
| (6) | Host descriptor on `ScriptPaint` + `IsolateCmd::PaintClick` + real frontend widgets; headless no-click returns `null` | **Accepted mapping.** |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `04d7b1f48180ab9b93bbd6fec45a655df4385233` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 108 SHA-256 | `6e62dc913836c7e00682a56017d552d4e259bbc58d741e960f80daec5a8ef915` |
| Kanban card | `t_edbe327d` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| Paint.ts (both catalogs; sha256 equal) | `cfef244e3627db88ff20abb501bcfb4f86e973227e769d0c931e65594dc3d7a6` |
| NatureCrafter.ts (both; sha256 equal) | `025ac395b25d64ef818cc0321478f0a2c84a051b79f99decbbfec5a9a2f0812a` |
| ScriptRunner.ts (both; sha256 equal) | `f3bb6124bfade9fe976a7ae8f288aae7638e02d79a265d002882d521d555d37b` |
| AIOQuester.ts (both; sha256 equal; dimmed) | `ec839021a129d8efca7f9ad4a15ecbd74586d28b5a040bdaacc983b7a6454e27` |
| paintLogic.ts (old catalog; consumeClick) | `a719037d59345b6daf6b299a1e8fc0ceafa347e0ec273651779060cda97e5585` |
| Native `paint.js` | `25956ff0d3f675261eda453f40db32db4e23e559ec07b015691ede4c71401653` |
| Native `script_runner.js` | `b9038704ae62a0c6c1278a797503a13e7d09c608a33d1a360322d3dbf8f7d95f` |
| Native `shim/mod.rs` | `3abe3a49c807ea14d78ee3b949aa3c02d652ff8b5313684ec1a345014e8e44b5` |
| Native `load.rs` | `9a15d4b5591ad59ee8d66eb33d0c185411ec7ef583104edbacbd9bd297e552fa` |
| Native `slot.rs` | `1593826ffe821e7a3cd0d4706b7778bbfcf15db8bc8560563d81222700d0d2c4` |
| Native `isolate.fbs` | `f7d689ace573a1d4eb343bb831b9a14711e6529bfe0d0ed608a126ff17ed6548` |
| Native `isolate_fb.rs` | `e2afb5d2e9887ebdd3d3c00890e7d539df9ed9e551d8e848b38559c0c7720fef` |
| Native `panel/src/paint.rs` | `aa1fc7960e804e1cc9c5f252ecb974e4ccb64e6997d40e960a026e090e15b9cf` |
| Native `tui/src/chat.rs` | `364161f710a791798419da6cce5f0054c5e1fe86ae17ce7c8276b1a221ffd739` |
| Native `host-play/src/lib.rs` | `dd4e1b91ddb1cc02b5b2d43899b6a02d86c183ed069c98391701c4627d16fdd3` |
| r289 Air Root912 log | `d8646933d4e6165fee74b7baee3047daf8558106a3ea034158819cd05991ce1a` |

Machine-readable copies: `evidence/paint-buttons-design/{refs,hypotheses}.json`.

## 1. Actual FAIL

Root912 old-catalog paired Air, revision 289,
`evidence/paired-catalog-fixtures/live/r289-air-100adccc-91289e9b.*`:

- Preparation reached `both-started` (shared Start after Falador East
  bank ack + ruins seed). Gold clock began.
- Master `live7i68fk_0` logged `NatureCrafter master starting` (onStart).
- Runner `live7i68fk_1` tick 27: `not impl: Paint.buttons`, then the
  onStart log, then a Trade interact.
- Harness `FAIL: paired_catalog_air_live: script error on live7i68fk_1`.

Mechanism, existing:

1. COMPAT_RUNNER paints each isolate tick (`onPaint` after kick; hold is
   paint-only).
2. Runner `onPaint` reaches `p.buttons([...])`.
3. Shim `Paint.begin` Proxy: missing `buttons` → `notImpl('Paint.buttons')`.
4. `.__rs2b0t_tick_async.catch` stores `host.lastError`.
5. `tick_loop` logs `tick {n}: {e}`.
6. `SlotScript::drain_logs` treats a `tick ` prefix as `last_error`.
7. Paired harness FAILs on `script_last_error`.

Master does not throw: native `ScriptRunner.paintControls` appends
`Pause/Resume: host · Stop: ScriptRunner.stop()` and never calls
`p.buttons`. Foreign `ScriptRunner.paintControls` *does* call
`p.buttons([{id:'pause'},{id:'stop'}])`; that is not the native seam.

Catalog sources are frozen and byte-identical across 100adc / 8e7d for
`Paint.ts`, `NatureCrafter.ts`, and `ScriptRunner.ts`.

## 2. Foreign contract (do not port the runtime)

Frozen `PaintFrame.buttons(items: {id, label}[]): string | null`
(both catalogs, `Paint.ts` 381–399):

- Collapsed or empty `items` → `null` (no consume).
- Else draw each item, register region `btn:${id}`,
  `paintState.consumeClick('btn:' + id)` (Set.delete, one-shot).
- Return the clicked **id**, or `null`. Last matching item in the array
  wins if several clicks were pending.
- Consume is by id, never by label. NatureCrafter toggles `goBank` only
  on `clicked === 'gobank'` while the label reads `Go bank` or `Resume`.

`paintState.pointerDown` hits a published region; widget hits add the
region id to a Set. `end()` publishes regions. Clicks land on a later
paint. This is canvas immediate-mode. Do not copy it.

Enabled caller:

- **NatureCrafter Runner** (catalog card): one button `id: 'gobank'`.
  `GoBankPark.validate` is `goBankActive() && !Trade.active()`. Walking
  to `runnerBank` is the script's task after the flag flips. Headless
  paired Air does not need the click to ferry essence.

Not enabled / not this mapping:

- Foreign `ScriptRunner.paintControls` Pause/Stop overlay. Native already
  host-owns Pause/Stop. Keep the hint row.
- **AIOQuester** (`pause` / `skip` / `stop` overlay) is dimmed
  (`block_reason: dim: AIOQuester`). Do not build skip-quest or replace
  host Stop. The generic `{id,label}` descriptor still fits if that card
  is later undimmed; pause/stop must remain host-owned.

`stepper` / `list` / `fill` / `wrap` / `cols` stay `not impl`. Existing
`tabs`/`select` stubs (return first/current, dropped at `end()`) stay as
they are. Do not treat them as button support.

## 3. Actual native paint / input ownership

```
onPaint (normal Tick, parked pump, or hold paint-only)
  Paint.begin → frame methods append ScriptPaint
  end() writes __rs2b0t_host.paint {title, accent, lines}
  tick_loop evals host.paint, serde → ScriptPaint
  forward_paint_if_changed → ThreadMsg::Paint FlatBuffer
  SlotScript::paint / Play status.script_paint
  panel PaintOverlay (chatbox rect, view-local collapse)
  TUI chat pane (paint-as-chat; `p` toggles game chat)
```

Pause: `want_run=false`, `IsolateCmd::Pause`, Tick cmds skipped, no
onPaint. Resume re-arms. Stop: join isolate, drop instance, bump
`work_epoch`, Idle; paint dies with the isolate.

Hold: Tick still delivered; `host_hold` skips loop/pump; onPaint still
runs. Pending script-button input may be consumed on that paint.

`WireCmd::{Continue,Answer,Walk}` is game input on the slot Driver.
`Play::script_pause` / `script_stop` are operator lifecycle. Neither is
a script-local paint id.

`ScriptPaint` is `{title, accent, lines}` only (`isolate.fbs` table
`Paint`). Overlay and TUI render title+lines. Collapse click is
view-local (`paint:collapsed` analog) and must not become a script
event.

No deep-copy of world state is required. Descriptors are a small vec on
the already-forwarded frame.

## 4. Minimum mapping

Callback/input glue only. Gameplay stays in the imported script.
Scheduling/transport stays Rust.

### Fields

`ScriptPaint` gains:

```
struct ScriptPaintButton { id: String, label: String }
struct ScriptPaint {
    title: Option<String>,
    accent: Option<String>,
    lines: Vec<String>,
    #[serde(default)]
    buttons: Vec<ScriptPaintButton>,
}
```

- `id` / `label` are the only advertised fields. No color, rect, dock,
  or implied action.
- `#[serde(default)]` so current `host.paint` objects without `buttons`
  still decode.
- Empty `buttons` is the no-control case; PartialEq treats it as quiet
  when unchanged so `forward_paint_if_changed` still skips identical
  frames. Label toggle (`Go bank` ↔ `Resume`) is a real change and must
  forward.

FlatBuffer: add optional `[PaintButton]` at the end of table `Paint`
(`id: string`, `label: string`). New `VT_PAINT_BUTTONS = 10`. Older
buffers without the field decode as empty. Do not version-bump Snapshot.

Host handle one-shot slot (not Snapshot, not Interact):

```
__rs2b0t_host.paintClick = string | null
```

### `crates/script/src/shim/paint.js`

Add `frame.buttons(items)`:

1. Normalize to `{id: String, label: String}[]`. Empty → return `null`,
   do not clear previously recorded buttons from this begin/end.
2. **Append** onto `rec.buttons` (two `buttons()` calls in one paint
   must both advertise; foreign draws both rows).
3. If `host().paintClick` is a string and equals one of **this call's**
   ids: clear `paintClick`, return that id. Else return `null`.
4. Never return an id that this call did not advertise. Never invent
   `gobank`. Never look at the label.

`end()` posts `buttons: rec.buttons` next to title/accent/lines.
Do not also stuff labels into `lines` (that would be an unavailable
text control).

### Isolate command (new; no existing click channel)

`IsolateCmd::PaintClick { id: String, generation: u64 }` processed on
the isolate thread like Pause/Settings:

- generation mismatch or `paused` → drop, do not set `paintClick`.
- else `host.paintClick = id` (replace unused pending; one slot).
- After each paint eval, if `paintClick` is still set, clear it
  (unadvertised / `buttons()` not called this paint).

`LoadIsolate::paint_click(id)` sends the cmd with current
`work_generation`. `Play::script_paint_click(name, id)`:

- no-op when no slot, state is not `Running`, id is empty, or the last
  forwarded `ScriptPaint.buttons` does not contain `id` (stale overlay).
- never walks, never pauses, never stops.

Do **not** add this to `WireCmd`. Do **not** queue it as `InteractReq`.

### Lifecycle

| Edge | Pending click | `buttons()` return | paint descriptors |
|---|---|---|---|
| Normal / parked Tick | consume if advertised | id or `null` | forwarded |
| Hold (paint-only) | consume if advertised | id or `null` | forwarded |
| Pause / `!want_run` | **drop** on Pause cmd; no onPaint | — | last frame kept |
| Resume | does **not** replay | later paints only | existing |
| Stop / join / reload | isolate dies | — | paint cleared |
| generation skip / ResetSession | drop; clear `paintClick` | — | — |
| Headless / no frontend click | never set | always `null` | still recorded |
| Other slot / other actor | Play is per `name` | never delivered | — |
| Overlay collapsed (view-local) | widgets hidden; no click | script still records | forwarded |

Pause must not start painting just to deliver a button. Hold already
paints; consuming there matches current paint-only ticks and does not
weaken the hold (loop/pump stay frozen). `goBank` is a JS flag; it does
not run `GoBankPark` until loop resumes.

### Frontends (required for “supported”)

Headless returning `null` is the correct default. It is **not** complete
support by itself.

**Panel** (`crates/panel/src/paint.rs`): inside the existing chatbox
overlay, when not view-collapsed, draw a real ImGui button per
`ScriptPaint.buttons` entry using the advertised **label**. Click →
`session` → `Play::script_paint_click(focused, id)`. Do not place these
on the script chrome Browse/Start/Pause/Stop row. Collapse remains
view-local and hides the widgets.

**TUI** (`crates/tui/src/chat.rs` + `bin.rs` wire): when paint-showing
and `buttons` is non-empty, render reachable controls (button row under
the paint lines; digit `1..=n` or j/k+Enter on a paint-local focus,
modal still wins). New `ChatAction::PaintButton(usize)` (index into the
advertised vec; keep Copy). `tui-play` maps index → id and calls
`Play::script_paint_click`. Space/Enter must **not** become
`WireCmd::Answer` / `Continue` while paint-showing without a modal.
Host Pause/Stop keys stay `script_toggle_pause` / `script_stop`.

### Owned implementation files (later card, not this one)

1. `crates/script/src/shim/paint.js`
2. `crates/script/src/shim/mod.rs` (`ScriptPaint` + button struct)
3. `crates/script/schema/isolate.fbs` + `crates/script/src/isolate_fb.rs`
   (Paint table only)
4. `crates/script/src/load.rs` (`PaintClick` cmd, pause/reset clear,
   leftover clear after paint eval)
5. `crates/script/src/slot.rs` (thin `paint_click` forward)
6. `crates/host-play/src/lib.rs` (`Play::script_paint_click` only)
7. `crates/panel/src/paint.rs` (+ session one-liner to Play)
8. `crates/tui/src/chat.rs` and the existing ChatAction match in
   `tui/src/app.rs` + `tui/src/bin.rs`

`load.rs`, `isolate_fb.rs`, and `host-play/src/lib.rs` are hotspots.
Touch only the paint-click / Paint-table sites. Do not edit
`client_adapter.js`, `script_runner.js` `paintControls`, NatureCrafter
sources, paired harness, or Pause/Stop chrome.

New focused file `crates/script/tests/paint_buttons.rs` (do not pile
`load_isolate.rs`). Preserve
`isolate_paint_begin_records_script_paint` /
`isolate_paint_accessor_returns_the_recorded_frame` (empty `buttons`
default). Update the “widget methods throw `not impl`” comment only as
needed so `buttons` is no longer claimed missing.

Meaningful behavior checks:

1. `p.buttons([{id:'gobank',label:'Go bank'}])` does not set
   `last_error` / `tick N: not impl`.
2. No pending click → return `null`; forwarded `ScriptPaint.buttons`
   equals the advertised list.
3. `paint_click('gobank')` then one Tick: first `buttons()` returns
   `'gobank'` once; second paint returns `null`.
4. `paint_click('nope')` or id not in this call’s items: `null`; pending
   cleared after paint.
5. Pause then `paint_click` then ticks: no consume; resume does not
   replay. Hold tick: consume still allowed; loop stays frozen.
6. Two isolates: click on A never returns from B’s `buttons()`.
7. generation skip / Stop / ResetSession: pending dropped.
8. FB round-trip of title/lines/buttons; truncated buffer still errs.
9. Panel overlay: advertised label is a real button; click dispatches
   focused-slot id; chrome Pause/Stop unchanged; collapsed hides
   widgets.
10. TUI: paint-showing digit/Enter dispatches `PaintButton`; modal still
    wins; no `WireCmd` for the script id.

Host-play: `script_paint_click` no-op when Idle/Paused/missing id;
Running + advertised id sends the isolate cmd.

No compiler / LIVE on this card. Source review of a later patch is not
paired Air LIVE acceptance. Headless Air can proceed once `buttons()`
stops throwing (null return); operator park (`GoBankPark`) still needs
the frontend click path before “button support” is claimed.

## 5. Unsupported flavors (honest)

Leave `not impl` (Proxy): `stepper`, `list`, `fill`, `grid`, `wrap`,
`cols`, canvas hit-testing, hover, wheel, foreign `paint:collapsed`
script state (panel collapse is view-local only).

Do not implement AIOQuester skip. Do not route Pause/Stop through
`Paint.buttons`. Do not auto-click `gobank` in catalog/LIVE harnesses.
Do not walk to a bank because a label said so.

## 6. Falsifiers

A later implementation is wrong if any of these hold:

- `Paint.buttons` still throws, or returns a fabricated id with no user
  event.
- Headless / no click returns `'gobank'`.
- Descriptors are only stuffed into `lines` with no real panel/TUI
  control.
- Click delivered after Pause, after Stop, on a stale generation, to
  the other paired actor, or for an id not advertised this `buttons()`
  call.
- Label `Go bank` / `Resume` is interpreted as a walk, pause, or
  `WireCmd`.
- Host Pause/Stop chrome is removed, duplicated as script buttons, or
  driven by `paintControls` calling `p.buttons`.
- Foreign `paintState`, canvas regions, or a second UI runtime is
  copied.
- Hold stops painting, or Pause starts painting, to make clicks work.
- `client_adapter.js`, catalog NatureCrafter, or paired-test sources
  edited here.
- World snapshot is deep-copied for this seam.

Root inserts implementation after this design. Not LIVE acceptance.
