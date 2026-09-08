# Native null_raster visual session 0742

Status: completed qualitative visual exercise, pending independent report review.
No performance, frame-rate, scene-1 freeze, or full lifecycle acceptance.

## Subject and artifact integrity

Used the frozen candidate `panel-play.exe` (host `fb3589ac28583242b999ac864ea69c4ef8fa5923`,
client `fd956c91bf09e059359c8e182a33583e2c626cd3`, executable SHA-256
`a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f`).
No rebuild or original measurement-runtime edits were made. The existing
`--live null_raster` path creates a temporary vault for local `test` and `test2`
and does not persist personal-vault UI settings. Root launched it as limited
BotTest on console session 2 with memory variables absent, local server/public
RSA/navigation configuration, and diagnostic Intel Vulkan attribution enabled.

PID 14692 started at `2026-09-08T07:41:13.5554029Z`. The harness printed
`PASS: live null_raster`, meaning its two-account readiness check passed.
The root supervisor imposed a 20-minute limit. Root requested normal window
close at the end of this exercise; it terminated at `07:55:20.6699910Z`.
The child exit code was **not recovered** (`terminal.json` contains null).
The supervisor task result 0 is not a substitute for the child exit code.
Final native inspection found no panel/tui frontend; builder remained Off with
zero assigned memory. This is not the campaign's lifecycle stress test.

Native archive:
`diagnostics/windows-visual-proof-nullraster-20260908-0742.tar.gz`
SHA-256 `f718ff2dcf6c55278bfd0cad97f15a905fd0a49e95b4893b0d7f873e583c5968`.
The native manifest SHA-256 is
`3ae95aed5ed84218f8a441b587ccb0556d1e2915a3840d27763b8e388e4635f3`.
Root verified the archive digest and all 78 manifest file lengths/hashes after
transfer. The extracted sibling directory contains 16 PNG/JSON capture pairs,
launch/terminal/close/final-state records, stdout/stderr, the preserved initial
capture failure, and actual native helper scripts/task XML. The tracked
`windows-nullraster-visual-0742-receipt.json` records all capture hashes and times.

The single capture helper `ea7fe6f` was approved by actual Grok 4.5/xai-oauth
session `20260908_033151_6222cc`. Native Windows PowerShell 5.1.26100.9168
parsed it with zero errors and compiled its C# successfully. Native helper hash
was `3e5ade936c9dd61dbc9d9b337cfafed442349503eff103968925dc5fe14d08b0`.
The first attempt refused capture because the target was not foreground; that
failure is preserved. Root then used SetForegroundWindow only on the verified
PID/start/path/session target. Capture revalidated foreground and window state.
Root's mouse actions used coordinates from inspected captures and refused changed
window bounds. These were diagnostic UI operations on the two local test accounts.

## Directly inspected visual observations

Root opened and visually inspected **all 16 PNGs** using an image-reading tool.
The whole-window rectangle remained `[152,152,2250,1078]`. No foreign overlay
obscured the game/status areas used below; a few panel-generated tooltips are
visible. The attribution log identifies the panel's Intel(R) Graphics Vulkan
adapter, Fifo, 1680x870 surface at scale 1.5. That identifies the GUI adapter;
it does not by itself identify the client's selected CPU raster backend.

| Captures | Observation |
| --- | --- |
| `focused-scene2`, `test2-selected`, `test1-returned` | Selected rail/profile/player status changed Test to Test2 and back; ingame scene 2 and two running accounts remained visible. Both initially had the same character-design modal, so these alone do not distinguish their game pixels. |
| `collapse-global`, `raster-none`, `raster-gpu-restored` | GPU selected with modal/minimap visible; none selected displayed “renderer off”; GPU restored the character modal, minimap and chrome. Both accounts remained listed as running. |
| `raster-cpu`, `gpu-after-cpu`, `close-config` | CPU selection and later GPU selection each showed the restored modal/minimap. Closing config exposed Test2 ingame scene 2 and its existing login/scene log, without another visible slot-up/login event. This is UI/backend-selection visual evidence, not a renderer counter measurement. |
| `test1-accept-design`, `test1-accept-held` | Ordinary Accept button attempts did not dismiss the modal at the capture times. No input success or client defect is inferred. No application fix was attempted. |
| `test1-lumbridge`, `test1-home-ready` | Existing Lumb control triggered teleport animation, then Test at tile 3222,3222 with the Lumbridge courtyard and corresponding minimap. The final UI log shows scene 1 then scene 2, but no screenshot captures the intervening scene-1 freeze. |
| `test2-distinct`, `test1-distinct-return` | Switching from Test's Lumbridge courtyard to Test2 restored Test2's character-design modal and tutorial minimap at 3094,3106. Switching back restored Test's courtyard and Lumbridge minimap at 3222,3222. These distinguish account pixels and show no cross-account scene/minimap bleed in this observed round trip. |

`general-config` and `close-config` additionally document the existing capture
input setting and available controls; they are not independent performance proof.

## Gate scope and next action

This supplies observed focus restoration between distinct account scenes, a
none-to-GPU visual round trip, CPU/GPU selection restoration, and scene-2 modal,
minimap and chrome composition. It does **not** establish 40/50 fps, background
1 fps, skip-paint simulation cadence, attach/detach counter values, exact client
identity continuity, resource release, leakage, or scaling. Slot-up/login log
absence is corroboration only, not a substitute for identity/lifecycle counters.

G1 remains unproven: a post-rebuild scene 2 screenshot and a scene-1 log line do
not show the last FBO or minimap during loading. G4 composition during that
freeze is also pending. The bounded burst helper task `t_0e21a98c` is separate,
requires same-card review and native validation, and has not been run here.
The full G2/G3 contracts retain their cadence/identity/counter limitations above.

The focused candidate's RSS claim was separately parked after the approved
longer screen. These visual diagnostics do not reverse that decision. Fine/input,
calibration, target budgets, lifecycle/scaling and final whole-branch review
remain campaign gates. No additional clean resource comparison was run.
