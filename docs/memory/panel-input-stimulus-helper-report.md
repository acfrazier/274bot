# Guarded Game Image input stimulus helper

## Scope

`windows-visual-proof-tools/invoke-panel-input-stimulus.ps1` is a source-only external event source for the reviewed `focused-one` panel prerequisite. It sends ordinary alternating Left/Right arrow pulses through the focused panel window; it does not click panel chrome, change vault settings, focus a window, launch or terminate a panel, invoke global hotkeys, or claim host acknowledgement.

The source route is confirmed by the current panel code:

- `crates/panel/src/app.rs` maps `Key::LeftArrow` to GameShell `ch=1` and `Key::RightArrow` to `ch=2`.
- `capture_keys` emits key down/up edges for the hovered Game Image.
- `crates/panel/src/session.rs::stream_capture_for` sends those key events only through the capture sender and calls `note_panel_input_start` for actionable key-down edges. A missing sender is a no-op.
- The client key-code path maps `ArrowLeft`/`ArrowRight` to Java codes 37/39 and held camera state values 1/2. These are camera-only arrow inputs and do not substitute gameplay clicks or account actions.

## Required invocation contract

Every invocation must explicitly provide:

- panel PID and exact UTC process start string;
- absolute `panel-play.exe` path matching `C:\ProgramData\274bot-Test\<stage>\panel-play.exe`;
- exact expected SHA256;
- expected `BotTest` user and local Windows session ID;
- four-coordinate physical window rectangle;
- two-coordinate physical Game Image point, supplied by root after screenshot verification;
- unique output label and an output directory below `C:\Users\BotTest\274bot-runs\visual-proof-*` or `C:\Users\BotTest\274bot-runs\latency-diagnostic-*`;
- explicit bounded duration (`1..900` seconds).

Root must pass `-CaptureEnabledVerified` and `-SlotZeroFocusVerified` only after verifying those facts from the actual panel UI. The helper cannot prove either fact externally.

## Guards and event protocol

Before moving the cursor and before every pulse, the helper verifies x64 Windows PowerShell 5.1, local-console status, process path, PID, start time, binary hash, expected session, same-session ownership, foreground HWND, non-minimized state, HWND owner PID, and unchanged physical window bounds. It rechecks those identity/hash/geometry guards immediately before each key-down, and checks that the supplied point is inside the current physical window rectangle; the point's Game Image membership remains a root screenshot proof, not a guessed default. `SetProcessDPIAware` is called before any physical rectangle or cursor operation.

The C# interop factory assigns the complete keyboard value type before placing it into the 40-byte `INPUT` union. It validates the type, virtual key, flags, and marshaled size before `SendInput`. Arrow flags are `KEYEVENTF_EXTENDEDKEY` (`1`) for key-down and `KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP` (`3`) for key-up.

The fixed schedule is one pulse every 1000 ms with an 80 ms hold and alternates Left/Right; cadence and hold are not caller-overridable. Duration is the explicit number of requested pulse slots. Scheduling uses a monotonic `Stopwatch`, records each slot's due time and lateness, and enforces a duration-plus-5-second wall-clock bound. A slot delayed by a full cadence is marked deferred/missed and the helper stops rather than issuing catch-up bursts. Any successfully injected key-down receives a release attempt in the per-pulse `finally`; a failed release remains tracked for a second bounded outer-`finally` release attempt. A missing or failed release makes the result non-success.

The receipt records requested/down/up timestamps, VK, flags, `SendInput` return values, identity, window geometry, completed/missed/deferred slots, terminal reason, and failure text. Existing labels are refused, failure receipts are retained, and success requires every requested pulse plus every release result to succeed. The receipt explicitly sets `performanceAcceptance=false`, `inputCoveragePass=false`, and `hostAcknowledgement='not inferred from external stimulus'`.

## Verification boundary

This helper is not capture proof, latency proof, or a host-acknowledgement proof. A successful helper result only establishes that the guarded external key source completed its requested schedule. Root must separately verify capture-enabled state, slot-zero focus, and the resulting host cohort start/bind/present records. Only those host records can establish panel input coverage or fixed-window latency accounting. If the source/UI prerequisite is absent, preserve the failure receipt and report the missing capability; do not replace it with gameplay clicks.

## Worker verification

The new files are intentionally limited to this helper and this report. No native Windows action, window input, SSH operation, capture run, Rust edit, existing helper edit, raw artifact edit, or `STATE.md` edit was performed by this worker. PowerShell runtime validation and the actual screenshot-verified Game Image smoke remain root-owned because this workspace is macOS and the task forbids native execution by the worker.
