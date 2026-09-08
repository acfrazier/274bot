# Bounded native TUI N1 attribution controller

Scope

`run_current_tui_calibration.py` now accepts an explicit bounded `--n` value of
`1` or `16`, defaulting to `16`. The selected value is carried through the
managed diagnostic CLI (`tui N active`), `BOT_MEMORY_N`, the generated spec
receipt, and the durable run receipt. No native run, build, SSH action, server
mutation, or new evidence claim was made by this change.

Preserved controls

- Warmup remains 120 s and observation remains 600 s.
- Active fixture/workload, frozen host/client provenance, no-alloc feature
  contract, profile-off environment, terminal/launcher path, memory guard,
  process identity checks, cleanup behavior, immutable output reservation, and
  preflight no-launch behavior are unchanged.
- `--n` rejects values outside `{1, 16}` during argument parsing, before input
  validation or launch.
- The default remains N16 so existing invocations retain their prior contract.
- The controller continues to mark performance acceptance false and does not
  claim a measured saving or qualification result.

Verification

`python3 -m unittest docs/memory/test_current_tui_calibration.py -v` passed:
23 tests. The tests cover N1 CLI/environment agreement, default N16 behavior,
invalid-N rejection before launch, and the existing guard/cleanup/no-launch
controls. `python3 -m py_compile docs/memory/run_current_tui_calibration.py
docs/memory/test_current_tui_calibration.py` passed.

Root-owned follow-up

After review, root owns the native preflight, the sole authorized N1 launch,
immutable archive/binding/recomputation, and any attribution interpretation.
This controller change is not performance acceptance and does not alter the
previous failed-guard proof or original evidence.
