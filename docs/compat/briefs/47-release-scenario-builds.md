# Separate release and scenario build capabilities

The operator requests another Grok architecture task: segregate what belongs in
release binaries versus scenario builds, building on the memory-profile feature
idea. This is design work alongside brief 46. Use grok46 profile defaults, read
applicable AGENTS.md and docs/execution.md, and verify the current campaign
branch. Own only docs/harness-integration/06-build-capabilities.md; commit the
report and complete. No source edits, LIVE, remote work or implementation cards.

Inspect the actual Cargo features and dependency paths in the root workspace,
panel, tui, host-play, scenario, script, e2e and the client interface as needed.
At dispatch, panel/TUI depend unconditionally on scenario; host-play makes it
optional under memory-profile. Verify this and trace real use sites, including
ordinary UI commands, --live, BOT_* controls, account/server fixture preparation,
PNG capture, tracing, allocator instrumentation, examples and ignored tests.
Read docs/harness.md, brief 46 and completed reports 01-05 when available. Do not
scan frozen exports or unrelated old worktrees. Record the exact source used.

Give a concrete capability/dependency matrix for ordinary release, scenario,
and memory builds, and explain how the latter two compose. Distinguish Cargo
features from optimization profiles and runtime options. Recommend the smallest
useful arrangement; do not assume a new flag or a duplicate executable is the
answer before examining existing seams. Assess Cargo feature unification,
accidental dependency activation, default/--all-features test coverage, feature
forwarding into FR, and whether crate boundaries need a narrow adjustment.

Classify each capability by user value: normal gameplay, profile validation,
build identity, support diagnostics and manual capture may belong in release;
fixture cheats, automated proof runners, synthetic accounts, benchmark fleets,
counting allocators and expensive traces may require explicit opt-in. These are
questions to resolve from source and operator intent, not predetermined gates.
Do not make a shell helper mandatory to use a shipped diagnostic capability.
Name disabled-command behavior and compatibility with existing scripted runs.

Scenario builds must exercise the same production script, protocol, navigation,
state publication, lifecycle and rendering paths. Show how to prove that the
feature split does not create a different product or silently remove a runtime
check. Keep ingame/scene-2 qualification, exact resource identity and failures.
Account for root's 6c6bb2d5 terminal-shot/full-bank-loop harness changes and
814e5293 startup preparation under corrective source review.

Deliver a bounded staged plan with crate/file ownership, migration order,
concrete build commands and a risk-proportional verification matrix for macOS,
Windows and Linux artifacts. Include clear artifact naming/provenance and which
variants should actually ship. Identify potential size/startup/dependency wins
as hypotheses unless measured; do not run a benchmark or claim savings. Resolve
how this design fits the shared capture/controller/platform proposals, include
tradeoffs and a stopping point. This report becomes an input to synthesis card
t_14b7a7e3; it is not authorization to change the current release configuration.
