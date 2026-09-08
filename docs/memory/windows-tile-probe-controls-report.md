# Windows tile probe controls report

Status: prepared only. No Windows build, server, VM, frontend, launch, poll, or archive was executed by this task.

Purpose and boundary

These controls prepare exactly one fresh native Windows N16 focused-plus-background diagnostic for TILE LAYOUT/OCCUPANCY observations. They do not change representation, rendering policy, scheduler behavior, performance thresholds, or acceptance gates. `performanceAcceptance` and `representation_change` are explicitly false. The earlier failed cell remains untouched; this package does not retry it.

Frozen inputs

- Host: commit `9268890217d968cfeb7c66ebb11dd5c3dd2c084f`; 871 files; aggregate `84054d9c02394d959cd84681dd85f3589d3a559b94bb37856c6f87cb0629269a`.
- Client: commit `abb811bd0afa1acd99319ccd5bc36bfb241080f9`; 183 files; source digest `ea6402702a56dc29359bf4effd9ca87407734fc9d940bfabdefb92cec8bf41ee`.
- Native binary: SHA-256 `e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5`.
- Stage: `C:\ProgramData\274bot-Test\renderer-owner-census-9268890`.
- Native build evidence: `docs/memory/diagnostics/windows-tile-probe-build-9268890/`.
- Client archive: `/private/tmp/274bot-windows-setup-20260907/client-abb811b.tar.gz`, SHA-256 `c8eac9ccd06c46a0beca420f6fc30ccd2e4b23237c1ea9894ea743b73fd834ab`.

`windows-tile-probe-native-freeze.json` records all identities and hashes. Its independent manifest check counted 871 total source-manifest entries and 183 `vendor/fr-client-rust/` entries. The six retained native build evidence files are individually length/SHA recorded there; source-pre and source-post are identical, and the build receipt reports exit code 0 with `cargo build --locked --release -p panel --bin panel-play --features memory-profile-no-alloc`.

Controls

- `prepare-tile-probe.ps1`: no-launch gate. Runs the real PowerShell AST parser, native preflight, Python producer tests, and `check-tile-probe-contract.py`. The latter is adapted from the previously executed BotTest hook: it runs the actual controller with `run_managed_cell.main` replaced by a no-launch function that loads the emitted spec, invokes the real spec validator, diagnostic argv parser/consistency checks, `managed.preflight` against actual files/environment/server, and the actual native binding-condition reader. It uses a distinct `...-contractcheck-tile` ID and explicitly proves no client and no scheduled task were started.
- `preflight-tile-probe.ps1`: requires the frozen stage, builder VM Off/0 assigned memory, no build/frontend/VM overlap, quiet services stopped, active unlocked BotTest console, AC/display/brightness/power/DxDiag evidence, Intel Vulkan route evidence, and fresh server PID/name/creation identity on port 43594. It writes a per-cell preflight receipt.
- `launch-tile-probe.ps1`: requires the per-cell preflight and frozen stage, rejects existing output/task names, and launches one bounded scheduled task with an 18-minute execution limit.
- `poll-tile-probe.ps1`: receipt-based completion/startup reader. `-Final` requires completion exit 0 and an emitted startup record whose requested and selected adapter are both `Intel(R) Graphics`.
- `archive-tile-probe.ps1`: refuses an existing destination, copies the managed run, requires every `receipt.json` external `run_dir`, copies those raw runs under `raw-run/`, and writes length/SHA entries for every archived file.
- `run-panel-tile-probe-focused-one.py` and its wrapper preserve `BOT_RENDER_OWNER_CENSUS=1`, `BOT_RENDER_PROFILE=1`, full scheduler/latency/responsiveness/fine-latency/GPU-completion/failure-capture flags, the existing server/cache/catalog/nav fixture, and the 16-bot focused-plus-background shape at 1 background FPS.
- `test_tile_probe_controls.py`: four executable local producer tests exercise the actual managed spec validator, diagnostic parser/required argv validator, build verifier, native binding-condition reader, and negative missing-field/stale-identity cases. It uses `RENDER_OWNER_CENSUS_HOST_ROOT` rather than a stale absolute checkout path.

Exact root execution sequence

1. Keep the existing native tools root `C:\Users\BotTest\274bot-workspaces\3118e96\host`, Python tools, server, cache/catalog/nav fixture, and strict reader unchanged. Do not build or regenerate them.
2. Confirm no old task/cell is running and preserve all old stages and archives.
3. Transfer only this directory and `windows-tile-probe-native-freeze.json` to the BotTest context.
4. Choose one externally supplied fresh ID matching `native-render-owner-census-focused-plus-background-[A-Za-z0-9_.-]+`, distinct from `...-0150`.
5. Run `prepare-tile-probe.ps1 -CellId <fresh-id> -HostRoot C:\Users\BotTest\274bot-workspaces\3118e96\host`; require all checks and no-launch output to pass.
6. Run `launch-tile-probe.ps1 -CellId <fresh-id>` once.
7. Poll the same ID with `poll-tile-probe.ps1 -CellId <fresh-id>` until its receipt-based completion is available; do not restart on a timeout. Run `-Final` only after completion.
8. Run `archive-tile-probe.ps1 -CellId <fresh-id> -Destination <new-archive-root>` once. Verify the generated archive manifest and `raw-run-sources.json`, then preserve the raw and managed artifacts.
9. Report process exit, workload qualification, binding evidence, and tile occupancy observations separately. Do not turn this diagnostic into a performance or representation claim.
