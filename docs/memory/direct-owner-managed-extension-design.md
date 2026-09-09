# Direct-owner extension of the existing managed N1 controller

Status: design only for task `t_59c6d6f5`. This document does not implement,
install, release, or run the direct-owner capture. It does not admit an account,
cache, server, host, binary, or live result. Independent review of this design is
required before implementation.

## 1. Decision

Extend the existing path

`run_current_tui_calibration.py` -> `run_managed_cell.py` ->
`run_diagnostic.py` -> `tui-play`

with one explicit `direct-owner-v1` mode. Do not create another controller or a
generic resource-runner abstraction. The mode is valid only for real-PTY TUI,
N=1, active Thiever, `--no-diagnostics`, and `--sustain`. It reuses the reviewed
N1 account-selection/seed behavior, normal observe-end `script_stop`, sixty-second
teardown, and owned-tree cleanup. The mode is mutually exclusive with Heaptrack,
snapshot-dedup, headless execution, input probes, navigation captures, stack
logging, scheduling/responsiveness/render probes, CPU fallback, and every N other
than one.

Absence of `--direct-owner-capture` preserves the current 120/600/60 timing,
128 MiB controller memory guard, 960-second managed ceiling (or the existing
Heaptrack windows), feature rule, collector stop rule, metadata path, and receipt
shape. New spec fields are rejected unless the explicit mode is selected. Both
the controller and launcher scrub an inherited `BOT_MEMORY_OWNER_CAPTURE`; only
the explicit mode sets it to `1` for `tui-play`.

The direct mode contract is:

| Item | Exact contract |
|---|---|
| frontend / population / workload | real 120x40 PTY TUI / N=1 / active |
| build features | exact canonical string `memory-profile-no-alloc,memory-owner-capture` |
| excluded build feature | `snapshot-dedup` absent and `features.snapshot_dedup` false |
| allocator | `std::alloc::System`; allocation counting false |
| runtime switch | `BOT_MEMORY_OWNER_CAPTURE=1`, emitted by `run_diagnostic.py`, not inherited |
| warmup / observation / teardown | 30 / 120 / 60 seconds |
| guard cadence | 0.5 seconds on an anchored monotonic schedule |
| outer handoff/startup deadline | 5 seconds from managed launcher `Popen` return; not RSS grace after frontend spawn |
| runtime available-memory floor | 256 MiB (`268435456` bytes) |
| frontend RSS ceiling | 512 MiB (`536870912` bytes) |
| aggregate owned-output ceiling | 64 MiB (`67108864` bytes) |
| frontend wall ceiling | 360 seconds from the conservative frontend spawn origin |
| outer launcher emergency ceiling | 365 seconds from launcher creation (5 + 360) |
| attempts | exactly one; no scene, protocol, or guard retry |

The Rust owner observer retains its separate hard budgets: 256 KiB owner JSONL,
256 KiB scalar mailbox/staging/output rows, 512 KiB COW scratch, 262144 visits,
128 rows per fragment, and cooperative five-millisecond fragment failure budget.
Those instrumentation budgets are not replacements for the supervisor limits.

## 2. Why the extension belongs at these seams

`run_current_tui_calibration.py` already owns source/server/cache inputs, N1
selection, environment cleaning, timing/spec assembly, one-attempt invocation,
and the final controller result. It is therefore the only public entry point that
gets a new mode switch.

`run_managed_cell.py` already reserves the cell, launches the diagnostic,
captures launcher/frontend PID ownership, runs the process collector, enforces an
outer monotonic deadline, and performs identity-checked cleanup. The multi-limit
runtime guard belongs in its existing launch/collect loop. A controller-side
thread cannot safely enforce frontend RSS or output ownership because it lacks
the validated frontend PID/start identity and canonical cell/run paths. The old
controller `MemoryGuard` remains unchanged for the default mode and is not run in
parallel with the direct-mode guard.

`run_diagnostic.py` is the only layer that knows the instant at which it calls
`Popen` for `tui-play` and the run directory it gives to the Rust harness. It must
emit the early identity handoff and explicitly set the runtime flag. It must not
enforce the supervisor limits itself.

`build_provenance.py` remains the build/fixture verifier. A small direct-owner
validator wraps its existing checks and adds exact source-lineage and feature
requirements. The ordinary verifier keeps its current behavior.

`managed_receipt.py` already binds immutable launch inputs and raw run files. In
direct mode it additionally binds `samples.owners.jsonl`, the guard JSONL/summary,
and the early handoff. No owner file is required on ordinary cells.

## 3. Source and binary admission

### 3.1 Immutable source lineage

The production source input remains the 1124-file archive
`frozen-direct-owner-source.tar.gz`, SHA-256
`2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d`,
with source-manifest SHA-256
`ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e`.
Its provenance labels remain:

- original host `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`;
- original client `3456edc8dabf7b25ada78110ffa56327af9f67a4`;
- reviewed host `3cdc3e4fbeb960fe4c270eb994ce22746787eb2d`;
- reviewed client `5c73a4a27f3d72834c2c2a071668eb197eb39fd9`.

Those are provenance identities, not the HEADs of a diagnostic checkout. The
separately reviewed cfg(test)-only TUI overlay is generated-test evidence only.
It is not in the production archive and must not appear in the production source
digest or binary build tree.

Root has materialized the archive as clean Git trees with distinct diagnostic
identities:

- host `dcdbeebf36665a1d07156402a5f64c823769e00d`, branch
  `codex/direct-owner-host-archive`;
- client `b74dfb3c998b10371b263189774b055ecf880380`.

`materialization.json` records that those commits were created after the earlier
`8e400e2d...` build. They must never be used to relabel that binary.

### 3.2 Fresh compile-time binding

Root subsequently produced a fresh native release build while the dcdbeebf host
and b74dfb3 client HEADs were present and clean throughout compilation. Its raw
receipt records:

- exact offline/locked owner feature pair;
- host source digest
  `b70608d10f203657cbe731c6fd5402d09c96c1053466ce6540a7ab0dacc62564`
  before and after;
- client source digest
  `169ba594a834fb5ea81f392ae77909251cc46889efa2324525fff71794a7ea30`
  before and after;
- binary SHA-256
  `392dbecc7a86f2fcd7e3f6b515aedceb3f3de3b1d4e140b95463bf20438c0c95`,
  95595424 bytes;
- unchanged 1124 source members, modes, and all three lockfiles.

This resolves the prospective compile-time Git-identity ambiguity, but it is raw
build evidence, not live admission. Root must still issue and review a normal
runtime build manifest and combine the new build with the already-reviewed
native generated coverage.

The previous binary
`8e400e2d622af2c00280bfb91c741753e0da3df1736bb7d6fe7d0f8251394194`
remains immutable generated-qualification evidence. Direct mode rejects a
manifest that pairs it with dcdbeebf/b74dfb3.

### 3.3 Exact `build_provenance` representation

The root-issued manifest uses existing schema fields as follows:

- `candidate.commit` is the compile-time host HEAD dcdbeebf...;
- `candidate.branch` is `codex/direct-owner-host-archive`;
- `candidate.build_exit` is integer zero;
- `candidate.sources_sha256_pre` and `_post` are both b70608d1...;
- `candidate.sources_stable_across_build` is true because the two measured
  digests and clean-tree receipts agree, not as a free-standing admission;
- `candidate.client.commit` is b74dfb3...;
- legacy `candidate.client.sources_sha256` is 169ba594...;
- `features.requested` is exactly
  `memory-profile-no-alloc,memory-owner-capture` (a string, not a list);
- `features.locked` is true, `features.allocation_counting` is false,
  `features.allocator` is `std::alloc::System`, and
  `features.snapshot_dedup` is false;
- `binaries.candidate_tui_play.path` is the root-selected immutable runtime copy
  of the new binary and `.sha256` is 392dbecc...;
- existing `nav` and `catalog` path/hash bindings remain required.

Add `candidate.source_lineage` rather than overloading any commit field. It holds
paths plus SHA-256 values for the frozen archive, frozen manifest,
`materialization.json`, and the fresh build result; source-member count 1124;
the four original/reviewed provenance IDs; and the two diagnostic checkout IDs.
The direct verifier hashes those files and checks their internal values. It does
not accept a summary field such as `source_admitted: true` or infer lineage from a
matching binary hash.

`verify_direct_owner_build` first calls the unchanged `verify_build`, then checks
the exact feature object and lineage. Its returned file bindings include every
lineage artifact so `recheck_files` catches mutation at completion.

Direct controller source admission is conjunctive:

1. `check_source` requires a clean host/client checkout at the diagnostic commit
   IDs from the verified manifest; free CLI overrides do not substitute IDs.
2. The existing source-digest algorithm is shared as a module-level helper and
   recomputed for host and client before launch. Results must equal the manifest.
3. Manifest candidate/client commits, materialization receipt commits, and live
   checkout HEADs must all agree.
4. Archive, source manifest, materialization, build receipt, runtime binary,
   nav, flags, catalog, and lock bindings must pass.
5. Host/client cleanliness, HEADs, and source digests are rechecked after the
   managed cell. Drift makes the result failed while preserving output.

There is no blanket dirty-tree exemption. Runtime output is placed outside the
source checkout so a successful run does not itself dirty the admitted tree.

## 4. Direct-mode spec, argv, and environment

The spec gains a single object only when selected:

`capture_contract.mode = "direct-owner-v1"` plus exact numeric values from the
table above, expected run directory, early-handoff path, owned output roots, and
source-lineage bindings. `validate_spec` rejects partial objects, unknown values,
non-exact timing, `heaptrack != null`, any non-system process backend, or any
inconsistency with top-level `n`, timing, `sampler_interval_s`, `max_wall_s`,
frontend, build role, and diagnostic argv.

The direct diagnostic argv is exactly the existing N1 resource-only argv plus
`--direct-owner-capture`, `--warmup 30`, `--observe 120`, an explicit fresh
`--run-dir PATH`, and `--frontend-handoff PATH`. The handoff argument is the
child-visible channel: its canonical value must equal
`capture_contract.frontend_handoff_path`, which in turn must be exactly
`cell_dir/frontend-handoff.json`; `--run-dir` must canonically equal the reserved
`capture_contract.run_dir`. It retains `--no-diagnostics --sustain` and has no
Heaptrack or other probe flag. `run_diagnostic.validate_args` rejects either path
flag outside direct mode and rejects direct mode without both.
`run_managed_cell.require_argv_consistent_with_spec` compares all of those values,
including the two canonical reserved paths, against the spec before launch.
Neither an environment variable nor a spec-only declaration can turn the mode
on or redirect either output path.

`run_current_tui_calibration.clean_environment` scrubs the owner variable for all
calls and sets it only in direct mode. `run_diagnostic.build_child_env` scrubs it
again, then emits `BOT_MEMORY_OWNER_CAPTURE=1` only after CLI validation. The
launcher metadata records the selected mode and the exact child environment
contract, while omitting private account identity.

The Rust teardown remains its existing fixed 60 seconds. The spec's
`teardown_grace_s=60` records and validates that existing behavior; it does not
introduce a competing teardown timer or delay Stop.

## 5. Output reservation and ownership

All direct-mode output is rooted under one root-selected, new output parent on a
single filesystem. Before launch, the managed runner exclusively reserves:

1. the controller result and spec paths;
2. only this cell's `cell_dir`, not the whole shared `cells_root`;
3. the exact frontend `run_dir` passed by `--run-dir`;
4. the handoff slot, its one fixed-name atomic-update temporary
   `cell_dir/frontend-handoff.next.json`, and guard files inside `cell_dir`.

`run_diagnostic` consumes the pre-reserved empty run directory in direct mode;
its timestamp-generated default remains unchanged otherwise. Any run-directory
value in stdout metadata or the early handoff that differs canonically from the
spec is a failure.

The output scanner starts immediately after launcher creation, before frontend
metadata. It walks only the four declared owned paths. Every root and parent is
resolved and checked before launch. During scans it uses `lstat`, never follows a
symlink, rejects a symlink/special file or a path-component replacement, and
deduplicates regular files by `(st_dev, st_ino)`. The only permitted owned leaf
replacement is the validated atomic `spawned` -> `exited` handoff transition in
section 7.3; its fixed-name temporary is counted while present, and every other
inode replacement fails. Nested roots and hard links are therefore counted once.
A hard link is measured but never followed outside an owned root. A declared or
reported path outside the owned roots fails; the runner does not search,
truncate, chmod, or delete unrelated files.

The aggregate is the sum of `st_size` for unique owned regular files, including
launcher/cell logs, process accounting, guard evidence, metadata, samples,
qualification, `samples.owners.jsonl`, and controller receipts. The scanner
records a per-root/per-file breakdown sufficient to reproduce its total. Files
that disappear, become unreadable, or mutate type during a scan make that sample
failed rather than being treated as zero.

Supervisor-authored guard/final receipts are bounded and reserve completion
space. If a child write takes observed output beyond 64 MiB, files are preserved
as found; the runner records the observed total and starts owned cleanup. It does
not truncate evidence to manufacture compliance.

## 6. Fresh preflight

`--preflight-only` keeps its no-launch/no-output-reservation semantics and prints
the proposed direct contract. A real invocation repeats all checks inside the
managed preflight after reserving its own receipt paths and before `Popen`.

Direct preflight requires raw measurements, not a root-provided `healthy: true`:

- Linux x86_64 and `process_backend="system"`;
- `/proc/meminfo` parsed in bytes with MemAvailable at least 768 MiB;
- no swap use: raw SwapTotal/SwapFree and vmstat swap counters are recorded, with
  zero used swap and no counter movement across the bounded preflight interval;
- cgroup/host OOM source, boot/cgroup identity, and before/after OOM counters are
  recorded and unchanged during that interval;
- `disk_usage` for the common owned-output filesystem reports at least 256 MiB
  free after reservation;
- a root-generated conflict receipt identifies the exact process classes checked
  (build, test, profiler, TUI/panel frontend), observation interval, and every
  matching PID/start identity/executable basename without argv or environment;
  any live conflicting match fails, and the controller never signals it;
- the declared server sidecar and public artifacts pass existing hash checks;
- the server PID/start identity is sampled at both ends of preflight, is not a
  zombie, and matches a fresh root health probe for loopback port 43594;
- server RSS/CPU samples remain a distinct server series. They are never added to
  frontend RSS and are not subject to the 512 MiB frontend limit;
- existing cache snapshot/version/file hashes, canonical unpack root, nav/flags,
  catalog, source, build, and binary checks all pass.

Root supplies private account, population, cache, and server admission receipts.
The controller validates their identities without copying account name,
password, vault content, or cache payload into public output. A stale or missing
receipt fails before launch. The exact freshness window is recorded by the root
release contract; implementation must not invent one if root did not bind it.

## 7. Frontend spawn and lifecycle handoff

Direct mode adds one stateful `frontend-handoff.json` slot under `cell_dir`.
`run_managed_cell` passes that exact path through the explicit
`--frontend-handoff` argument and passes the pre-reserved run directory through
`--run-dir`; both canonical paths are in the validated spec. There is no implicit
environment or current-directory path channel.

### 7.1 Stop-safe spawn registration

This mode is Linux-only, so `run_diagnostic` closes the otherwise unowned interval
inside `Popen` with POSIX signal masking rather than a second launcher. Before
spawn it installs direct-mode SIGTERM/SIGINT handlers, snapshots its existing
direct-child PID/start identities, and blocks those signals with
`signal.pthread_sigmask`, retaining the returned pre-block mask and the previous
signal dispositions. Admission requires SIGTERM and SIGINT to be unblocked in
that pre-block mask; otherwise it fails before spawn because soft cleanup could
not be proved. It records `spawn_before_monotonic_s` immediately before `Popen`.

The direct-mode POSIX `preexec_fn` composes the existing PTY session setup with
child-only signal restoration. While SIGTERM/SIGINT are still blocked, the child
restores their pre-install dispositions; as the final step before returning from
`preexec_fn`, it calls `signal.pthread_sigmask(signal.SIG_SETMASK,
pre_block_mask)`. Thus the frontend exec inherits the launcher's original
unblocked mask, not the parent's temporary registration mask. Any child-side
restoration error fails `Popen` and enters the partial-spawn accounting below.
The child restoration changes only the forked child's mask and dispositions: the
parent keeps the direct-mode handlers and SIGTERM/SIGINT blocked through durable
registration and handoff.

On return `run_diagnostic` assigns the `Popen` handle, records
`spawn_after_monotonic_s`, samples the new child's Linux start identity, and puts
the handle/PID/identity into the cleanup state. It keeps the signals blocked until
the initial handoff in section 7.2 is durable, then restores the old mask. A stop
delivered during `Popen` therefore remains pending until the exact owned handle
is registered and its spawn evidence is preserved; the handler records the
request and the normal identity-checked cleanup path reaps that child. It never
signals from a guessed PID.

If `Popen` raises before a handle is assigned, the launcher performs one bounded
comparison against the pre-spawn direct-child snapshot while the signals remain
blocked. It may clean only a unique new direct child whose executable is the
admitted binary, whose parent remains this launcher, and whose Linux field-22
start tick converts through `SC_CLK_TCK` to the monotonic boot-time spawn bracket
(allowing one scheduler tick of measurement tolerance). Zero or ambiguous matches
produce a spawn-failure receipt with explicit `orphan_risk`; no candidate is
signaled. The old signal mask is restored in a `finally` path after this
receipt/cleanup attempt. A handoff-write or other failure
after handle assignment uses that registered handle and identity while the stop
signals are still blocked, preserves its failure receipt, and only then restores
the mask. These rules cover a pending stop, a `Popen` exception after partial OS
creation, and an ordinary post-spawn exception without changing default-mode
signal behavior.

### 7.2 Spawn state and first RSS sample

Before ordinary metadata work or any other fallible post-spawn operation,
`run_diagnostic` exclusive-writes, flushes, and fsyncs the handoff in state
`spawned`, then restores the signal mask as specified above. It contains:

- schema/mode/state and a random per-launch nonce generated before `Popen`;
- frontend PID and the launcher's sampled frontend start identity;
- launcher PID;
- exact run directory;
- the two monotonic spawn brackets;
- frontend/TTY/N/workload/timing values.

The managed runner accepts either the `spawned` state or a later `exited` state
that repeats the complete immutable spawn object. It requires a regular
non-symlink file at the reserved path and its actual launcher. It records the
first nonce and requires that value unchanged for every later state. It requires the
exact reserved run directory, finite ordered brackets inside the runner's
launcher-start/receive envelope, and an allowed PID distinct from
server/controller/ambient/launcher/collector. For `spawned`, it requires a live
direct child. It independently samples the frontend PID and requires the sampled
start identity to equal the launcher's handoff identity before storing that
identity as the guard key. A handoff that first appears as `exited` cannot qualify
unless the managed runner had already obtained a valid identity-bound RSS sample
for that same spawn object.

The five-second deadline is only an outer bound from the managed launcher's
`Popen` return through the diagnostic launcher's pre-spawn setup. It does not
permit five seconds of a live frontend without RSS enforcement. The first valid
identity-bound frontend RSS sample must finish no later than
`spawn_before_monotonic_s + 0.5`; a handoff received after that point fails
immediately. Subsequent samples use the same absolute half-second grid anchored
at `spawn_before_monotonic_s`. Thus a slow `Popen`, late handoff, or slow first
sample cannot qualify by merely arriving inside the five-second outer window.

Before frontend spawn, MemAvailable, output, and outer-launcher wall are measured
but frontend RSS is inapplicable. The receipt calls this `pre_frontend_spawn`, not
guard coverage. Once `spawn_before_monotonic_s` is recorded, any interval beyond
the first 0.5-second sample deadline is a guard failure. Missing/malformed outer
handoff fails at five seconds and cleans the owned launcher. The stop-safe spawn
registration above prevents a spawned-but-unreported frontend from being silently
abandoned; if ownership cannot be proven, the receipt reports orphan risk and
does not signal a guessed PID.

### 7.3 Identity-bound exit state

After `child.wait()` returns, and before reader/PTY teardown, final metadata,
provenance recheck, analysis, or any other post-exit operation,
`run_diagnostic` records `wait_return_monotonic_s` and the exact exit code. It
exclusive-creates and fsyncs `cell_dir/frontend-handoff.next.json`, then atomically
replaces the handoff slot with state `exited` and fsyncs `cell_dir`. That state
repeats byte-for-byte the immutable spawn object and adds the stored frontend
start identity, exit code, and wait return time. The per-launch nonce, PID,
identity, paths, and spawn brackets cannot change. Keeping the complete spawn
object means an atomic replacement cannot erase evidence if the frontend exits
before the managed reader sees the first state, although such a run still fails
the required first RSS sample.

The `exited` state is evidence that this launcher waited on its registered child;
it is not an RSS sample and does not claim when between the last sample and wait
return the process exited. Final metadata must repeat the same spawn/exit tuple
and exit code. A stale file, nonce change, non-atomic/truncated state, changed
immutable field, or final-metadata disagreement is a lifecycle failure.

## 8. Runtime guard in the managed loop

Starting with the first sample required by section 7.2, the existing managed loop
runs a direct guard sample on the absolute 0.5-second monotonic grid until the
identity-bound exit state is accepted and the launcher completes, or a breach
begins cleanup. Each row is appended and flushed to
`direct-owner-guard.jsonl` and contains scheduled/start/end monotonic times,
lateness/acquisition duration, frontend PID/start identity/current RSS,
MemAvailable, output total/breakdown digest, elapsed frontend wall, and any
failure reason.

At every grid point:

1. parse MemAvailable from `/proc/meminfo`; missing/malformed data fails;
2. sample the exact frontend PID through `server_resources.sample_process`;
3. require its start identity to equal the handoff identity and its parent to
   remain the owned launcher while that launcher is alive;
4. use `resident_bytes` from Linux `/proc/PID/stat` field 24 pages multiplied by
   `SC_PAGE_SIZE`; KiB or `ru_maxrss` is never substituted;
5. scan all owned output paths using the rules above;
6. compare elapsed time to the conservative frontend spawn origin.

No previous value is reused. An identity change/PID reuse, parent mismatch,
acquisition error, skipped grid point, or sample whose end is already beyond its
next scheduled point is a guard failure. A missing process sample starts only a
bounded `exit_pending` state; it is neither a zero-RSS value nor immediate proof
of failure. The managed runner continues the MemAvailable/output/outer-wall
checks and rereads the handoff until the earlier of the next absolute grid point
or 0.5 seconds after the missing sample began. It accepts normal exit only if an
atomic `exited` state arrives in that interval, exactly matches the previously
validated nonce/PID/start identity/spawn object, reports `child.wait()` return no
later than receipt of the state, and the same owned launcher subsequently exits
with final metadata and the declared exit code. If the proof is absent, late,
malformed, mismatched, or followed by launcher/final-metadata disagreement, the
original missing sample is an unexplained disappearance and the run fails.

Once the exit state is accepted, frontend RSS enforcement stops because the
identity-bound child has been waited and no longer exists. The last valid RSS row
is retained; no RSS value is imputed after it. The receipt records the terminal
unsampled interval from that row's end through `wait_return_monotonic_s` and does
not call it continuous or hard-limit coverage. MemAvailable and aggregate output
continue on the anchored grid until launcher completion. Frontend wall is frozen
at `wait_return_monotonic_s - spawn_before_monotonic_s` and is checked against
360 seconds when the exit state is accepted; the separate 365-second outer
launcher emergency ceiling still bounds finalization. A launcher exit without
the matching exit state is never accepted, even if its return code is zero.

The first observed value satisfying any of these conditions triggers failure and
owned cleanup:

- MemAvailable `< 268435456`;
- frontend RSS `> 536870912`;
- aggregate owned output `> 67108864`;
- frontend elapsed wall `> 360.0` seconds.

The guard writes one terminal summary with threshold, last good row, breach row,
observed overshoot, cleanup request time, and final post-cleanup output scan. The
summary does not say an unsampled threshold was never crossed.

These are sampled supervisor ceilings, not kernel-enforced hard limits. Temporal
detection overshoot is bounded by the recorded 0.5-second schedule plus actual
acquisition lateness; the implementation fails if it misses that schedule.
Magnitude overshoot in RSS or output between samples is not bounded because a
process can allocate or write before the next observation. The receipt reports
that limitation and the observed overshoot. The owner JSONL/internal observer
caps listed in section 1 remain hard checked-write/budget limits.

In direct mode the ordinary process-accounting collector is not stopped two
intervals after `observe-end`; it remains alive through the normal sixty-second
teardown and is stopped/reaped only after launcher completion or guard cleanup.
This preserves the separately measured server/controller/launcher series during
A/B/Stop/C. The default mode retains its existing two-interval stop behavior.
Frontend RSS enforcement comes from the identity-bound guard series, not from a
retroactively edited collector role map.

## 9. Stop, cleanup, and completion

A healthy run receives no supervisor signal. The existing Rust path writes
`observe-end`, calls `play.script_stop` normally, records Stop begin/end, enters
teardown, requests owner C at teardown+30 seconds, and exits after the unchanged
sixty-second teardown. The supervisor does not call script Stop, extend teardown,
or wait for a missing owner reply past the existing lifecycle.

On guard, protocol, launcher, or sampling failure, reuse `_terminate_owned` and
`_cleanup_frontend`. The direct diagnostic's stop-safe spawn registration from
section 7.1 is the only additional pre-handoff cleanup seam. Signals are limited
to the `Popen`-owned collector and launcher and the captured frontend after
immediate PID/start-identity and parent checks. The game server, SSH parent, root
helpers, and conflicting processes are never cleanup targets. Before each
escalation, identity is rechecked. Foreign, ambiguous, or reused PIDs produce an
unresolved cleanup receipt with orphan risk, not a signal.

Every path preserves one-attempt evidence:

- preflight failure: reserved cell report plus raw preflight values,
  `launched=false`, attempts one at the managed-cell level;
- handoff/startup failure: launcher log, guard startup rows, launch receipt, and
  identity-safe cleanup result;
- runtime breach: all partial run files, guard rows/summary, process accounting,
  exact breach and cleanup receipts;
- protocol/workload failure: raw owner/qualification rows and all resource data;
- cleanup failure: partial files plus explicit orphan risk.

`managed_receipt.complete` hashes `samples.owners.jsonl` in direct mode and binds
its bytes before validation. The controller calls
`validate_direct_owner_capture.validate_file` with that recorded SHA-256. A
completed controller result requires all of the following without changing the
validator's honest `native_qualified=false` and `rss_reconciliation=false`:

- generic managed receipt completed;
- no preflight or guard failure;
- exact source/build/cache/server bindings unchanged;
- owner protocol returns `protocol_complete=true` for A/B/C and Stop order;
- workload qualification and per-slot progress pass;
- launcher/frontend/collector exit and owned-descendant cleanup are complete;
- final output scan is within the sampled ceiling.

The result remains `performance_acceptance=false`. It does not emit
`live_qualified=true`; root decides live qualification only after independent
review of the complete raw package.

## 10. Narrow implementation paths

Only these existing Python paths need implementation changes:

1. `docs/memory/run_current_tui_calibration.py`
   - explicit mode flag, exact direct timings, source/build/preflight wiring,
     owned output/run-dir assembly, default guard preservation, post-run source
     recheck, and owner validator result;
2. `docs/memory/run_managed_cell.py`
   - strict direct spec/argv path binding, spawn/exit handoff intake, first-sample
     deadline, identity-bound 0.5-second guard and exit-pending state, output
     scanner, direct collector lifetime, cleanup/failure receipts;
3. `docs/memory/run_diagnostic.py`
   - direct CLI validation including explicit handoff/run paths, owner env
     scrub/set, monotonic Popen bracket, signal-masked spawn registration,
     spawn/exit handoff states, and pre-handoff owned cleanup;
4. `docs/memory/build_provenance.py`
   - shared source digest and `verify_direct_owner_build` wrapper with lineage
     file bindings; ordinary verification unchanged;
5. `docs/memory/managed_receipt.py`
   - conditional handoff/guard/owner raw hashes and completion bindings;
6. focused tests in
   `docs/memory/test_current_tui_calibration.py`,
   `test_run_managed_cell.py`, `test_run_diagnostic.py`,
   `test_build_provenance.py`, and `test_managed_receipt.py`.

`docs/memory/validate_direct_owner_capture.py` is reused without a broad rewrite.
A focused test may be added if controller invocation exposes a validator bug, but
its protocol scope does not expand. No Rust production file, generic new runner,
SSH helper, STATE file, account/cache/server artifact, or release script is part
of this controller-extension implementation.

## 11. Generated regressions and failure injection

All controller tests use temporary files and generated dummy processes. They do
not use a real account, server, cache, PTY login, or network fixture.

### Default/control path

- Snapshot the existing N16 argv/env/spec and N1 default behavior with the mode
  absent; assert 120/600/60, 128 MiB guard, 960 seconds, existing collector pad,
  and no owner handoff/raw-file requirement.
- Exercise the existing Heaptrack path and prove direct mode is rejected with it.
- Seed inherited `BOT_MEMORY_OWNER_CAPTURE=1`; prove ordinary and invalid calls
  scrub it and direct mode alone re-emits it.

### Contract/source/build

- Accept only N1 TUI active, real PTY, sustain/no-diagnostics, exact 30/120/60,
  0.5-second cadence, and canonical owner feature string.
- Reject feature list form, missing owner feature, snapshot-dedup, counting
  allocator, heaptrack, N16, headless, probe flags, and env/spec/argv disagreement.
- Independently vary the spec handoff/run paths, `--frontend-handoff`, `--run-dir`,
  and inherited environment; reject every mismatch or path outside the reserved
  `cell_dir`/run root before launching.
- Reject original/reviewed H/C IDs used as derivative HEADs, dirty source, source
  digest drift, archive/manifest/materialization/build-receipt drift, client HEAD
  mismatch, nav/catalog mismatch, and runtime binary mutation.
- Accept a generated manifest shaped like the dcdbeebf/b74dfb3 fresh build and
  reject the same manifest if its binary is replaced by 8e400e2d....
- Mutate source after preflight and prove completion becomes failed without
  deleting the run.

### Preflight

- Inject exact-boundary and one-byte-below MemAvailable and free-disk values.
- Inject missing/malformed meminfo, active swap/counter movement, OOM counter
  movement, stale health receipt, server PID reuse/zombie, server artifact drift,
  and each conflict class; assert no launcher call.
- Prove the separately sampled server survives every failure and is never included
  in the frontend RSS limit.

### Handoff and process ownership

- Delay handoff to just below and just above five seconds; inject missing,
  truncated, duplicate, symlink, wrong-run-dir, wrong-launcher, forbidden PID,
  parent mismatch, and invalid monotonic brackets.
- Allow slow pre-spawn launcher setup inside five seconds, then require the first
  valid RSS sample by `spawn_before+0.5`; inject Popen/handoff/sample delays on
  both sides of that deadline and prove the outer window never grants RSS grace.
- Reuse a PID between handoff and first sample and between soft/hard cleanup;
  assert no signal reaches the unrelated process.
- Spawn a frontend before suppressing the handoff; prove the preinstalled launcher
  cleanup reaps it, or records explicit orphan risk if identity cannot be proven.
- Deliver SIGTERM while a fake/direct `Popen` is in progress; prove it remains
  pending until the returned handle and start identity are registered, then only
  that child is cleaned. Raise after partial spawn with zero, one, and multiple
  new direct-child candidates; clean only the unique executable/parent/identity
  match and otherwise preserve `orphan_risk` without signaling a guessed PID.
- In generated Linux subprocess fixtures, inspect `/proc/<pid>/status` and prove
  the real child does not inherit the parent's temporary SIGTERM/SIGINT block.
  Send SIGTERM and SIGINT in separate cases after spawn and prove each child
  responds promptly without the hard-kill stage. Inject an already-blocked
  pre-block mask and prove admission fails before `Popen`.
- Hold the parent inside `Popen`, deliver a pending parent SIGTERM/SIGINT in
  separate cases, and prove the child-side mask restoration does not unblock the
  parent: only after durable PID/start-identity handoff may the parent handler run
  and clean that registered child. Also inject child restoration failure and
  require the partial-spawn cleanup/`orphan_risk` receipt rather than a guessed
  signal.
- Make the first/any later frontend sample absent, malformed, slow, stale, or
  identity-changing; assert one failure and no retry.
- Exit the frontend normally while `run_diagnostic` is still closing PTY/readers,
  checking provenance, and writing final metadata. Prove the matching atomic exit
  state permits that bounded finalization without another RSS sample or any
  supervisor signal, while MemAvailable/output/outer-wall checks continue. Then
  remove, delay, truncate, replay, or identity-mismatch the exit state and prove a
  missing PID remains a failure even when launcher/final metadata later look
  successful.

### Limits and receipts

- Drive each threshold at equal, one byte/epsilon over, and a large jump over:
  256 MiB available floor, 512 MiB frontend RSS, 64 MiB output, 360-second wall.
- Use a fake monotonic clock to prove the wall origin is the pre-`Popen` frontend
  bracket and the grid remains anchored rather than sleep-relative.
- Create nested owned roots and hard links; prove byte totals are deduplicated.
  Add symlink escapes, special files, path-component replacement, unreadable
  files, disappearing files, and a metadata run directory outside owned roots;
  prove failure without reading, modifying, or deleting the external target.
- Force a single large child write between scans; prove the receipt reports
  sampled magnitude overshoot rather than claiming a hard 64 MiB guarantee.
- Fail receipt and final-result writes near the output cap; prove earlier partial
  guard/launch evidence remains and no existing path is overwritten.

### Lifecycle and Stop

- Generated launcher rows prove the direct collector continues after
  `observe-end`, through Stop/C/launcher exit, while the default collector still
  stops after its two-interval pad.
- Feed valid A/B/Stop/C owner rows and independently inject missing/duplicate/stale
  phase, wrong frame/slot, cap breach, malformed terminal row, Stop reversal, and
  C-before-Stop; assert raw hashes remain and success is refused.
- Simulate normal completion and prove no supervisor signal occurs, Stop timing is
  unchanged, teardown remains sixty seconds, exit proof precedes final metadata,
  and cleanup reports no descendants.
- Inject a guard breach during warmup, observation, and teardown; prove only the
  owned tree is stopped, server/ambient/unrelated processes remain alive, all
  cases report attempts one, and no second scene/N is launched.

## 12. Release boundary

Passing these generated tests would establish only that the existing controller
can enforce and receipt the designed direct-owner procedure. It would not admit
the fresh binary, combined generated qualification, private fixture, current
server, cache, host conditions, or a live launch. Root still owns those bindings,
the one-attempt release, result review, candidate decision, and final whole-branch
Grok 4.6 review.
