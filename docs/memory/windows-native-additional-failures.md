# Windows native additional failures — modal RECT + CRC bound

**Task:** `t_0fbec043`  
**Scope:** read-only diagnosis only. No production/test patches, no git mutations, no timeout extensions, no weakened/skipped asserts, no shader/render changes in this card.  
**Workspace:** host `codex/memory-diagnostics` @ `63c8baf` (earlier freeze notes use `952ba22`); client submodule `codex/windows-native-parking` @ `2b1af85`.  
**Exclude:** `gpu_texture::gpu_textured_shade_scales_texel_brightness` (already covered by `windows-gpu-shade-diagnosis.md`).  
**Native freeze (root):** host `952ba22` / client `2b1af85`, Windows 11 x64, Rust 1.98 MSVC, `cargo test --locked --release -p client --no-fail-fast -- --test-threads=1` → **763 passed / 3 failed**. Isolated fresh-process EXEs reconfirmed both failures below (no concurrent build/live load).

## Summary table

| Failure | Native isolated | Mac exact (this card) | Class |
|:---|:---|:---|:---|
| `iface_model::gpu_main_modal_rect_is_opaque_over_the_scene` | FAIL black `0` vs `0x336699` (~2.94s); EXE SHA `ACF3389F…E8F73F` | FAIL same assert (black center) | **Cross-platform GPU main-modal chrome upload / coverage seam** — not Windows-only |
| `maininit::crc_unreachable_sets_error_loading_in_bounded_time` | FAIL took **20.966s** (bound 5s); EXE SHA `8BC750FE…FA65D1` | **PASS ~0.54s** | **Windows TCP connect latency × 10 retries** vs test assumption “refuse instantly” |

Artifacts: `docs/memory/diagnostics/windows-native-20260907a/`  
- Full suite log: `test-952ba22-client-all-targets.log`  
- Isolated: `isolated-952ba22-{iface_model,maininit}.log`, `isolated-followup-952ba22.jsonl`  
- Suite receipt: `native-tests-952ba22.jsonl`

---

## 1. `gpu_main_modal_rect_is_opaque_over_the_scene`

### What the test requires

File: `vendor/fr-client-rust/crates/client/tests/iface_model.rs` ~191–252

1. Prefer GPU; two `game_draw` frames with `ingame` + `scene_state==2`.
2. Synthetic main-modal: `TYPE_LAYER` root 90 + filled `TYPE_RECT` child 91, colour `SEA = 0x00336699`, `main_modal_id = 90`.
3. Read back GPU frame; center sample `pixels[24*765+24] & 0xffffff` must equal `SEA`.

Comment intent: ship_journey / glidermap main-modals draw into `area_game`; the GPU scene window is a 3D hole unless **overlay coverage** marks opaque chrome over the hole — not only `TYPE_MODEL`.

### Observed evidence

**Windows (root isolated EXE):**  
`main-modal TYPE_RECT must cover the GPU scene hole, got 0x000000` (left 0, right 3368601).

**Mac (this worktree, release exact):** same panic / black center.

**Outside-tree GPU probe (Mac, same client path, no production edits):**

| Check | Result |
|:---|:---|
| `area_game` size | `512×334` (matches `SCENE_W/H`) |
| `area_game[20,20]` after modal frame | `0x336699` |
| `draw_area[24,24]` after modal frame | `0x336699` |
| GPU readback scene window nonzero RGB | **0** (fully black hole) |
| Chrome outside scene (`readback[0,0]`) | non-black (chrome composite alive) |
| Force atlas dirty (`redraw_side` / `selected_area==2`) then redraw | center becomes **`0x336699`** |
| Direct `fill_rect` / `draw_interface` under `coverage_guard` | coverage marks + RGB both non-zero |
| Unit `viewport_overlay_moves_and_clears_without_chrome_redraw` | **PASS** (manual `overlay_coverage` + `finish`) |

Sibling `gpu_ship_journey_paints_the_title_over_the_scene` also failed here with **0 overlay px** in the scene window (same hole class). Earlier campaign notes that ship_journey “passed” on some machines are **not** treated as proof the seam is healthy for pure main-modal-only frames.

### Call path (concrete)

```
Renderer::game_draw
  → render_frame(Game)
    → GpuBackend::scene (scene_state==2)
         render_scene → scene_ready=true
         draw_scene_overlays:
           overlay_coverage.fill(0)
           coverage_guard(overlay_coverage, 512, 334)
           entity_overlays / coord_arrow / other_overlays
             other_overlays (draw.rs ~492–522):
               if main_modal_id != -1:
                 animate_interface + draw_interface → TYPE_RECT → Pix2D::fill_rect
                   (+ mark_rect when coverage attached)
    → GpuBackend::composite_scene
         scene_state==2 → cpu.composite_scene → area_game.blit_into(draw_area, 4, 4)
    → GpuBackend::chrome
         atlas_dirty from side/chat modals, menus, redraw_* flags, selected/obj_drag areas, …
         **does not list main_modal_id**
         chrome_upload_pending = atlas_dirty || !chrome_uploaded
         cpu.chrome (side/chat panels; side/chat modals only)
    → GpuBackend::finish
         copy scene_texture into frame at (4,4)
         overlay_changed = scene_ready && chrome_uploaded
             && scene_overlay_changed(draw_area, overlay_coverage, chrome_rgba, …)
         if chrome_upload_pending || overlay_changed:
             fill_draw_area_rgba(… coverage → scene-window alpha …)
             write_texture(chrome)
         composite chrome (SrcAlpha) over scene
```

Scene-window alpha rule (`fill_draw_area_rgba`, gpu.rs ~1679–1684): for `x∈[4,516), y∈[4,338)` alpha = `coverage[(y-4)*512+(x-4)]`. Alpha 0 → transparent chrome → empty scene (black) shows through. Alpha 255 → opaque overlay RGB from `draw_area`.

### Root-cause analysis

**Not:** wrong RECT colour selection for this fixture (SEA is in `area_game` / `draw_area`).  
**Not:** missing `area_game` or wrong size (512×334).  
**Not:** dead chrome pipeline (outside-scene chrome paints).  
**Not:** Windows-only / DX12-only (Mac Metal reproduces).  
**Not:** the separate textured-shade-16 issue.

**Best-fit defect (high confidence on symptoms; medium on the exact missed branch):**

1. CPU main-modal paint and blit are correct (SEA in CPU buffers).
2. GPU presentation of that paint requires a chrome atlas upload whose scene-window alpha comes from `overlay_coverage`.
3. `GpuBackend::chrome` marks atlas dirty for `side_modal_id` / `chat_modal_id` (and several HUD flags) but **never for `main_modal_id`**. Side/chat modals also force `redraw_side` / `redraw_chat` in `CpuBackend::chrome`; main modal does not — it only draws in `other_overlays`.
4. Lazy path `scene_overlay_changed` is supposed to refresh viewport overlays without UI redraw flags (commit `bc8bb4e`). The direct unit test that mutates `overlay_coverage` and calls `finish` **passes**. The full `game_draw` pure-main-modal path still leaves the hole black.
5. Forcing any atlas-dirty signal on the next frame makes SEA appear — so **upload + blend + readback are capable** once the atlas is rebuilt with current coverage/RGB.

**Remaining uncertainty (explicit):** whether full-path `other_overlays` sometimes fails to attach/mark coverage (TLS `coverage_guard` + matching 512×334 surface — dimensions match when measured), or whether `overlay_changed` is false for another reason (stale/`chrome_uploaded` gating, compare buffer). Either way the **user-visible miss is the same**: main-modal pixels stay in CPU maps and never win the scene hole. A fix card should instrument one frame of `overlay_coverage` nonzero count and `chrome_upload_count` across pure vs dirty-forced frames before patching.

### Ordinary runtime impact

**Yes, for GPU main-modals** (`ship_journey`, `glidermap`, mystery-cube-class overlays) whenever the frame does not independently dirty the chrome atlas (side/chat redraw, menu, drag, etc.). CPU backend is unaffected (no hole/coverage seam). Live panel may still look fine on frames that redraw chrome for other reasons — flaky relative to pure modal tests.

### Scoped next step (when authorized — not this card)

1. Instrument (scratch/test-only): after modal `game_draw`, assert `overlay_coverage` any-nonzero and `chrome_upload_count` increments without forced HUD dirty.
2. Minimal production fix candidates (pick after instrument):  
   - Treat `main_modal_id != -1` (and/or coverage dirty) as atlas dirty in `GpuBackend::chrome`, mirroring side/chat; and/or  
   - Ensure `draw_scene_overlays` coverage always drives `overlay_changed` / pending upload.  
3. Keep `gpu_main_modal_rect_is_opaque_over_the_scene` strict; add regression if needed for ship_journey freeze path.  
4. **Do not** “fix” by reading CPU `draw_area` in the test or by skipping GPU.

---

## 2. `crc_unreachable_sets_error_loading_in_bounded_time`

### What the test requires

File: `vendor/fr-client-rust/crates/client/tests/maininit.rs` ~463–496

1. Bind `127.0.0.1:0`, drop listener → **unused** port; set `c.http_port = unused`.
2. `fetch_retry_wait = 1ms` (collapse countdown sleeps).
3. `maininit()` must set `error_loading`, message `"Game updated - please reload page"`, finish in **&lt; 5s**.

Comment assumption: “connect() refuses instantly, so each fetch attempt is fast and the whole 10-attempt retry budget is the countdown.”

### Observed evidence

| Platform | Result |
|:---|:---|
| Windows isolated EXE | FAIL: took **20.9661454s** (wall ~20.99s) |
| Mac release exact | **PASS ~0.54s** |
| Mac `TcpStream::connect` ×10 to dropped unbound port | ~**0 ms** each (instant refuse) |

Windows ~21s / 10 attempts ≈ **~2.1 s per connect attempt** — classic slow localhost-refuse / stack delay class, not the 1 ms `fetch_retry_wait`.

### Call path (concrete)

```
Client::maininit
  → fetch_jag_checksums (client.rs ~1820–1839)
       loop up to 10 failures:
         get_jag_checksums(host, http_port)
           → http_get → http_get_plain (local BOT_TARGET; not HTTPS)
                TcpStream::connect((host, port))   // NO connect_timeout
                // fail → "connection problem"
         retries++; if retries >= 10: report "Game updated…"; return None
         retry_countdown(wait=fetch_retry_wait, …)
           ticks = wait.as_secs().max(1)  // 1ms → 1 tick
           sleep(wait/ticks)              // ~1ms
         wait = min(wait*2, 60s)
  → error_loading = true on None
```

Secure transport (`uses_secure_transport` / HTTPS :443) is **off** for default local `BotTarget` — not involved in this test.

### Root-cause analysis

**Functional path is correct:** unreachable `/crc` eventually sets `error_loading` and the Java reload message (Windows failure is the **time assert**, not wrong flags/messages in the panic text).

**Dominant delay on Windows:** blocking `TcpStream::connect` to an unbound localhost port is **not** instant (order ~2s × 10 ≈ 21s). Mac refuse is instant → test passes under 5s.

**Not:** wrong retry count logic; not `fetch_retry_wait` alone (1 ms sleeps cannot sum to 21s); not a production HTTPS misfire in this fixture.

**Test assumption vs platform:** the test encodes a Unix-like “instant RST” assumption that **Windows does not satisfy** for this connect pattern.

### Ordinary runtime impact

- **Dead jag HTTP on Windows:** each plain connect attempt can add multi-second latency before the port’s countdown sleep. With **default** `fetch_retry_wait` (5s, doubling to 60s), countdown sleeps dominate the UX; Windows connect tax is extra but secondary to the intentional multi-minute-style retry budget.
- **Healthy servers:** no impact (connect succeeds).
- **This failure mode is primarily a CI/native-test portability miss**, not proof that checksum logic is broken.

### Scoped next step (when authorized — not this card)

Per campaign rules: **do not** raise the 5s bound or weaken/skip the assert as the “fix.”

Preferred directions (choose in a fix card):

1. **Production hardening (small):** `TcpStream::connect_timeout` with a short bound on plain jag HTTP (local dead-server path), preserving retry count and messages; or  
2. **Test isolation:** inject a connect/http seam so “unreachable” does not depend on OS TCP timing; keep wall bound tight.  

Avoid changing the 10-attempt / “Game updated” semantics.

---

## Cross-links

- Shade-16 (separate failure): `docs/memory/windows-gpu-shade-diagnosis.md`  
- Native proof rollup: `docs/memory/windows-native-first-proof.md`  
- Overlay lazy upload intent: client commits `bc8bb4e` (viewport overlay refresh), `451759f` (minimap hold on overlay-only freeze)

## Explicit non-claims

- No full-suite green, no memory savings, no adapter identity for the live panel from this card.  
- No authorization claimed for shader redesign, timeout waiver, or test deletion.  
- Modal miss is **not** classified as “Windows-only”; CRC bound miss **is** Windows-timing vs Mac.

## Bottom line

| ID | Verdict | Fix posture |
|:---|:---|:---|
| Modal RECT black hole | Real GPU main-modal presentation bug: CPU has SEA; scene hole stays uncovered unless chrome atlas is forced dirty. Dirty-flag gap for `main_modal_id` + unreliable lazy coverage upload on full `game_draw`. | Fix render chrome/coverage seam; keep assert. |
| CRC &gt;5s on Windows | Real platform timing: 10× slow connect to unused port ≈21s; logic/messages OK; Mac fast. Test assumption false on Windows. | Prefer connect_timeout or test seam; do not merely extend bound. |
