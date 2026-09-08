# Native synchronized rebuild capture — 0830

The synchronized Windows capture contains a directly inspected scene2 → scene1
→ scene2 transition with the previous Lumbridge scene, minimap and chat retained
in two scene1 captures, followed by Varrock rendering. This establishes the
stated qualitative freeze and recovery observation. It does **not** close the
full G1/G4 checklist: no loading splash was visible in those frames, and no main
modal or script overlay was exercised during the freeze. Splash expectations
are being audited separately in `t_8c2b78ce`; do not declare either a source bug
or a full pass from this observation alone.

## Native execution and failure preservation

Frozen `a9b581ba…` panel ran as BotTest in local console session2, PID8748,
start `2026-09-08T08:33:38.6938556Z`, using `--live null_raster` and its two
temporary local test accounts. Harness printed PASS. Both started at Lumbridge.
Root inspected the actual Teles popup, then supplied Varrock coordinates
(1760,693) relative to physical window bounds [228,228,2326,1154]. No personal
vault or production service was used. Builder was Off with zero assigned RAM.

The first controller at08:35:11 refused the non-foreground target before starting
its capture child or sending input. Its failed receipt and task result1 are
preserved. A new uniquely labelled wrapper verified and focused the same target,
captured `pre-controller-focused`, then called the unchanged reviewed controller.
No model/tool-call timing race was used between capture readiness and input.

Controller1c694d8 was approved by actual Grok4.5/xai-oauth session
`20260908_043001_cd3914`, t_bfaf7292. Native source hash84bc1a2d… and unchanged
burst helper a6b3632 hashc09639cf… were verified when staged. Earlier native
no-input PS5.1 AST/C#factory proof is `windows-synchronized-input-interop-1c694d8.json`.

The successful controller ran08:35:58.1176957–08:36:22.0275339UTC. Capture child
PID2432 exited0 with retained handle. It captured300 frames over23.2987042s,
08:35:58.6489738–08:36:21.9476780UTC. Ready was08:35:58.7394604UTC; mouse-down
08:35:58.8604922 and up08:35:58.9629220, each SendInput returned1. Both events
are inside the accepted burst and after its verified fresh first frame. This
proves synchronization, not focused-input p99 or physical presentation cadence.

## Direct image inspection

Root inspected the original full PNGs using view_image, not just OCR output:
initial scene2, Teles popup, pre-controller focused popup, burst32/33/34/35,
and after-varrock. OCR of all300 frames was only a selector; its derived JSONL
is outside the native manifest. Root did not visually inspect every burst frame.

- Frame32: status scene2, Test at3222,3222; Lumbridge courtyard, minimap, chat.
- Frames33 and34: status scene1, destination tile3213,3423; same Lumbridge
  viewport including rat/player teleport pose and held minimap remain visible.
  Chat and panel chrome remain. No black scene/minimap hole or loading text is
  visible in these two captures. This is qualitative image observation, not
  pixel equality or proof of every intermediate GPU frame.
- Frame35: status scene2 at3213,3423, Varrock scene and minimap replace Lumbridge.
- After-varrock08:36:57: Test stands at Varrock; NPC/player positions differ
  from frame35, supporting resumed visible scene updates rather than persistent
  freeze. Two bots remain running. No modal was opened during this transition.

The panel was normally closed; retained process handle yields exit0 at
08:37:26.7143078UTC. Final08:37:51 receipt confirms no panel/TUI frontends,
all visual tasks terminal, builderOff/0RAM. Controller child exit and panel exit
are separate receipts, not inferred from scheduler success.

## Integrity and acceptance boundary

Local raw directory: `diagnostics/windows-visual-proof-nullraster-20260908-0830`.
Burst archive263669573bytes SHAa3a681659c3d39cce6e95a66a7dda2d498c013b822179267e1845ce1df386f2f.
Session archive4062709bytes SHAa7eccff42365a382ba7a9f082a68f5c0d6f220d7a60715983f2d31ff0210c82f.
Manifest SHA5a3ba4fe3bc1d67fc6a86ba3957b1e0395cb645cb26b0265adcd7b8025a7eb01.
Root verified all638 union manifest files by length/SHA and all300 frame PNG
hashes. Detailed timing, image hashes and source review are in the companion
`windows-nullraster-synchronized-0830-receipt.json`. Original0800 failed
synchronization and0830 foreground refusal remain intact.

No RSS saving, latency p99, GPU cadence/scanout, renderer identity counters,
full G1–G4, lifecycle, scaling or final campaign acceptance follows. The next
visual decision is the bounded loading-splash source audit; the current full
finish-line gap audit continues to govern remaining performance work.
