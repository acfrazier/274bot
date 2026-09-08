# Native panel capture burst helper

Status: implementation complete; native Windows execution and G1 acceptance remain pending.

## Files

- `windows-visual-proof-tools/capture-panel-burst.ps1` is the new bounded PowerShell 5/System.Drawing/Win32 helper.
- This report is the source-grounded verification record. The approved single-capture helper and Rust/runtime/control/STATE files were not changed.

## Contract implemented

The script requires the existing BotTest interactive session, the exact frozen `panel-play.exe` path, expected process start UTC, same Windows session, and the approved SHA-256 binary hash before capture. It requires an already-existing output directory under `C:\Users\BotTest\274bot-runs\visual-proof-`; it never creates that directory or overwrites a burst label, ready marker, frame PNG, or frame receipt.

`DurationSeconds`, `MaxFrames`, and `DelayMilliseconds` are validated to 1..30 seconds, 1..300 frames, and 10..1000 ms respectively. The C# Win32 type and `System.Drawing` assembly are initialized once per invocation. Each frame captures only the target window rectangle with `Graphics.CopyFromScreen`, then disposes its `Graphics` and `Bitmap` in `finally`; no image collection is retained in memory. The binary is hashed once before the loop.

Before every frame, and again after the pixel copy, the helper checks the target process identity, start UTC, session, HWND ownership, foreground status, non-minimized state, and exact stable window rectangle. Any change aborts the burst. Each successful frame gets a sequential PNG plus timestamped JSON receipt containing PID, start UTC, session, binary identity/hash, HWND, rectangle, capture timestamp, PNG hash, and explicit non-inference fields. A `<label>.ready` marker is written only after frame 1 and therefore gives the root operator a point at which to click the already-visible Lumbridge control independently.

The `<label>.burst.json` receipt records requested bounds, actual capture start/end, elapsed duration, frame count, frame receipt paths/hashes, terminal outcome, and failure text when applicable. Partial successful outputs are preserved. The helper deliberately records `sceneState` as `not inferred from capture` and `performanceAcceptance` as false: filenames, delay, and frame count do not prove G1, FPS, scanout, or scene-state behavior.

## Native gates still pending

A native PowerShell 5.1 AST/compile check and an actual BotTest run on the frozen candidate remain required. The root operator must independently verify that the target is the intended visible panel, trigger the existing UI control after the ready marker, inspect the captured images, and decide G1 (`scene_state==1` last-FBO freeze) from visual evidence. The helper cannot establish minimap retention, overlay compositing, scene state, scanout timing, or successful G1 on its own.

## Local verification

`git diff --check -- docs/memory/windows-visual-proof-tools/capture-panel-burst.ps1` passed. PowerShell is not installed in this macOS workspace, so native AST/compile and capture execution were not claimed here.
