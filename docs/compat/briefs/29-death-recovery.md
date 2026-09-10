# Death observation and required recovery

Use configured `sol` defaults after PeriodicBank review. Campaign branch:
codex/rs2b0t-multirevision. Read applicable instructions, docs/execution.md,
fail-closed-dispatch, plan step 6 and DeathRecovery in
04-provisioning-recovery-design.md. Verify actual enabled frozen callers.

Implement the Rust-owned death latch/recovery operation behind the existing
thin DeathRecovery ABI. Observe a new game-chat death line; seeded flags,
duplicate snapshots or old chat must not re-trigger it. Preserve configured
anchor/radius and script callbacks onDeath/onRecovered. WildyAgility's supplied
walkBack callback is required and retains its script-owned food-bank/ridge
sequence. No enabled caller requires needs/AcquireTask; keep that explicit
unsupported boundary and do not create a foreign recovery planner.

Preserve the 20000 ms respawn observation bound and subsequent three actual
game ticks, then perform the existing host walk to anchor or supplied walkBack.
Report recovery from actual position/callback completion. onRecovered fires
once for the observed event. Inspect the client/session behavior of actual
death/respawn so ordinary death remains recoverable while logout/reconnect,
Stop and isolate replacement discard old work. Pause/Guardian hold freezes
deadlines and sends. Retain existing 60000 ms walk and bank bounds.

Allowed: script shim/runtime/service/wire, minimal host/API observation plumbing
and focused tests. Reuse posted ordered chat and existing operations. No client
change, nav router rewrite, frontend, external engine, operator settings, LIVE,
gate, main, remotes or gitlink.

Composed tests must traverse the script ABI through Rust, ordered new/duplicate
death chat, respawn/tick wait, actual return outcome, once-only callbacks and
WildyAgility walkBack. Exercise timeout, Pause/Guardian, Stop/session abort and
stale results. Do not count queued movement or a mocked initial anchor as the
positive recovery outcome. Use existing tests/Clippy proportionately.

Deliver 04-capabilities-recovery.md and raw checks in
evidence/recovery-capabilities/. Commit only scoped files, request same-card
profile `reviewer` review, then stop. No agents or live qualification claim.
