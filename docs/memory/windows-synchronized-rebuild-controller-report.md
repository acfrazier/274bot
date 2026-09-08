# Synchronized native rebuild capture controller

Status: source implementation complete; native Windows AST, compile, and BotTest execution remain root-owned validation gates.

## Scope and source inputs

This change adds only:

- `windows-visual-proof-tools/invoke-rebuild-capture.ps1`
- this report

The existing `windows-visual-proof-tools/capture-panel-burst.ps1` is not modified. The controller declares and verifies its expected SHA-256 (`c09639cf3f94dcdadb276eb9a4b0256c81e4b988207b8d748d636c9e9b7ba181`) before launching it. The frozen panel target remains `C:\ProgramData\274bot-Test\tile-boxed-fb3589a\panel-play.exe`, SHA-256 `a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f`.

## Controller contract

The entry point requires explicit `PanelPid`, exact `ExpectedStartUtc`, an existing dedicated `OutputDirectory`, unique `Label`, `InputX`, `InputY`, and four explicit `ExpectedRect` integers. It refuses non-BotTest execution, output paths outside `C:\Users\BotTest\274bot-runs\visual-proof-`, existing controller/output labels, missing or changed helper source, wrong target executable/hash/start/session, minimized or non-foreground windows, foreign HWND ownership, and rect mismatches. It checks the supplied window-relative input is inside the freshly verified window rectangle; it does not guess a target or coordinate.

All Win32 input interop is loaded before the child capture starts. The controller launches the unchanged helper as a hidden `powershell.exe` child with the reviewed 30-second/300-frame bounds, retains the child process handle, and confirms the child is in the target's Windows session. It waits in one bounded controller for a unique ready marker and unique first-frame receipt produced after controller start while the child remains alive. The first frame must match label, frame number, panel PID/start, frozen binary/hash, HWND, rectangle, and the SHA-256 of its PNG on disk. Its capture and ready timestamps must be ordered and must follow controller start.

Immediately before cursor movement and again immediately before button events, the controller checks panel PID/start/session, frozen binary path, foreground HWND, non-minimized state, HWND owner, and unchanged rectangle. It moves the cursor only to the supplied window-relative point, verifies the actual screen cursor identity, then sends exactly one ordinary left-button down/up pair. The press is bounded to approximately 80 ms, and the release is attempted in `finally` even if the down event fails. No `SetForegroundWindow`, global hotkey, or focus-stealing operation is used.

The controller waits for the helper's terminal receipt within a 60-second total bound, records the retained child's exit code, and persists redirected stdout/stderr. On timeout or failure it stops only the controller-owned child after rechecking its PID/start identity, waits for its observed exit, and never terminates the panel or unrelated processes. A pre-existing receipt is never overwritten; a receipt is written in `finally` only after this invocation has claimed a fresh label. Partial helper frames/receipts are preserved. The receipt includes an explicit chronology section proving both click events fall inside the completed, identity-matched burst bounds, records both `SendInput` return values, and keeps `sceneState` as `not inferred from capture` and `performanceAcceptance` false; a synchronized click and completed burst are not a G1/FBO/scene/FPS acceptance claim.

## Verification performed in this checkout

- Re-read the reviewed helper and its report before authoring the controller.
- Recomputed the helper SHA-256 locally and embedded that exact value.
- `git diff --check` is required before commit for both new files.
- Static source checks are required before commit for the fixed helper path/hash, explicit input/rect parameters, Win32 interop declarations, hidden child launch, retained handle, unique ready/first-frame wait, identity/rect guards, one down/up pair, bounded waits, release `finally`, structured receipt, and explicit non-inference fields.

PowerShell 5.1 is not installed in this macOS workspace, so native `ParseFile`/AST, C# compilation, and BotTest execution are deliberately not claimed here. Root must run those checks and the native controller after same-card review using a freshly inspected screenshot's explicit coordinates and rectangle. A successful native run still requires direct inspection of the resulting images before any G1 conclusion.
