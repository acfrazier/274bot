# Direct-owner controller lifecycle fidelity review

Verdict: APPROVE

Design commit: `5cb63749da03463fa3030a8a35d4d37e123fe4b0`
(`docs: restore direct-owner child signal mask`).
Blob `957670708dc042d08d613952178b223d0b039536` is identical at
`HEAD` `05918b08d6b34e76ec895a017af69e290194bf72`; `05918b0` only edited
`docs/memory/STATE.md`. This review inspected that exact design commit, not a
moving unrelated tree.

This is independent coherent design review after two substantive lifecycle
rejections (`0149711` round 1, `ff47094` round 2 mask hole closed in `5cb6374`).
It is not duplicate routine implementation review and not whole-branch review.

Scope limits: design-contract only. This approval cannot establish native
generated coverage, live readiness, performance acceptance, account/cache/server
admission, or a release. Root must reconcile before implementation. No
production/STATE/live/SSH/native/private-fixture work was performed.

## Method

Read once: `AGENTS.md`, `docs/execution.md`, plan
`direct-per-bot-owner-capture-plan.md` §§4–6, final
`direct-owner-managed-extension-design.md` at `5cb6374`, parent `t_59c6d6f5`
contract review, and the actual existing launch/cleanup/provenance code:

- `docs/memory/run_current_tui_calibration.py`
- `docs/memory/run_managed_cell.py`
- `docs/memory/run_diagnostic.py`
- `docs/memory/build_provenance.py`
- `docs/memory/server_resources.py`
- `docs/memory/managed_receipt.py`
- `docs/memory/process_accounting.py` (collector sibling only)
- `docs/memory/heaptrack_capture.py` (`child_preexec`; mutually exclusive)

Checked practical Python fork/exec/preexec/thread/signal behavior against that
path, not the design prose alone.

## Implementable in the existing controller?

Yes. The extension stays on
`run_current_tui_calibration.py` → `run_managed_cell.py` → `run_diagnostic.py`
→ `tui-play`. No new runner. Direct mode is an explicit `direct-owner-v1`
opt-in.

Default path is preserved by absence of `--direct-owner-capture`:

| Default (mode absent) | Source |
|---|---|
| 120 / 600 / 60 | `WARMUP_S`/`OBSERVE_S`/`TEARDOWN_GRACE_S` at calibration 37–39 |
| 128 MiB MemAvailable guard | `MEM_AVAILABLE_GUARD_BYTES` at 35; `MemoryGuard` 273–292, 315–338 |
| 960 s managed ceiling | `build_spec` `max_wall_s` 220 |
| 0.5 s sampler interval | `build_spec` 234 |
| Heaptrack windows unchanged | 221–223 |
| Collector stops two intervals after observe-end | managed 1104–1124, 1211–1218 |

Approved one-attempt 30/120/60 N1 diagnostic is mode-only, matching plan §5
(30/120/60, 0.5 s, preflight 768/256/no-swap/conflicts, runtime 256/512/64/360,
one attempt, normal Stop+60 s, owned-only cleanup). Direct mode does not run
`MemoryGuard` in parallel (design §2 vs calibration `run()` 315–327).

`validate_spec` today rejects unknown kinds/backends but does not require a
`capture_contract` object (managed 68–163). New fields can be rejected unless
the explicit mode is selected without changing ordinary cells.

## Prior rejected holes — corrected interaction

### 1. Handoff path channel (`0149711` R1)

Closed. Child-visible `--frontend-handoff` and `--run-dir` are required in
direct argv; both must canonically equal reserved
`cell_dir/frontend-handoff.json` and `capture_contract.run_dir`
(design §§4, 7). `require_argv_consistent_with_spec` (managed 198–228) already
compares binary/manifest/role/frontend/timing/launcher identity and is the
existing seam for those two canonical paths. Neither env nor spec-only
declaration can enable the mode. Current diagnostic parser has no those flags
(run_diagnostic 31–62); default timestamp `run.mkdir(..., exist_ok=False)` at
308/350 stays for non-direct.

### 2. Unregistered partial spawn / in-Popen stop (`0149711` R1)

Closed. Direct mode blocks SIGTERM/SIGINT around frontend `Popen`, registers
the returned handle/PID/start identity before unblocking, and on `Popen`
failure before a handle may clean only a unique new direct child whose
executable is the admitted binary, parent is this launcher, and start tick
converts into the spawn bracket (one `SC_CLK_TCK` tolerance). Zero or
ambiguous matches → `orphan_risk`, no guessed PID (design §7.1). Cleanup still
reuses `_terminate_owned` / `_cleanup_frontend` (managed 464–589), which
already recheck identity and parent and never signal the server.

### 3. Normal-exit / missing-PID race (`0149711` R1)

Closed. `run_diagnostic` currently writes metadata at 461, waits at 485, then
rewrites metadata at 524 after PTY/reader/provenance work — that is the race.
Design §7.3 writes identity-bound `exited` after `child.wait()` and before
reader/PTY teardown, final metadata, or provenance recheck. Guard treats a
missing process as `exit_pending`, not zero RSS; it accepts normal exit only
for an atomic `exited` state matching nonce/PID/start identity/spawn object
with `wait()` no later than receipt, then matching launcher/final metadata
(design §8). No imputed RSS after the last valid row.

### 4. First-sample blind window (`0149711` root / R2)

Closed. Five-second bound is only launcher `Popen` return → diagnostic
pre-spawn setup. First identity-bound RSS must finish by
`spawn_before_monotonic_s + 0.5`; later samples use that absolute 0.5 s grid.
`pre_frontend_spawn` is labeled inapplicable for RSS. The existing managed
loop already wakes every `min(0.05, interval/2)` (managed 1166), so the first
sample need not wait a full 0.5 s poll. A slow `Popen`/handoff/sample fails
closed rather than hiding behind the outer five seconds. Intentionally strict:
if `Popen` itself returns after `spawn_before+0.5`, the cell cannot qualify.
That matches root’s “0.5 s from spawn, not 5 s RSS grace” instruction.

### 5. Inherited blocked signals (`ff47094` / `5cb6374`)

Closed. Child `preexec_fn` restores dispositions while still blocked, then
`pthread_sigmask(SIG_SETMASK, pre_block_mask)` as the last preexec step.
Parent keeps handlers and the blocked mask through durable handoff. Admission
fails before spawn if the pre-block mask already blocks TERM/INT. Tests
require `/proc/<pid>/status` SigBlk, TERM/INT response without hard-kill,
parent pending-stop isolation, and restoration-failure → partial-spawn /
`orphan_risk` (design §§7.1, 11).

## Ownership trace against the actual launch path

### Launch / preexec / partial `Popen`

Unix TUI today (run_diagnostic 416–433):

```
terminal_session: os.setsid(); TIOCSCTTY
child_setup: terminal_session(); optional heaptrack child_preexec
Popen([binary], stdin/stdout/stderr=slave, preexec_fn=child_setup)
```

Direct mode composes signal restore onto that same `preexec_fn`. Heaptrack
`child_preexec` (heaptrack_capture 84–97) is mutually exclusive with direct
mode. Managed launcher `Popen` (982–988) has no `preexec_fn`; it execs
`python run_diagnostic.py`. Frontend preexec therefore runs only in the
fresh diagnostic process.

If preexec raises, CPython’s errpipe path waits for the child; design still
runs the unique-child scan while signals remain blocked. Matches partial-OS-
creation without changing default-mode signal behavior (handlers today are
installed after `Popen` at 470–473).

### Parent pending-stop and child mask restoration

`signal.pthread_sigmask` is per-thread and copied by fork. Blocking in the
parent, restoring the pre-block mask only in the child, then unblocking the
parent after durable handoff, is the correct POSIX split. A stop during
`Popen` stays pending in the parent; the child does not inherit the temporary
block across exec. Managed cleanup of a stuck launcher can still escalate to
SIGKILL (`_terminate_owned` 518–528), which cannot be blocked; an unreported
child becomes explicit `orphan_risk`.

### Durable spawn/exit handoff and argv binding

Current ownership channel is stdout JSON plus `metadata.json` after `Popen`
(460–462). Direct mode makes the reserved file the ownership slot, with
spawned → atomic `frontend-handoff.next.json` replace to exited. Managed
already identity-checks frontend PID/parent/forbidden set (592–605, 1072–1075).
Direct mode adds nonce, spawn brackets, reserved run-dir, and independent
start-identity resampling before the guard key is stored.

### First RSS, anchored sampling, no imputed RSS

`server_resources.parse_proc_stat` (33–49) maps split fields so `fields[21]`
is Linux field 24 RSS pages and `fields[19]` is field 22 starttime.
`_linux_sample` 124: `resident_bytes = rss_pages * SC_PAGE_SIZE`. Design
forbids KiB/`ru_maxrss` substitution. Guard rows are fresh samples; identity
change, skip, or overrunning the next grid point fails. Exit is not an RSS
value.

### Output ownership and limits

Existing exclusive cell dir (managed 230–236, `open("x")` writes) is the
reservation seam. Direct mode adds run-dir/handoff/guard files, `lstat`-only
walk, `(st_dev, st_ino)` dedup, symlink/special/replacement rejection, and
64 MiB sampled ceiling without truncation. Matches plan §4 output caps
(256 KiB owner JSONL hard vs 64 MiB supervisor sampled). Server RSS remains a
distinct series (design §6; calibration `validate_live_server` 159–168 never
adds server RSS into a frontend ceiling).

### Source / build lineage

Controller defaults still pin original provenance
`EXPECTED_HOST=c0709aba…`, `EXPECTED_CLIENT=3456edc8…` (calibration 32–33).
`check_source` (65–80) compares HEADs and tracked dirtiness.
`build_provenance.verify_build` (33–100) binds manifest/binary/nav/catalog and
does not grow ordinary behavior. Direct mode adds `candidate.source_lineage`
and `verify_direct_owner_build` wrapper: diagnostic Git IDs
`dcdbeebf…` / `b74dfb3…` distinct from original/reviewed
`c0709aba`/`3456edc8`/`3cdc3e4f`/`5c73a4a2`; fresh binary `392dbecc…`
required; `8e400e2d…` rebound with those HEADs refused. No
`source_admitted: true`, no dirty exemption. Runtime output outside the
admitted checkout so a successful run does not dirty the tree.

### Failed artifacts and owned-only cleanup

`_preflight_fail_report` (785–801) and `managed_receipt.complete` (80–188)
already persist failed/partial receipts (`status=failed_or_unavailable`,
`performance_acceptance=False`). Direct mode adds guard/handoff/owner hashes
conditionally; ordinary cells still do not require owner files. Signals stay
limited to Popen-owned collector/launcher and identity-checked frontend.
`attempts=1` is already hard in `run_managed_cell` 822 / calibration 317.

### Collector lifetime

Default two-interval post-observe-end stop remains (managed 1104–1124). Direct
mode keeps the collector through the existing 60 s teardown so A/B/Stop/C have
a separately measured series. Frontend RSS enforcement is the identity-bound
guard, not a retroactive collector role-map edit.

## Orch questions: SIG_IGN and preexec/threads

### Inherited ignored dispositions

POSIX: `SIG_IGN` survives `exec`; caught handlers reset to `SIG_DFL`. Design
§7.1 rejects a preblocked TERM/INT mask but restores pre-install dispositions
rather than naming `SIG_IGN`.

Traced admitted chain:

1. Controller installs only `SIGUSR1` (calibration 318–333). It never calls
   `signal.signal` / `SIG_IGN` on SIGTERM or SIGINT.
2. Managed never installs parent TERM/INT handlers. It `send_signal(SIGTERM)`
   only to owned children (498) and escalates with kill (518–528).
3. Diagnostic currently installs TERM/INT handlers *after* frontend `Popen`
   (470–473), so they do not affect today’s child.
4. `process_accounting` TERM/INT handlers (514–517) run in the collector
   sibling, not an ancestor of `tui-play`.

At frontend `Popen` in a normal Python launcher, SIGINT is
`signal.default_int_handler` (caught) and SIGTERM is `SIG_DFL`, unless the
operator environment already ignored them. `exec` of `tui-play` therefore
yields SIGINT `SIG_DFL` and SIGTERM `SIG_DFL`. Restoring those pre-install
dispositions in this chain, then exec, yields the same.

Existing launch invariants therefore prove normal child TERM/INT response for
the admitted controller/managed/diagnostic installs. A narrow extra admission
rejecting inherited `SIG_IGN` is not required: that environmental case already
exists today, is still reaped by SIGKILL in `_cleanup_frontend` hard stage
(550–588), and is outside this design’s live-authority expansion. Generated
tests that send TERM/INT and require response without hard-kill will fail a
fixture that injects `SIG_IGN` (the suite already uses a SIG_IGN child only to
prove escalation in `test_run_managed_cell.py` 152–159 `orphan_after_start`).
Do not treat POSIX-theoretical ancestor `SIG_IGN` as a design defect in this
path.

### Preexec versus threads at frontend `Popen`

`preexec_fn` is unsafe if other threads exist. At the Unix TUI `Popen`
(run_diagnostic 425–433) this process has not started the PTY drain thread
(453–454), input probe (436–438), or heaptrack guard (466–468). Direct mode
excludes heaptrack and does not start controller `MemoryGuard` (that thread
lives in the controller process at calibration 324, which `Popen`s the
launcher without `preexec_fn` at managed 982–988). Composing mask restore
onto the existing `setsid`/`TIOCSCTTY` preexec therefore has the same
thread-safety profile as the already-admitted Unix TUI launch.

Implementation must keep that order: no drain/probe/guard thread before
frontend `Popen`. Starting the drain thread after `Popen` while the
registration thread remains blocked is acceptable; restore the mask on the
thread that blocked it. Do not add a Python thread in `run_diagnostic` before
that `Popen`.

## Bounded tests that can disprove the design

Design §11 is sufficient to falsify the corrected interaction without live
fixtures:

- Default 120/600/60, 128 MiB, 960 s, collector pad, no owner files when mode
  absent; inherited `BOT_MEMORY_OWNER_CAPTURE` scrubbed (`_SCRUB_CHILD_ENV`
  126–132 does not include it today — direct mode must add it on both
  controller and launcher).
- Argv/spec/env disagreement on `--frontend-handoff`/`--run-dir`.
- `dcdbeebf`/`b74dfb3`/`392dbecc` accept vs `8e400e2d` rebound reject;
  original/reviewed IDs refused as derivative HEADs.
- First RSS just below/above `spawn_before+0.5`; 5 s outer never grants RSS
  grace.
- In-Popen SIGTERM/SIGINT pending until registered handle; unique vs
  ambiguous partial spawn.
- Child SigBlk and TERM/INT response; parent remains blocked; restoration
  failure → `orphan_risk`.
- Normal exit during PTY/metadata vs missing/mismatched `exited` state.
- Equal / +1 byte / large-jump breaches of 256/512/64/360; sampled overshoot
  honesty; symlink escape without touching the external target.
- Direct collector through Stop/C vs default two-interval stop; attempts
  remain one.

These are generated dummy-process tests, not native/live proof.

## Non-blocking notes

- Handoff “slot” reservation must not create a durable pre-spawn
  `frontend-handoff.json` that could be accepted as `spawned`. Reserve the
  path; let the launcher exclusive-write the first state.
- `spawn_before+0.5` fail-closed includes `Popen` duration. That is
  intentional, not a 5 s grace hole. Implementation should record both
  brackets so a cold exec that misses the deadline is an honest failure.
- Optional belt-and-suspenders: fail admission if pre-install TERM/INT is
  `SIG_IGN`. Not required by the admitted chain (see above).
- `validate_direct_owner_capture.py` is a protocol validator already in tree;
  controller completion still leaves `native_qualified=false` and
  `rss_reconciliation=false`.

## Conclusion

`5cb6374` can be implemented on the existing controller without changing
defaults or the approved one-attempt 30/120/60 N1 diagnostic. The five prior
lifecycle holes are closed as a single interaction: child-visible path
binding, stop-safe spawn registration with child mask restore and parent
still blocked, first RSS by `spawn_before+0.5`, identity-bound exit without
imputed RSS, lineage for `392dbecc` + `dcdbeebf`/`b74dfb3` distinct from
original provenance, and owned-only cleanup with retained failures. Tracing
the actual launcher chain does not justify a SIG_IGN admission defect or a
preexec/thread defect.

CHANGES REQUIRED: none.

Root still owns reconciliation, implementation review, native coverage, live
release, and final whole-branch review.
