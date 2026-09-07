# Candidate TUI N=16 stop forensics

## Scope and verdict

This is a read-only forensic report for candidate run
`docs/memory/diagnostics/20260907T005032Z_tui_n16_active`.
No behavior, fixture, timeout, catalog, server state, or measurement code was
changed. The run is a failed diagnostic cell, not an N=16 pass: it exited 1
with a script-requested stop during setup/observation, and only two N=1 cells
qualified.

The exact stop reason cannot be recovered from the retained artifacts. The
strongest supported statement is: slot `livefd010_5` requested a script stop at
tick 289; the host retained only the generic stop message, and the run did not
retain the optional failure diagnostics sidecar.

## Original failure evidence

Metadata (`metadata.json`) records:

- frontend `tui`, N=16, workload `active`
- warmup 30 s, requested observation 120 s
- `diagnostic_only: true`, `diagnostic_sidecar: false`, `debug: false`
- host commit `bd5dc1e28c8230d603746cb43c41fed7e957bf41`
- client commit `451759f2a7df9c57895657d5b8d506172860cee1`
- binary `tui-play-candidate-system-20260906T235815Z`
- process exit code 1

The qualification receipt (`samples.qualification.jsonl`) contains one
`observe-start` record at elapsed 119.708981 s. It is the only qualification
record, so there is no failure-boundary snapshot. For `livefd010_5`, that
record shows:

- state `Running`, error `null`
- runtime dispatched 82, last completed tick 102
- paint title `ThievingBot — Pickpocket Guard at (2664, 3302, 0)`
- position `(2665, 3302, 0)`
- Food 4, Steals 1, HP 84%
- `in_flight: null`
- inventory contains four Lobster entries/counts and Coins count 30
- bank is empty, `bank_open: false`, `bank_loaded: false`
- client `ingame: true`, `scene_state: 2`, position `(2665, 3302, 0)`

The PTY log is raw terminal output rather than a structured event receipt. The
retained failure message identifies the event as `script requested stop on tick
289; isolate stopping`; it does not identify the reason argument or the bot
state that caused it.
The receipt's last completed tick (102) is not evidence
that the stop happened at tick 102: it is a different, sparse observation of
runtime progress taken before the failure boundary. The PTY-reported stop tick
is 289; there is no retained runtime snapshot at that boundary.

## What the source proves

`crates/script/src/shim/script_runner.js:6-12` implements `ScriptRunner.stop`
as follows: it sets `globalThis.__rs2b0t_host.stopRequested = true`, and stores
`host().stopReason` only when the caller supplies a string. It does not emit a
log or forward the reason itself.

`crates/script/src/load.rs:2604-2620` checks only `stopRequested` after the
current tick, emits `script requested stop on tick {n}; isolate stopping`, and
breaks the isolate loop. That log format omits `stopReason`.

`crates/script/src/load.rs:1174-1179` drains isolate log lines into the host's
pending log collection. `crates/host-play/src/lib.rs:3288-3297` exposes
`memory_script_progress` and `script_last_error`, but there is no corresponding
public accessor for `stopReason`. Thus a reason may have existed transiently
inside the JS host object but is not present in the retained progress/error
fields unless the script itself logged it.

`crates/host-play/src/memory.rs:437-440` enables memory diagnostics only when
`BOT_MEMORY_DIAGNOSTICS=1`. The failure paths at lines 645-668 call
`write_diagnostics` only when that flag is enabled. The sidecar is created only
in that same mode at lines 507-515. The candidate metadata says both
`diagnostic_sidecar: false` and `debug: false`; therefore this run had neither
the failure-only slot snapshot nor debug log forwarding available as a retained
artifact.

`crates/host-play/src/memory.rs:1070-1078` shows the normal qualification
record shape: state, last error, runtime progress, and client location. The
runtime progress returned by `crates/script/src/load.rs:1122-1128` includes
paint and in-flight details; `crates/script/src/slot.rs:377-385` adds inventory,
bank, and bank-open/loaded state. The observe-start record therefore already
retains those fields under `runtime`. It does not include `stopReason`, and
this run has no failure-boundary qualification record. The failure-only
diagnostic shape at `crates/host-play/src/memory.rs:1081-1098` would additionally
include seed status and `memory_diagnostics::sample`, but was not produced for
this run.

## Stop-path candidates: evidence versus hypothesis

The checked navigation/stop review material identifies `ThievingBot.stopSafely`
as capable of stopping for bank reach/open, no food, withdraw, close-count, and
return conditions. Those are candidate paths only. The candidate run does not
retain the call argument, a stop-path label, or a failure-boundary state, so no
one of those paths can be selected as the cause.

The observe-start facts do not establish any of them. In particular, Food 4
and `Steals: 1` do not prove a no-food path; an empty, closed bank does not
prove a bank path; and the scene/position do not prove route ownership or a
return condition. No error is present. The correct conclusion is “reason
unknown,” not a reconstructed stop reason.

## Shared NavWorld and causality boundary

`docs/memory/shared-play-nav-world-report.md:33-61` describes the reviewed
candidate change: the memory frontend constructs one `Play`, then binds seed
runners from `play.world()` by cloning the existing `Arc<NavWorld>`. A missing
world remains `None` and does not trigger a second decode. The report explicitly
states that production `Play::new`/interactive unlock paths are unchanged and
that runner mutable state remains independent.

That establishes an ownership/identity change (one immutable world Arc shared
between Play and the seed runners), not a route/seed-policy change. It does not
prove that the shared Arc changed route results, random seeds, bot action
ordering, or the Stop decision in this failed run. The failed run's artifact has
no before/after route trace, seed-runner identity trace, or stop reason, so a
causal claim linking shared navigation to the stop would be speculation.

## Minimal next diagnostic action (not implemented)

Use one bounded failure-only diagnostic capture on a reproduction with the
same binary, nav pack, accounts/fixture, tick cadence, policy, timeout, and
action ordering. Enable the existing diagnostic sidecar only for that targeted
reproduction; do not enable permanent per-tick verbose logging in clean
measurements.

The smallest useful capture improvement is to make the failure-boundary record
include, for each slot that is stopping or failed:

1. the stop reason string, if present;
2. the current paint frame;
3. bank-open/loaded state and inventory;
4. the client position/scene state;
5. current in-flight request information; and
6. the last completed/dispatched tick values.

This should be a failure-only read of already-held state, emitted at the same
existing clean-failure branch that currently calls `write_diagnostics`; it must
not add movement, waits, retries, policy changes, stop behavior, or a new
periodic log stream. The stop reason needs to cross the existing isolate-to-host
boundary before the isolate drops, rather than being inferred later from paint
or inventory. If a targeted reproduction is not available, do not label this
run's likely path; retain it as an unresolved failed cell.

A targeted repro is preferable to changing the normal measurement receipt. The
candidate already demonstrates that the normal receipt is insufficient, while
an opt-in failure-only sidecar can preserve clean measurement semantics and
avoid per-tick overhead. The diagnostic run must remain excluded from RSS,
latency, and performance claims.

## Evidence limits

- No build, test, live rerun, server reset, or fixture mutation was performed.
- No exact `stopReason` was recovered.
- No N=16 workload qualification or performance claim is made.
- No navigation/seed causality claim is made.
- The clean screen/report owned by task `t_54b3b727` was not modified.
