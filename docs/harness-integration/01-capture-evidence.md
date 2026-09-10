# Capture and evidence — application integration

Architect: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: architecture only for brief 46 section A.
Not implementation, not LIVE, not catalog/frontend acceptance, not a
second capture backend, not a restart of the memory campaign.

Read once: `AGENTS.md`, `docs/execution.md`, `docs/harness.md`,
`docs/compat/STATE.md`, `docs/compat/briefs/46-harness-architecture.md`,
and the named current seams. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work is this report only.
No product edits, LIVE, fixtures, STATE, matrix, other workers' files,
stash, reset, restore, checkout, merge, remotes, gitlink, or release
actions.

## Verdict

Keep the existing whole-window GPU readback. Do not add OS-level
`CopyFromScreen`, a second wgpu path, a CPU framebuffer dump, or a
headless PNG synthesizer.

What is useful now is to treat that one capture path as an evidence
camera with honest metadata, and to show the result inside panel/TUI
instead of only on stdout and in operator Python receipts.

Proceed with four bounded application increments, then stop:

1. A capture envelope that names what the PNG actually is, and refuses
   to pretend an empty snapshot is game state.
2. Use the catalog `terminal_shot` labels that landed in `6c6bb2d5`
   (BoneBurier/ChickenKiller/Alcher/BankFletcher/Thiever). Do not re-arm
   them. Remaining work is honest sidecars, drain/error visibility, and
   Results — not more labels.
3. A run manifest (source/profile provenance) written beside the PNGs,
   plus PNG/sidecar hashes in the same dir.
4. A readable Results surface: panel thumbnail + compact evidence; TUI
   text evidence only (TUI has no GPU shot).

Python launchers stay launchers. They already record command, PID,
commits, binary hash, log and timeline hashes. They should hash shot
files after the process exits, not become a second product.

Design approval is not source acceptance and not native proof.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Inspection base (shot/panel/diagnostics read here) | `c9d01a29fbe2b8f8dcf0a1750b226b503e9bcdff` |
| HEAD at report write | `bba5179c` (game-data provenance; not capture) |
| Catalog terminal-shot arms | `6c6bb2d5` ("capture catalog milestones") |
| Named client gitlink | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief SHA-256 | `4a5a7df764e4c25fc6f1eeafeaa1ce316df2422a5f69397a9714ab60ce7192a9` |
| `crates/scenario/src/shot.rs` last commit | `217f150f` (selected memory/harness publication) |
| `crates/scenario/src/evidence.rs` last commit | `f2d45ecc` (FAIL chat dumps) |
| `crates/scenario/src/runner.rs` last commit | `217f150f` |
| Panel `window.rs` ShotState/readback | `7aaa8c39` |
| Panel `app.rs` F12/`pump_shots` | `814e5293` (startup-prep commit; shot path unchanged in intent) |
| Host-play `memory_diagnostics.rs` | `217f150f` |
| Historical ref (read-only) | `codex/memory-diagnostics` = `f9975102` |
| Kanban card | `t_65dd7c69` |

While this report was being read against `c9d01a29`, `6c6bb2d5` committed
the catalog `terminal_shot` arms that were previously dirty in
`crates/scenario/src/lib.rs` (BoneBurier, ChickenKiller, Alcher variants,
BankFletcher) plus `catalog_boundary_live` bank-fletcher loop changes.
Those labels are now product. Untracked headed receipts under
`docs/compat/evidence/` remain root's; this report did not use them.
Nav scenarios and `render_smoke`'s `StepKind::Shot { label: "scene2" }`
were already on `c9d01a29`. `6c6bb2d5` did not add extra `StepKind::Shot`
milestones — only terminal labels.

`docs/memory/architecture-synthesis.md` on `codex/memory-diagnostics` is
an observation/transport/presentation execution decision, not a capture
design. Capture evidence from that campaign is the selected reports below,
not a requirement to revive owner-census, heaptrack, or managed collectors.

## Current capture path (authoritative)

One camera. Four threads/roles, already wired.

```
slot thread          UI frame                 render pass              UI frame
ScenarioRunner  -->  ShotState.requests  -->  ShotState.wanted   -->  ShotState.done
  shot_sink            pump_shots               copy_texture_to_buffer    write_shot
  (label + pretty      (optional smoke          map_async + poll wait     PNG + sidecar
   GameSnapshot JSON)   settle hold)            to_rgba normalize         under shot_dir
```

- Files: `crates/scenario/src/shot.rs` names `~/.274bot/smoke/<stamp>_<pid>/`
  or `$274BOT_SMOKE_DIR`. PNG + JSON sidecar, 377 stamp/safe-label.
- Coordination: `panel::window::ShotState` `{requests, wanted, done}`.
- GPU: `AppWindow::render` copies the just-drawn imgui target (surface or
  offscreen when the surface is not `COPY_SRC`). That is the **whole
  window**, chrome included, not the game FBO alone.
- Write: `pump_shots` on the UI body. File I/O is not on the GPU thread.
- Map failure: `map_readbacks` drops the shot (`None`) instead of
  panicking. Smoke then FAILs at 300 s if no PNG lands. Live script
  drain waits up to 10 s (`NAV_FULL_SHOT_DRAIN`) then exits anyway.
- Interactive F12: `enqueue_manual_shot` pushes
  `(manual-<stamp>, GameSnapshot::new() pretty JSON)`. Empty snapshot.
  Disabled while any `--live`/`--smoke` harness is running so a manual
  write cannot trip smoke's "any written shot passes".
- Headless e2e and `catalog_boundary_live` install a no-op `shot_sink`.
  That is honest: there is no window to read back.
- TUI `--live` prints `PASS`/`FAIL` plus `Evidence::to_json()` and
  exits. No screenshot, no drain. Comment in `tui/src/main.rs`: proof
  is the runner, not a screenshot.

`--smoke` is the one `StepKind::Shot` user: `render_smoke` fires
`scene2` the tick seed reaches `ingame && scene_state == 2`. The panel
holds the request for `SMOKE_SETTLE` (3 s) because the slot renderer
can be 1 fps on CPU, and an immediate readback is the title screen.
The process exits 0 only after `pump_shots` writes the PNG.

Terminal shots fire on PASS **and** FAIL from `finish_pass` /
`finish_fail`, using the runner snapshot **before** it is released
(`terminal_snapshot_is_released_after_shot_and_evidence`). The PNG
sidecar can therefore be the real terminal state. Catalog labels exist
as of `6c6bb2d5`; the headed PNG/envelope still has to use them.

Rendering honesty that already exists and must stay:

- Client GPU `scene_state == 1` keeps the last 3D texture and draws the
  loading splash over it (`gpu.rs` `scene_ready` / `freeze_last_scene`).
  A whole-window capture during rebuild can look like the previous
  courtyard plus "Loading - please wait." The sidecar must say
  `scene_state: 1` when that is what ran.
- Skip-paint / unfocused 1 fps slots are not a second camera. Capturing
  the window still shows whatever the focused Game pane last presented.
- `BOT_CPU=1` is CpuPix3D; the window readback still works because it
  copies the imgui/present target, not the client's 3D texture.

## Capability inventory

### Built-in (ship, reuse)

| Capability | Where | Notes |
|---|---|---|
| Whole-window GPU capture | `panel/window.rs` readback | One backend. Keep. |
| PNG + JSON sidecar naming | `scenario/shot.rs` | Shared headed/headless naming. |
| `StepKind::Shot` milestone | `scenario` + `render_smoke` | Arm holds → sink. Headless no-op. |
| Terminal shot on PASS/FAIL | `ScenarioRunner::fire_terminal_shot` | Only if `settings.terminal_shot` set. |
| 10 s FAIL/PASS drain | `panel` `hold_terminal_shot` | Headed only. |
| Compact `Evidence` JSON | `scenario/evidence.rs` | Predicate, ticks, tile, inv, chat on FAIL, scene. |
| Stdout PASS/FAIL lines | panel `live_script_tick`, TUI `live_status` | Operator and Python timeline parse these. |
| `--smoke` settle + PNG gate | panel `LiveSmoke` | Exit 0 is the write, not runner PASS alone. |
| F12 on-demand enqueue | panel, interactive only | Empty sidecar today. |
| Opt-in memory diagnostics | `host-play/memory_diagnostics.rs` | 1 Hz, 32×512 log/request ring. `recent_navigation` is JSON `null` on purpose. |
| Memory samples | `samples.jsonl` + qualification + optional diagnostics | Memory campaign leftover; useful bounded sidecar, not catalog proof. |
| Headless catalog observations | `catalog_boundary_live` (committed) | Snapshot deltas; no-op shots. |
| Last-FBO freeze | client GPU backend | Preserve; annotate captures. |

### External (helpers / historical; do not swallow)

| Capability | Where | Keep outside? |
|---|---|---|
| Headed launch + stdout timeline JSONL + process receipt | `.superpowers/catalog-headed/run_cell.py` | Yes as launcher. Hash shots after exit. |
| Headless launch + log receipt | `run_headless_cell.py` | Yes. No PNG expected. |
| Frozen binary identity JSON | `docs/compat/evidence/catalog-headed/binary-*.json` | Operator/build, not runtime. |
| Windows `CopyFromScreen` of `catalog-watch` HWND | `windows-capture-owned.ps1` | Stay operator-only. Second backend. |
| Failure-only qualification row | historical `BOT_MEMORY_FAILURE_CAPTURE` on `codex/memory-diagnostics` | Idea yes; env/harness no. Scenario `Evidence` already is the failure row. |
| Panel `capture_tx` Game-pane arm | historical panel-memory-capture-arm | Input-capture lifecycle, not screenshots. Do not revive as a shot path. |
| Process RSS/CPU collector schema-2 | historical `process_evidence.py` | Memory measurement. Not catalog evidence. |
| Heaptrack / direct-owner census / nav capture journals | historical reports | Excluded experiments. No current user benefit for compatibility. |

### Missing (useful, not present)

- Honest on-demand sidecar (focused snapshot + scene/ingame/tick, or an
  explicit `snapshot_absent` flag).
- Run manifest next to shots: host/client commit, profile, revision,
  binary identity if known, `274BOT_SMOKE_DIR`, start time.
- PNG/sidecar SHA-256 recorded by the writer, not only by Python logs.
- Catalog terminal PNG drain/error in Results (labels now exist at
  `6c6bb2d5`; headed write + honest sidecar still missing).
- Headless evidence **file** (compact `Evidence` + identity JSON) so a
  no-op sink still leaves a durable record without inventing a PNG.
- In-app Results surface (panel thumbnail + TUI text).
- Bounded in-process event timeline (seed/step/proof/shot/pass/fail),
  capped, not per-tick.
- Capture error propagation into `Evidence` / Results ("readback
  dropped", "shot dir failed") instead of only `eprintln!`.

### Obsolete / do not reintroduce

- OS screenshot as the product camera.
- Per-tick world copies or a diagnostic snapshot stream as mandatory.
- Campaign managed process controllers, replay, heaptrack.
- Debug-only capture that ordinary release binaries cannot run.
- A second scenario engine for "evidence scenarios".
- Treating process exit 0 as visual proof (`docs/harness.md` already
  forbids this for memory; catalog headed receipts still need the same
  discipline for PNGs).

## Ownership, API, UI seams

### Application (panel-play / tui-play / scenario)

New small type, e.g. `scenario::CaptureMeta` (name flexible), written as
the sidecar instead of raw pretty `GameSnapshot` alone:

```text
kind: milestone | terminal | on_demand | smoke
label: string
at: utc stamp
ingame, scene_state, tick, tile
snapshot: GameSnapshot | null
snapshot_honest: bool
window: whole_window_imgui
settle_ms: optional
error: optional
```

`snapshot_honest` is false when F12 has no focused snapshot, when
readback failed, or when `scene_state != 2` and the caller asked for
gameplay proof. Never serialize `GameSnapshot::new()` as if it were
observed state.

`ScenarioRunner::shot_sink` stays `FnMut(&str, &GameSnapshot)`. Panel
wraps it: add envelope fields from the live profile/session, enqueue.
Do not put GPU types in `scenario`.

`write_shot` stays bytes-in, paths-out. Optional follow-up: write
`<base>.sha256` or a `manifest.json` in the run dir after each write.

Panel Results dock (right panel or a small collapsible): last outcome,
predicate, tile, scene, path to PNG, thumbnail from the already-copied
RGBA (no extra readback), last error. Interactive F12 lists in the same
log. Do not open the PNG in an external viewer as a requirement.

TUI Results: last `Evidence` pretty-printed in the status/script area.
No image. If no evidence yet, show runner step. That is the honest
headless surface.

### Shipped CLI

No new capture CLI in this increment. `panel-play --smoke` already is
the capture smoke. `tui-play --live` already prints evidence.

A later optional `274bot-evidence` subcommand that packs a run dir
(manifest + png + sidecar + evidence JSON) can live as a tiny bin, not
a build-time tool. Not required to stop.

### Build-time

Binary identity JSON (`binary-<source>.json`) remains a frozen-export
concern. The app should **record** `CARGO_PKG_VERSION` and, when
present, an env-supplied `274BOT_BUILD_ID` / git describe. It must not
shell out to `git` during a user session.

### External operator automation

`run_cell.py` / `run_headless_cell.py`: keep spawning, env, frozen
binary check, engine/nav hashes, log/timeline. After wait, hash
`$274BOT_SMOKE_DIR/**` into the receipt. Do not parse PNGs as
acceptance. Windows `windows-capture-owned.ps1` remains a last-resort
operator photo of a hung window, labeled as **not** the application
camera and **not** tied to `GameSnapshot`.

Server-admin / moderator fixtures stay isolated. Capture must not imply
test privileges in a normal vault.

## Thread ownership and completion/error

| Event | Thread | Success | Failure |
|---|---|---|---|
| Arm / fire sink | Slot (runner tick) | Push `(label, json)` | No sink: silent no-op (headless). |
| Drain requests → wanted | UI `pump_shots` | Extend `wanted` | Smoke settle: put requests back. Shot dir create fail: skip writes, eprint. |
| Readback | Render, after imgui | `done` gets RGBA | Map fail: drop. Lost surface: skip frame. |
| Write PNG | UI `pump_shots` | print path, count++ | eprint; smoke will deadline. |
| Exit hold | UI live watch | wrote_shots > 0 or 10 s | FAIL path still exits 1 after drain. |

Screenshots are tied to observed state only if the sidecar is the
snapshot the sink received on that fire, and the PNG is the window
presented on the subsequent render. That is already the queue order.
Gaps: empty F12 JSON; catalog not firing; dropped readback with no
envelope error; smoke settle still captures chrome + whatever the 1 fps
slot had drawn, which may be one frame behind the sink snapshot.

Do not wait on slot-thread file I/O. Do not read back from a skip-paint
client texture. Do not deep-copy the world to "help" the sidecar; the
existing `GameSnapshot` rebuild is the observation.

Full `GameSnapshot` JSON can be large (locs, scene, widgets). Prefer
the compact `Evidence` plus a short snapshot subset (tile, inv, scene,
ingame, chat head) in the envelope; keep optional `full_snapshot=true`
for `--smoke` and F12 when the operator wants the whole sidecar. That
is a size/maintenance choice, not a second camera.

## Staged implementation cards

Suggested orch cards after this design. Not created here. No LIVE in
the first two.

1. **Honest capture envelope + F12** — `scenario` envelope type + tests;
   panel `enqueue_manual_shot` uses focused snapshot or `snapshot: null`.
   Acceptance: unit test that default snapshot is not written as honest
   game state; F12 with a focused slot includes `scene_state`/`ingame`.
2. **Catalog terminal shots** — **done at `6c6bb2d5`** for labels.
   Do not re-edit `lib.rs` for the same arms. Follow-up is headed
   drain/Results using those labels, not another settings patch.
3. **Run manifest + hashes** — on first `create_run_dir` / first write,
   emit `manifest.json` with profile, revision, host/client commits if
   the session knows them, stamp, pid. After each `write_shot`, record
   png/json SHA-256. Acceptance: shot.rs temp-dir test.
4. **Results surface** — panel Results list; TUI evidence pane. No new
   GPU path. Acceptance: panel/TUI unit on the display struct, not a
   live window.
5. **Headless evidence file** — when sink is no-op, still write
   `evidence.json` next to the test output dir if `274BOT_SMOKE_DIR` is
   set. Acceptance: existing catalog harness prints already; add a file
   write behind the env, tested with a temp dir.
6. **Bounded event timeline (optional next)** — JSONL of seed / step /
   proof / shot / pass / fail, cap 256 lines, in the run dir. Replaces
   the need for Python to be the only clock. Do not log every tick.

Card 6 is the first thing to cut if scope slips. Cards 1, 3 and 4 are
the remaining useful compatibility-release wiring. Card 2's labels are
already on HEAD; treat `crates/scenario/src/lib.rs` as a hotspot and do
not pile more catalog scenario body onto it in the capture lane.

## Risks

- **Hotspot** `crates/scenario/src/lib.rs`: catalog terminal labels just
  landed in `6c6bb2d5` with bank-fletcher loop changes. Capture follow-up
  should not add more scenario body there.
- **Smoke false pass** if any PNG write counts. Keep F12 disabled in
  live; do not let Results-pane refresh enqueue shots.
- **Readback stall**: `device.poll(Wait)` is already on the render path.
  Do not add extra captures per frame. Milestone/terminal/on-demand only.
- **Sidecar size** if full snapshots land every FAIL. Default compact.
- **scene_state==1 freeze** looking like success. Envelope must show
  scene; Results must show it; acceptance must not infer world from PNG
  filename (`docs/execution.md`).
- **Unfocused 1 fps** Game pane vs chrome. Whole-window PNG is an
  operator photo of the UI, not a proof that every slot rendered.
- **Headless PNG temptation**. Refuse. Evidence JSON is the headless
  artifact.
- **Privileged fixtures** leaking into user capture dirs. Manifest
  records profile id, not credentials. Shot dir stays under smoke/ or
  the caller-supplied evidence path.

## Focused validation (no LIVE in this design)

- Existing: `scenario` shot naming/write tests; runner terminal-snapshot
  release test; panel `to_rgba` / smoke settle predicates.
- Add: envelope honesty tests; F12 null-vs-focused snapshot; manifest
  hash of a 2×2 PNG.
- Do not run LIVE, headed catalog, or Windows capture to accept this
  design.
- Later live proof (not this card): one `--smoke` PNG with
  `scene_state==2` in the envelope; one headed FAIL with terminal PNG
  whose sidecar tile matches `Evidence.tile`; one TUI FAIL with evidence
  JSON and no PNG. A human or vision-capable tool reads the PNG; the
  filename is not the proof.

## What belongs where (summary)

| Need | Place |
|---|---|
| GPU window camera | Application panel, existing readback |
| Compact pass/fail record | Application `Evidence` |
| Honest capture metadata | Application envelope + run manifest |
| On-demand shot | Application F12 |
| Milestone/failure shot | Application `StepKind::Shot` / `terminal_shot` |
| Readable last result | Application panel Results + TUI text |
| Headless durable record | Application file write when `274BOT_SMOKE_DIR` set; no PNG |
| Frozen binary / catalog identity | Operator/build receipts |
| Process launch, stdout timeline | External Python, until card 6 |
| Hung-window OS photo | External PowerShell, labeled non-product |
| RSS/CPU collectors, owner census | Stay out |

## Suggested stopping point

Stop after cards 1, 3 and 4 land on top of `6c6bb2d5` labels. At that
point a headed catalog FAIL can produce a PNG tied to a real snapshot,
F12 is not a lie, and both frontends show the last evidence without
leaving the app. Python receipts can then hash the shot dir. That is
enough for this compatibility release.

Do not wait for an in-process timeline, an evidence pack CLI, TUI
images, or Windows HWND capture to call the integration useful.
Do not restart memory-campaign capture systems to get there.
