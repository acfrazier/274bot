# Remaining native visual behavior proof plan

Status: **plan only**. Not authorization to claim gates passed. Root chooses
execution after per-task review. No Rust/runtime/control/STATE changes in the
task that produced this document.

References: `docs/execution.md`; `performance-finish-plan.md` §§5–6 (behavior /
lifecycle and separate focus/watch, overlays, minimap freeze, CPU-renderer
visual checks); `AGENTS.md` (preserve last-FBO freeze while `scene_state==1`);
`behavior-contract.md` (visual correctness); `renderer-live-protocol.md`
(captures alone are not visual proof); `tile-boxed-fields-native-protocol.md`
(last-FBO / focus / attach-detach as distinct gates); `windows-tile-boxed-native-freeze.json`
pending line: “Native live CPU/GPU visual and last-FBO/focus/detach proof”.

## 1. Frozen subject under proof

| Field | Value |
| --- | --- |
| Candidate host | `fb3589ac28583242b999ac864ea69c4ef8fa5923` |
| Candidate client | `fd956c91bf09e059359c8e182a33583e2c626cd3` |
| Candidate checkout | `/Users/acfrazier/experiments/274bot/.worktrees/windows-render-owner-census` branch `codex/windows-render-owner-census` (verified HEAD host=`fb3589a…`, client gitlink=`fd956c9…`) |
| Native binary SHA-256 | `a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f` |
| Stage | `C:/ProgramData/274bot-Test/tile-boxed-fb3589a` |
| Freeze receipt | primary `docs/memory/windows-tile-boxed-native-freeze.json` |
| Baseline (paired context only) | host `9268890…` / client `abb811b…` / binary `e2deb1db…` — freeze `windows-tile-probe-native-freeze.json` |
| Write workspace for this plan | primary `codex/memory-diagnostics` @ `.worktrees/t_a1f4796f` |

Use the already-frozen native panel binary. Do not rebuild for this proof
series. Verify staged hash before launch. Prefer GPU (restored measurement
runtime; original four Python runtime files already restored after CPU pair —
`scene1_freeze_proven: false` in `windows-tile-cpu-paired-0215-receipts.json`).
CPU path is a **separate** cell with `RasterMode::Cpu` / process `BOT_CPU=1` on
the same frozen binary lineage if GPU-init-failure path is still required; do
not contaminate clean GPU resource cells.

## 2. Gates (what “pass” means)

| ID | Gate | Observable pass | Observable fail / missing capability |
| --- | --- | --- | --- |
| G1 | `scene_state==1` last-FBO freeze | During rebuild: 3D viewport keeps last live frame (not black, not TV-static zap only, not a fresh mesh rebuild); “Loading - please wait.” splash when fonts present; **minimap retained** (held texture / not blank hole); modal/IF overlays still composite over freeze | Blank 3D hole; rotating geometry during load; minimap wipe; splash missing when p12 present; freeze lasting after `scene_state==2` without motion resume |
| G2 | Focus / watch switch | Changing focused slot (or only-render-selected rail) moves full-rate Game seat; unfocused skip-paint / 1 fps watch keeps sim; focused seat paints non-zero frames; no cross-slot frame bleed | Wrong slot paints; unfocused head grows when policy forbids; stale pixels of previous focus after switch without reattach |
| G3 | Renderer attach / detach | Draw-off / Raster Off / GPU↔CPU flip drops head and reattaches on same `Client`; `attach_n`/`detach_n` (if profile on) move; post-reattach scene+minimap restore without account restart | Client restart; blank forever after reattach; mailbox Texture use-after-free (crash); headed overlay meshes left on sim after detach |
| G4 | Overlay behavior | Chrome/chat/sidebar/script-paint/queue-card/minimap composites correctly in scene2 **and** over G1 freeze (main modal / ship_journey class); dirty chrome updates without destroying held minimap | Overlays vanish on freeze; dock-boats-without-chart class bug; chrome seals minimap hole incorrectly |

These are **behavior** gates, not RSS/CPU budgets. §5 non-regression margins do
not apply. Do not change gameplay policy, fixtures, timeouts, or cadence to
make a cell look green.

## 3. What existing automated coverage already shows (and cannot)

### 3.1 Client unit / integration (offline or cache-local)

| Artifact | Proves | Cannot prove |
| --- | --- | --- |
| `vendor/fr-client-rust/.../tests/rebuild.rs` `rebuild_normal_paints_loading_splash_when_draw` | REBUILD → `scene_state=1`; `area_game` not cls’d (frozen fill) ± splash text | Live panel, real GPU FBO, minimap hold, multi-slot host |
| `.../render/backend/gpu.rs` `freeze_last_scene_*`, minimap-hold composite unit | Predicate Game+state1; freeze does not cls overlays; freeze overlay path retains minimap pixel in synthetic atlas | End-to-end panel present path; operator-visible PNG; attach/detach |
| `.../tests/iface_model.rs` `gpu_ship_journey_stays_over_a_frozen_scene` | GPU freeze still composites IF chart over last scene | Host lifecycle, focus policy, native Windows adapter |
| `.../tests/client_build.rs` detach re-arms share_light / set_draw enables overlay_mesh | Sim-side attach flags | Host `Renderer` drop, mailbox drain |
| Host `crates/host/src/lib.rs` tests: `draw_off_drops_renderer_draw_on_reattaches`, `prefer_cpu_rebuilds_renderer_not_client`, `render_profile_observes_attach_detach_*`, dematerialize-on-detach | `client_frame` attach/detach, prefer_cpu flip, profile counters, minimap dirty | Live network scene rebuild, real FBO freeze image, panel UI |
| Panel `crates/panel/src/focus.rs` unit + `memory_draw_policy` | FocusedOne / FocusedPlusBackground / only-render-selected policy math | Actual paints, GPU residency, freeze |
| e2e `crates/e2e/tests/panel_view.rs` LIVE | Headless `set_draw(true)` + non-zero frame + capture walk at scene2 | Frozen Windows binary; scene1 freeze; multi-bot focus; FBO minimap hold |

**Rule:** green unit/host tests are **necessary regression oracles**, not native
visual proof. Do not mark G1–G4 from Cargo alone.

### 3.2 Existing native / live harness hooks (already on frozen binary)

| Hook | How to exercise on frozen panel | Observable |
| --- | --- | --- |
| Status / log `scene N` | `Session` status pump logs `{name}: scene {n}` on change (`session.rs` ~2023–2028); status strip “ingame scene {}” (`app.rs`) | Confirms logical `scene_state` without reading client memory |
| F12 whole-window capture | `Key::F12` → `manual-<stamp>` shot via `pump_shots` (`app.rs`) | PNG under panel shot dir; must be **read** by a vision-capable inspector |
| Nav captures (diagnostic only) | `host_play::nav_capture` + panel `nav_capture` when enabled; memory controllers already use them on focused-one functional cells | Scripted labels; still not freeze-aware |
| Focus / Game pane / only-render-selected | Panel UI + `Focus` / `set_renderer` / rail | G2 |
| Raster Gpu / Cpu / Off | `Session::set_focused_raster` → `prefer_cpu` + `set_draw` (`session.rs` ~2704–2722; `vault::RasterMode`) | G3 backend flip and detach |
| Cheats inducing rebuild | `cheat_focused("~home")`, destination cheats, `::tele` path used by scatter/nav (`session.rs`) → server REBUILD_NORMAL → client `scene_state=1` (`client.rs` ~5779) | G1 trigger without new opcodes |
| Capture + click walk | Capture checkbox + Game pane input (`should_capture`); e2e `panel_view` analogue | Distinguishes frozen splash from live scene2 control |
| Opt-in `--render-profile` | Host `render_profile`: per-slot `scene_state`, `attach_n`/`detach_n`, backend, paint/skip, stable_paint intervals; JSONL via host-play memory samples | Instrumentation for G2/G3; **not** pixel proof; keep off clean CPU/latency cells or label diagnostic |
| Zap cadence | `client_frame`: `scene_state != 2` forces full-rate paint so loading is not 1 fps snow (`host/lib.rs` ~422–425, test ~2079) | Loading must animate splash/static path; distinguishes “stuck 1 fps freeze” from intentional FBO hold |

### 3.3 What recent native cells already did **not** prove

- Clean N16 GPU ABBA / focused screens: scene2 qualification + descriptive RSS; no G1.
- CPU N1 pair 0215: six PNGs scene2 market/bank/return; receipts set
  `scene1_freeze_proven: false`, `focus_detach_lifecycle_proven: false`.
- Owner-census / tile-boxed layout: occupancy and layout, not freeze behavior.
- `renderer-live-protocol.md` cell 1 allows nav captures as functional/visual
  diagnostic **outside** clean metrics — still does not define freeze protocol.

## 4. Recommended executable protocol (diagnostic, not clean perf)

**Isolation.** Dedicated diagnostic session. No concurrent clean measurement,
build VM, or other frontends. Do **not** feed RSS/CPU from these cells into
paired savings. If `--render-profile` is on, mark the whole run diagnostic.
No gameplay policy, fixture, or timeout edits.

**Binary.** Staged `tile-boxed-fb3589a` hash-verify
`a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f`.
N=1 focused-one first (simplest visual oracle); optional N=16 only after N=1
G2/G3 pass if multi-head attach storms need evidence.

**Bounded timeout (wall).**

| Phase | Bound |
| --- | --- |
| Login → first `ingame && scene_state==2` | ≤ 180 s |
| Each G1 freeze window (tele/home → scene1 → scene2) | ≤ 90 s total; capture within 2 s of first `scene 1` log |
| Focus switch settle | ≤ 15 s to non-zero new frame |
| Attach/detach round-trip (Off→Gpu or Gpu↔Cpu) | ≤ 30 s to restored non-zero scene2 frame |
| Whole diagnostic cell | ≤ 20 min including teardown |

Fail closed on timeout: preserve logs/PNGs, exit non-zero for the gate, do not
retry indefinitely (§5 park/investigate rule).

### 4.1 Preflight receipts (required before claims)

1. Binary SHA match freeze receipt; stage path recorded.
2. Adapter identity (actual GPU vs CPU fallback) from process/log/profile.
3. Fixture identity (account, world, cache/nav provenance) recorded.
4. `render_profile` on/off explicit; if on, samples.jsonl retained.
5. VM Off / no competing clean cell (operator checklist).

### 4.2 Gate procedures

#### G1 — last-FBO freeze + minimap

1. Reach scene2; F12 **baseline-live** (textured scene, live minimap).
2. Induce rebuild without logout: focused cheat `~home` or long-range tele
   that yields REBUILD_NORMAL (client sets `scene_state=1` at
   `client.rs` ~5779; splash path `Renderer::scene_loading_splash` /
   `check_minimap` at `draw.rs` ~3920–3968; GPU freeze
   `freeze_last_scene` + overlay pass `gpu.rs` ~1191–1199, finish minimap hold
   ~1388–1534).
3. On status log `{name}: scene 1`, immediately F12 **freeze-hold** (and optional
   second shot 0.5–1.0 s later).
4. Wait for `{name}: scene 2`; F12 **post-rebuild-live**.
5. **Human/vision inspection checklist (mandatory):**
   - freeze-hold 3D ≈ baseline geometry (last FBO), not black clear and not a
     newly streamed incomplete mesh thrashing;
   - splash text present if fonts loaded;
   - minimap content continuous with baseline (held), not empty punched hole;
   - if a main modal / IF was open, it remains composited (ship_journey class);
   - post-rebuild-live shows **new** locality/camera consistent with destination
     and **moving** entities/UI — proves freeze ended.
6. **Anti-stale rules (do not mistake freeze for gameplay):**
   - A single PNG of a still scene2 bank booth is **not** G1.
   - A freeze-hold that is pixel-identical to post-rebuild-live with no scene
     log transition is **stale capture**, not pass.
   - Require the ordered triple: live → scene1 log + freeze PNG → scene2 log +
     live PNG with visible progress (tile/base change or entity motion).
   - Qualification helpers that require `scene_state==2` (`qualify_control`,
     e2e `wait_ingame`) must **not** be used as the freeze oracle; they only
     bound the before/after live endpoints.
   - Host zap forces paints while `scene_state!=2`; a 1 fps frozen image with
     no splash and no scene1 log is **watch cadence**, not FBO freeze.

**Pass package:** three labeled PNGs + status log excerpt with timestamps +
adapter/backend note. Optional: render_profile rows with `scene_state` 2→1→2.

#### G2 — focus / watch

1. N≥2 preferred; N=1 only proves Game-pane draw on, not switch.
2. FocusedOne policy: only focused head; switch focus A→B; F12 each; confirm B
   paints, A stops (profile `renderer_present` / paint_n or visual blank rail).
3. FocusedPlusBackground (optional second cell): A full-rate, others ~1 fps
   watch; switch focus; confirm full-rate seat follows focus; backgrounds keep
   low paint rate (profile intervals), sim still advances (script/tile).
4. Capture-on only on focused seat; click-walk on focused proves input path
   (panel_view semantics) without claiming G1.

**Cannot use unit focus tests alone.** Need at least one read PNG or profile
row pair proving residency moved.

#### G3 — attach / detach

1. Scene2 live PNG.
2. Raster **Off** (or uncheck game renderer): expect detach (`renderer` absent,
   `detach_n++` if profile); Game pane empty/placeholder; **client stays ingame**
   (status still ingame, scene2).
3. Raster **Gpu** back on: attach; non-zero frame ≤30 s; minimap recomposed
   (`minimap_level` dirty path in `set_draw` / host insert).
4. Optional: Gpu→Cpu→Gpu via `set_focused_raster` / `prefer_cpu` without logout;
   each edge: detach+attach, restored scene.
5. Fail if process crash, forced relog, or permanent black after reattach.

#### G4 — overlays

1. Scene2: inventory/chat/script overlay/minimap visible (reuse CPU 0215 class
   inspection).
2. During G1 freeze: overlays/splash/minimap hold per G1 checklist.
3. Optional modal: open a known main modal if available on fixture; freeze must
   keep IF composite (unit `gpu_ship_journey_stays_over_a_frozen_scene` is the
   offline oracle; live is stronger).

### 4.3 Required receipts per gate

- Gate id, start/end UTC, binary SHA, host/client commits, backend actual.
- Commands/UI actions sequence (cheat text, focus names, raster transitions).
- Status log lines for scene transitions and errors.
- PNG paths + SHA-256 + inspector identity + pass/fail notes per checklist item.
- If used: render_profile sample excerpts (`scene_state`, `attach_n`, `detach_n`,
  backend, paint_n).
- Explicit `performance_acceptance: false` and `clean_metrics_contaminated: true`
  when profiles/captures on.
- Final boolean per gate: pass | fail | blocked-missing-capability | inconclusive.

## 5. Gate → current capability map (concise)

| Gate | Offline/unit | Host unit | Live e2e | Frozen native UI/hooks | Automated native freeze harness | Verdict for execution |
| --- | --- | --- | --- | --- | --- | --- |
| G1 freeze+minimap | Strong GPU/CPU predicates + splash + IF-over-freeze | Zap cadence only | No | Status scene log + F12 + tele/home cheats + GPU finish hold | **None** | **Executable manually/diagnostic on frozen binary**; no new code strictly required |
| G2 focus/watch | Policy unit only | — | panel_view single-slot draw | Focus UI + memory policies + optional render_profile | No multi-slot visual auto | **Executable** with N≥2 diagnostic panel |
| G3 attach/detach | client_build flags | **Strong** client_frame + profile counters | — | Raster Off/Gpu/Cpu UI | No pixel auto | **Executable**; pair UI with profile or visual restore |
| G4 overlays | IF freeze unit; chrome dirty tests | — | — | F12 + freeze series | No | **Executable** as inspection on G1/G2 PNGs |

## 6. Minimal additional diagnostic hooks (propose only — do not implement here)

Existing mechanisms can prove all four gates on a frozen binary **if** an
operator (or root) drives UI and a vision-capable inspector reads PNGs.
Automation gaps are convenience and fail-closed CI, not fundamental missing
host verbs.

Propose **only if** root wants unattended native proof:

1. **Scene-transition capture arm (panel or host-play diagnostic flag)**  
   When `prev.scene_state != 1 && cur == 1` (and again on return to 2), enqueue
   labeled shots `freeze-enter` / `freeze-exit` without F12. Opt-in env, default
   off; never on clean controllers.

2. **Freeze-window pixel delta probe (diagnostic binary or test hook)**  
   On freeze-enter, hash crop of 3D rect (4,4)–(516,338) over 500 ms: expect
   low motion in world pixels + splash text pixels changing or stable splash;
   on scene2 expect higher motion. Must not alter paint path when disabled.

3. **Publish `scene_state` on nav_capture checkpoint metadata**  
   So functional capture series cannot be mislabeled as freeze proof without
   state tags.

4. **Do not propose:** gameplay telebots, weakened splash expectations,
   disabling zap, pooling scene textures, or clean-cell profile defaults.

If root declines new hooks, execution uses §4 manual/diagnostic procedure only.

## 7. Contamination and anti-false-pass summary

- Diagnostic captures + render_profile ⇒ exclude CPU/RSS/p99 from acceptance.
- Scene2 banking PNGs (CPU 0215 class) ≠ G1.
- Filename “freeze” without scene1 log ≠ G1.
- Unit `freeze_last_scene` green ≠ native G1.
- `qualify_control` scene2 ≠ freeze proof.
- Still image after attach ≠ detach correctness without Off interval evidence.
- Do not average failed freeze attempts into pass.
- Preserve every failure artifact; bounded investigate-or-park (§5).

## 8. Suggested execution order (for root after review)

1. N1 GPU diagnostic: G1 + G4 on fb3589a binary (≤20 min).
2. N2 or N16 focused-one diagnostic: G2 focus switch.
3. Same session or follow-up: G3 Raster Off/On and optional Gpu↔Cpu.
4. Optional CPU RasterMode cell repeating G1 checklist (separate from clean GPU).
5. Record gate table; **no** performance acceptance; whole-branch Grok still later.

## 9. Source index (primary lines)

- Freeze predicate / minimap hold: `vendor/fr-client-rust/crates/client/src/render/backend/gpu.rs` (`freeze_last_scene`, `finish`, minimap_live/held).
- Splash / check_minimap draw half: `.../render/draw.rs` `scene_loading_splash`, `check_minimap`, `mainredraw`.
- REBUILD → state 1: `.../client/client.rs` REBUILD_NORMAL handler ~5779; `set_draw` minimap dirty ~10396–10406.
- Host attach/detach / zap: `crates/host/src/lib.rs` `client_frame` ~404–520, `rearm_after_head_drop`, tests ~1443–1610.
- Focus policy: `crates/panel/src/focus.rs`; raster: `crates/panel/src/session.rs` `set_focused_raster`; F12/`pump_shots`: `crates/panel/src/app.rs`.
- Live headless draw proof: `crates/e2e/tests/panel_view.rs`.
- Freeze receipt pending: `docs/memory/windows-tile-boxed-native-freeze.json`.
- CPU pair explicit non-claim: `docs/memory/windows-tile-cpu-paired-0215-receipts.json` (`scene1_freeze_proven`, `focus_detach_lifecycle_proven`).

## 10. Non-claims

This plan does **not**: implement hooks; run native/SSH; modify STATE; assert
any gate passed; authorize clean retention; replace final lifecycle/scaling
matrix in performance-finish-plan §6; or treat unit green as live gold.
