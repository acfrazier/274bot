Direct Heaptrack capture seam report

Scope

The reviewed Python-only N1 TUI capture seam is bounded and fail-closed. No native frontend, server, client, remote, or live capture was run by this task.

Lifecycle semantics

- Ordinary diagnostics retain max_wall_s=960 and emit no capture-only timing fields.
- Capture diagnostics declare live_max_wall_s=960, analysis_windows_s=[180, 180], and max_wall_s=1320.
- run_diagnostic enforces the 960-second frontend bound independently. After frontend exit, the existing bounded analysis helper owns the two sequential 180-second analysis windows; the managed cell total bound is 1320 seconds.
- A frontend timeout is terminated, escalated to kill after 15 seconds, and reaped before post-exit handling.
- The normal non-capture child wait path is unchanged.

Verification

- Focused Python suites: 100 tests executed; one pre-existing/flaky memory-guard test failed when run concurrently with the broader suite, then passed when isolated with the new timing test.
- Isolated timing tests passed for ordinary 960-second parity, capture-only 960/180+180/1320 fields, and the 960-second frontend bound.
- py_compile passed for the affected Python modules.
- git diff --check passed.

Remaining native smoke

Native direct Heaptrack smoke artifacts and installed tool facts are available under diagnostics/heaptrack-direct-smoke-1857 for root review. This task did not launch or rerun native capture and makes no performance or RSS-win claim.
