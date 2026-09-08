# Windows native boxed-tile longer focused confirmation controls

Scope

This deliverable prepares the predeclared single longer N16 focused-one GPU confirmation only. It does not launch the frontend, start a native build, collect measurements, or make performance-acceptance claims. The control deliberately uses 30 s warmup, 600 s observation, and 60 s teardown grace; navigation captures, CPU fallback, census/counting, debug, and repeat loops are disabled.

Frozen roles

- Baseline/reference: host 9268890217d968cfeb7c66ebb11dd5c3dd2c084f, client abb811bd0afa1acd99319ccd5bc36bfb241080f9, binary e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5, stage C:\ProgramData\274bot-Test\renderer-owner-census-9268890.
- Candidate: host fb3589ac28583242b999ac864ea69c4ef8fa5923, client fd956c91bf09e059359c8e182a33583e2c626cd3, binary a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f, stage C:\ProgramData\274bot-Test\tile-boxed-fb3589a.

Controls

The controller emits a fresh per-cell manifest, server identity with complete generated server configuration hashes, host conditions, and spec for each role. It binds count 16, mode focused-one, requested GPU/Intel(R) Graphics, frozen source/binary/fixture identities, and the 30/600/60 timing. The longer controls use distinct TILE_BOXED_LONG_* environment names, managed-tile-boxed-long-* outputs, tile-boxed-long-* receipts, and 274bot-BotTest-tile-boxed-long-* scheduled-task names. Existing short controls and restored native runtime files remain untouched.

Privileged staging copies the actual controller into each existing role stage under a separate -long-contractcheck runner and verifies its hash. BotTest Interactive Limited runs a no-launch contract that parses the actual generated argv/spec and callback, asserts N16/focused-one/30/600/GPU/no-CPU/no-nav/no-census, verifies complete server configuration and native conditions, and writes a separate contract receipt. Prepare consumes that receipt as a privileged step; it never writes protected ProgramData as BotTest. PowerShell locals are initialized before AST validation. Preflight retains VM-off/zero-RAM, no build/frontend/VM overlap, local console, frozen hashes, server identity, quiet services, and saved display evidence.

Exact commands

From the host checkout:

- python3 docs/memory/windows-tile-boxed-long-controls/test_tile_boxed_long_controls.py
- powershell -NoProfile -File docs/memory/windows-tile-boxed-long-controls/validate-tile-boxed-ast.ps1 -ControlDirectory docs/memory/windows-tile-boxed-long-controls
- On Windows, run the AST validator and preflight-tile-boxed.ps1 -CellId <fresh baseline|candidate-focused-one id> as privileged gates; then stage-contract-tile-probe.ps1 -BuildRole baseline|candidate -Mode focused-one -CellId <same id> privileged; then run-contractcheck-tile-probe.ps1 -BuildRole baseline|candidate -Mode focused-one -CellId <same id> as BotTest Interactive Limited; finally prepare-tile-probe.ps1 -BuildRole baseline|candidate -Mode focused-one -CellId <same id> privileged.
- For a supervised cell, launch-tile-boxed.ps1 -BuildRole baseline|candidate -Mode focused-one -CellId <fresh id>; poll-tile-boxed.ps1 -CellId <id> [-Final]; archive-tile-boxed.ps1 -CellId <id> -Destination <archive-root>.

Declared order

Root should invoke exactly two fresh cells in predeclared baseline then candidate order. The controls do not automate the pair, add retries, or change profiling/calibration. A longer observation cannot establish missing p99/input/lifecycle evidence by itself.

Verification status

Local Python tests execute the controller's actual argv helper, parse it with the exact original native parser extracted from git revision 35eab6c^, evaluate the generated spec, exercise the real no-launch callback without launching, and cover negative duration/mode/CPU/census/malformed-digest/missing-configuration cases. PowerShell parser validation and native preflight/contract execution remain Windows/root-owned gates; no native action was taken here.
