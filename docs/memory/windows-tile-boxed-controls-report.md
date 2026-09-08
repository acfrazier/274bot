# Windows native boxed-tile controls report

Scope

This deliverable prepares native Windows paired controls only. It does not launch the frontend, start a native build, collect measurements, or make performance-acceptance claims. The clean comparison deliberately leaves owner census, allocation/stack/debug diagnostics, and navigation PNG capture disabled.

Frozen roles

- Baseline/reference: host 9268890217d968cfeb7c66ebb11dd5c3dd2c084f, client abb811bd0afa1acd99319ccd5bc36bfb241080f9, binary e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5, stage C:\ProgramData\274bot-Test\renderer-owner-census-9268890, host source aggregate 84054d9c02394d959cd84681dd85f3589d3a559b94bb37856c6f87cb0629269a (871 files), client source aggregate ea6402702a56dc29359bf4effd9ca87407734fc9d940bfabdefb92cec8bf41ee (183 files).
- Candidate: host fb3589ac28583242b999ac864ea69c4ef8fa5923, client fd956c91bf09e059359c8e182a33583e2c626cd3, binary a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f, stage C:\ProgramData\274bot-Test\tile-boxed-fb3589a, host source aggregate 189dac149beb6f258f5a79bdf09de43818556a5f06ffa783a608f0113a64d8a4 (876 files), client source aggregate fad2e79248c8d62cff2ceba5c757ef555705e85d3dc6797586c9841d9dbfb53e (183 files).

Controls

The controller emits fresh per-cell manifests for both roles and both modes. It binds role, mode, binary, source manifests, server identity, nav/cache/catalog fixtures, Intel(R) Graphics requested adapter, and the 30/120/60 warmup/observation/teardown settings. The launcher uses a unique BotTest Interactive Limited scheduled task and refuses an existing cell output. Preflight requires the builder VM Off with zero assigned RAM, no builds/frontends/VM overlap, local console, server identity, quiet services, and saved display evidence. The contract runner is no-launch and separates privileged preflight from BotTest contract checks. The archive copies managed output, every receipt-referenced raw run directory, launcher log, and a hashed manifest.

Declared order

Root should invoke fresh, manually supervised cells in the predeclared order: background baseline, candidate, candidate, baseline; then focused-one baseline, candidate. No automated multi-launch or retries are encoded. The root owns one exact launch and all measurements later; these controls do not perform them.

Exact commands

From the host checkout:

- python3 docs/memory/windows-tile-boxed-controls/test_tile_boxed_controls.py
- powershell -NoProfile -File docs/memory/windows-tile-boxed-controls/validate-tile-boxed-ast.ps1 -ControlDirectory docs/memory/windows-tile-boxed-controls
- On the Windows host, first run `preflight-tile-boxed.ps1 -CellId <fresh role-mode id>` as the privileged host gate, then run `run-contractcheck-tile-probe.ps1 -BuildRole baseline|candidate -Mode focused-plus-background|focused-one -CellId <same fresh role-mode id>` as BotTest Interactive Limited, and finally run `prepare-tile-probe.ps1 -BuildRole baseline|candidate -Mode focused-plus-background|focused-one -CellId <same fresh role-mode id>` as the privileged host gate to consume that no-launch receipt. The contract uses a distinct `-contractcheck` cell namespace and never writes the launch cell output.
- For a supervised cell, launch-tile-boxed.ps1 -BuildRole baseline|candidate -Mode focused-plus-background|focused-one -CellId <fresh role-mode id>; poll-tile-boxed.ps1 -CellId <id> [-Final]; archive-tile-boxed.ps1 -CellId <id> -Destination <archive-root>.

Verification status

Local Python tests exercise the controller's actual argv/environment helper seams, role/identity bindings, no-navigation clean settings, package-local no-launch wiring, and archive raw-run inclusion. PowerShell parser validation and the native preflight/contract execution remain Windows/root-owned gates; no native action was taken here.
