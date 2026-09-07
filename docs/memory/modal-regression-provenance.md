# GPU main-modal regression — provenance

**Task:** `t_d7d33b21` (follow-up to `t_0fbec043`)  
**Scope:** read-only provenance. No production/test edits, no git mutations on the campaign checkout, no live game, no Windows work.  
**Checkout:** host `codex/memory-diagnostics` @ `dc6000d` (workspace path `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`); client submodule `codex/windows-native-parking` @ `2b1af85`.  
**Prior diagnosis:** `docs/memory/windows-native-additional-failures.md` (Mac+Windows black hole; CPU buffers correct; force atlas dirty paints SEA; exact missed branch still open).

## Verdict (for root decision)

| Question | Answer | Confidence |
|:---|:---|:---|
| Did the exact synthetic main-modal scene paint correctly **before** lazy chrome / memory-era GPU upload gating? | **Yes** | **High** (Mac release exact probe) |
| First bad client change? | **`f2504be`** — `fix(gpu): stable present texture + lazy chrome + minimap layer` (2026-09-04) | **High** |
| Parent still good? | **`cc81f6e`** (`f2504be^`) — same test body, **PASS** | **High** |
| Did later “overlay refresh” / minimap-hold commits restore it? | **No** — `bc8bb4e`, `451759f`, and HEAD `2b1af85` still **FAIL** black `0` vs `0x336699` | **High** |
| Is this a memory-campaign behavior break vs an ancient pre-campaign miss? | **Memory-campaign-era client GPU change** introduced the fail; not “always broken because cross-platform.” Cross-platform only means Mac and Windows both show the post-`f2504be` hole. | **High** |
| Campaign posture | Fixing is **required behavior restoration** for the GPU main-modal / overlay-coverage contract the suite already encodes — not a new product feature. The miss is the lazy-chrome gating introduced in `f2504be`, not the freeze feature that added the test. | **High** on classification; medium on the single best one-line patch (see open seam) |

## Test under study

- File: `vendor/fr-client-rust/crates/client/tests/iface_model.rs`  
- Name: `gpu_main_modal_rect_is_opaque_over_the_scene`  
- **Introduced:** `6c03f8f` (`feat: freeze the last GPU scene during map rebuild`, 2026-09-01)  
- **Body unchanged** from intro through HEAD (`git diff 6c03f8f HEAD -- …/iface_model.rs` empty for this test).  
- Fixture: prefer GPU → `game_draw` once (ingame, `scene_state==2`) → synthetic `TYPE_LAYER` 90 + filled `TYPE_RECT` 91 colour `SEA=0x00336699` → `main_modal_id=90` → second `game_draw` → readback center `pixels[24*765+24] & 0xffffff == SEA`.

No parent API adaptation was required: the assertion and setup are identical on every probed rev.

## Mac release exact bisect (scratch clone)

Shared clone outside the campaign tree: `/tmp/modal-prov-client2` from client object store. No edits to the worktree submodule. Command each rev:

`cargo test --release -p client --test iface_model gpu_main_modal_rect_is_opaque_over_the_scene -- --exact --nocapture`

| Rev | Role | Result |
|:---|:---|:---|
| `6c03f8f` | test origin (always-upload chrome finish) | **PASS** (~0.11s) |
| `cc81f6e` | immediate parent of lazy chrome (`f2504be^`) | **PASS** (~0.23s) |
| **`f2504be`** | **lazy chrome + minimap layer** | **FAIL** black `0` vs `3368601` (`0x336699`) |
| `bc8bb4e` | viewport overlay refresh without UI redraw flags | **FAIL** same |
| `451759f` | held minimap on overlay-only freeze uploads | **FAIL** same |
| `2b1af85` | current client HEAD | **FAIL** same |

First-bad is therefore **`f2504be`**, not `bc8bb4e` / `451759f`. Those later commits neither introduced nor cured this failure.

## Mechanism (source, tied to the bisect)

### Before `f2504be` (`cc81f6e` / `6c03f8f`)

`GpuBackend::finish` **always** rebuilt the chrome RGBA from `draw_area` + `overlay_coverage` and `write_texture`d every frame. Main-modal paint into `area_game` + coverage marks on the second fixture frame were therefore always present in the composited atlas over the scene hole.

### `f2504be` change (first bad)

Commit message / diff intent: stable present texture; **chrome atlas upload only when redraw flags fire**; minimap split to its own layer.

`GpuBackend::chrome` now sets:

```text
atlas_dirty ⇐ title | menu | side_modal_id|chat_modal_id | selected/obj_drag areas |
              tut_flash | chat scroll drift | redraw_side/chat/icons/chat_mode
chrome_upload_pending = atlas_dirty || !chrome_uploaded
```

**`main_modal_id` is not in that dirty set.** Side/chat modals are.

`finish` uploads only when `chrome_upload_pending` (first frame seals the atlas with empty scene-window coverage for this fixture’s first `game_draw`; second frame with pure main-modal has no HUD dirty → **no re-upload** → hole stays transparent → black scene shows through).

That matches the existing diagnosis: CPU `area_game` / `draw_area` hold SEA; force any atlas dirty and SEA appears; ordinary chrome outside the hole still paints.

### `bc8bb4e` / `451759f` (attempted overlay path — still red)

`bc8bb4e` adds `scene_overlay_changed(draw_area, overlay_coverage, chrome_rgba, …)` so `finish` can re-upload when viewport overlay alpha/RGB diverge from the last atlas **without** chat/sidebar redraw flags. Comment explicitly mentions frozen scenes / modal pixels.

`451759f` only adjusts minimap punch/hold when that overlay-only path fires.

**Mac probes show both still FAIL** the same black-center assert. So the intended independent overlay refresh does **not** restore the pure main-modal synthetic path (consistent with `t_0fbec043`: unit-level coverage+finish can pass while full `game_draw` main-modal stays black). Provenance does **not** re-open a full instrumentation of why `overlay_changed` is false or ineffective on that path; prior doc already flags coverage-mark vs compare-buffer uncertainty. For root: **the regression starts at lazy gating in `f2504be`; later overlay commits did not close it.**

## What this is / is not

- **Is:** a regression of GPU presentation of main-modal chrome over the 3D hole, introduced by memory-era lazy chrome upload (`f2504be`), against a test that already passed on the freeze-era always-upload backend.  
- **Is not:** a Windows-only / DX12 defect (Mac Metal fails identically post-`f2504be`).  
- **Is not:** proof the test was wrong or that the failure predates the memory campaign.  
- **Is not:** the separate shade-16 or CRC Windows-timing failures.

## Explicit uncertainty

1. **Exact single-line production fix** (dirty `main_modal_id` vs make `overlay_changed` reliable vs both) — still for a fix card with one-frame instrumentation; this card only pins **first bad rev** and campaign classification.  
2. **Why `bc8bb4e`’s compare path does not save the full `game_draw` pure-main-modal fixture** — not resolved beyond prior diagnosis; does not weaken the bisect.  
3. **No Windows re-run here** (not authorized); Windows black-hole evidence remains from `t_0fbec043` on the same post-`f2504be` lineage.

## Artifacts

- Scratch client clone: `/tmp/modal-prov-client2` (shared from submodule; checkouts of probe revs only there).  
- Prior symptom/path analysis: `docs/memory/windows-native-additional-failures.md`.

## Recommendation to root

Treat restoration of `gpu_main_modal_rect_is_opaque_over_the_scene` (and sibling main-modal / ship_journey coverage over the hole) as **in-campaign behavior restoration** tied to undoing or completing the `f2504be` lazy-chrome contract for main-modals — not as optional greenfield work and not as a waived/timeout/shader redesign.
