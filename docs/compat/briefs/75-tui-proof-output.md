# Keep live proof output from corrupting an active TUI

Use grok46 defaults. Own only crates/tui/src/bin.rs and narrowly necessary TUI
reporting glue/tests, plus docs/compat/06k-tui-proof-output.md and unique
evidence/tui-proof-output/. Other workers own runtime, API and scenario files.

Root actual Concord B1/client9d PTY proof shows a PASS: live alcher JSON line
printed to stdout while Ratatui's alternate screen is active. This leaves
persistent fragments across script paint/chat rows even after later renders.
The terminal is real: input/output at140x40 with production tui::bin::main
and a four-line private IsolatedEnv entry only. Raw ANSI and interpreted screens
under evidence/platform-isolated/concord-tui-b1cff8a7/controls show running,
paused1/2, resumed and stopped states. Wrapper redirects BOT_DEBUG stderr to
its own file; the PASS stdout line alone still corrupts the active display.
Full raw stdout/stderr will be collected by root after the run ends.

Make the smallest coherent TUI-owned correction: preserve machine-readable
PASS/FAIL text and exit status, including FAIL + exit1, while reporting at a
safe point relative to the active terminal. Headless/no-TTY behavior stays
correct. Preserve every scenario predicate, deadline/soak, Start/Pause/Stop,
input ownership and product behavior. Do not remove proof output or fabricate
success, suppress errors, change logger credentials/config, route script policy
into the TUI, or implement the separate harness architecture project.
Inspect the actual output/terminal cleanup ownership first. Consider all early
return/error paths and avoid duplicate/lost output. A bounded TUI report buffer
flushed after terminal restoration is one option, not a mandate.

Use exact committed host/client export and a fresh isolated target for checks;
do not build against concurrent shared runtime WIP. The regular Git-blob helper
in evidence/build-isolation/export_git_files.py avoids archive extraction.
Run relevant existing TUI/controller tests and strict affected Clippy; add a
focused regression only if it catches output/cleanup behavior, not an exact
implementation mirror. Commit only owned source/report/evidence and request
SAME-card profile reviewer with exact identity and checks, then STOP. Root owns
actual TUI rerun and whole-branch acceptance; no worker LIVE or remote actions.
