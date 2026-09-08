# Cohort stimulus/controller integration review (Grok 4.6)

Bounded integration milestone review before live matched pair. Not
per-task re-review and not whole-campaign approval.

## Identity

| Field | Value |
|---|---|
| Reviewer profile | `branchreviewer` |
| Actual model / provider | `grok-4.6` / `xai-oauth` (no task overrides) |
| Branch | `codex/memory-diagnostics` |
| Frozen source head | `1cdc3fb94fc23ed037112e6b17bbf0c8fc875f51` |
| Base | `3da59e113c80a2e6bdad1b893e628e86f6f91c04` |
| Checkout HEAD at review | `1cdc3fb94fc23ed037112e6b17bbf0c8fc875f51` |
| Client submodule (unchanged) | `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| UTC | 2026-09-08T14:33:11Z |

No Rust, crate, or `Cargo.*` changes in `3da59e1..1cdc3fb`. No native,
network, SSH, or live cell actions in this review. Root source HEAD
remained `1cdc3fb` while independent staging continued outside tracked
source.

## Verdict

**BOUNDED ACCEPT** of combined pair-controls + stimulus-accounting
source at `1cdc3fb` as the integration gate before native final
no-launch / preflight and the one predeclared reference/candidate pair.

This does **not** release:

- native final no-launch contract and privileged prepare (root-owned;
  historical `a522` 31-file proof is not this controller)
- fresh per-cell scene2 / capture-enabled / slot0 / rectangle / Game
  Image binding
- actual one-pair execution
- input coverage, host acknowledgement, p99, RSS comparison, or
  campaign completion

Inert wrapper proofs are real Popen/`windows_process_sample` mechanics
only. They are not pulses, frontend identity, or cohort membership.

## What was inspected

Cold then against the named contract:

- `docs/memory/cohort-stimulus-controller-integration.md`
- `docs/memory/native-cohort-pair-execution-protocol.md`
- `docs/memory/current-latency-companion-plan.md` §§4–7
- `docs/memory/windows-cohort-pair-controls/run-cohort-pair.py` and
  `test_cohort_pair_controls.py` vs `3da59e1`
- `docs/memory/windows-cohort-stimulus-accounting/run_stimulus_accounted.py`
  and `test_run_stimulus_accounted.py` (added after `3da59e1`)
- `docs/memory/run_managed_cell.py` `QualificationBoundaries.consume`
  call sites (unchanged)
- immutable helper copies
  `windows-cohort-pair-controls/invoke-panel-input-stimulus.ps1` and
  `windows-visual-proof-tools/invoke-panel-input-stimulus.ps1`
- `launch-cohort.ps1` / `prepare-cohort.ps1` no-launch consume order
- native inert receipts
  `diagnostics/windows-cohort-stimulus-root-probes/wrapper-inert-7427b9b.json`
  and `wrapper-inert-1cdc3fb.json` (operator mid-review); earlier
  `-Command` probes remain in that directory

`docs/memory/STATE.md` at this head still records the 14:15 UTC
`2fbfd03`-era boundary. Treated as a stale snapshot, not current
next-action, matching the orch freeze note.

## Contract mapping

### Observe-start publication uses the existing loop clock

`publication_boundaries` subclasses `rmc.QualificationBoundaries`
without replacing `consume` validation. Superclass still parses the
qualification row, sets `self.start` / `self.end` / errors, and records
`end_received_mono` only on observe-end.

The subclass writes `observe-start-publication.json` only on the first
transition to a validated start with an empty error list. The stored
`monotonicSeconds` is the `now` argument already passed by the managed
loop (`run_managed_cell.py` polls qualification with
`time.monotonic()` at the live consume sites). That is controller
receipt time, not a reconstructed host `[START,END)` timestamp.
Qualification bytes are hashed; the row is not rewritten.

No extra process or polling loop. The class is restored in `finally`.
A second start is consumed by the base parser (error) and does not
overwrite the publication file.

### Wrapper launch: `-File`, typed arrays, finite pre-spawn window

Helper SHA256
`04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1`
matches both helper copies and the protocol. Manifest schedule is
120 s / 1000 ms / 80 ms, trigger `[60, 90]`, `noRetry=True`.

Launch argv is a list: `powershell.exe -NoProfile -ExecutionPolicy
Bypass -File <per-run launcher.ps1> <launch-spec.json>`. Geometry and
Game Image point are JSON then `[int[]]` in the static launcher. This
is the replacement for the `c875105` `-Command` binding that native
probe showed truncating arrays.

Nonfinite publication clocks fail in `_required_binding` before spawn
(`type(...) not in (int, float)` also rejects `bool`). After writing
launcher/spec artifacts, delay is recomputed against the recorded
publication; outside `60..90` inclusive fails without `Popen`.
Fractional delays are kept; they are not rounded to the old integer
whitelist.

Hung helper: one spawn, finite lifetime, terminate, cleanup budget,
kill. No retry. Missing/mismatched helper receipt stays `incomplete`.
`inputCoveragePass` and `performanceAcceptance` are always false.

### Identity, samples, helper UTC honesty

Target start identity is sampled before spawn. Helper identity is
`windows_creation_filetime:*` from the sampler, not invented UTC.
`helperStartUtc` is copied only from a sample `start_utc`; otherwise
null. CPU/RSS summaries are the last valid cumulative sample; exited
final rows stay unavailable and are not labeled exit totals.

Roles are `input-helper` and `wrapper-sampler`. Target identity is
recorded and `targetExcludedFromManagedTotals=true`. Sampler loss is
retained as unavailable rows, never zero-filled. Role identity change
invalidates that role.

### Post-run validator consumes the producer envelope

`validate_stimulus_receipt` now takes managed `frontend_pid` /
`frontend_start_identity` and the publication file. It requires:

- schema/outcome/cell/helper/cadence match
- finite `startedUnix` not before this run
- capture and slot0 flags true
- native helper FILETIME identity (UTC not required)
- target pid/start identity equal to the managed cell report
- finite delay in `[60, 90]`
- `receipt.observeStartPublication == publication` and
  `helperSpawnMonotonicSeconds - publication.monotonicSeconds == delay`
- sampler identity object
  `{label: root-managed windows_process_sample, backend:
  windows_process_sample.sample_process}`
- helper and wrapper raw sample linkage, positive RSS, both finite
  CPU counters, last-sample summaries
- archived `helper-receipt.json` hash match with 120 ordered
  Left/Right events and `SendInput` results `== 1`
- `inputCoveragePass is False` and `performanceAcceptance is False`

SendInput rows are helper-local pulse accounting, not host ack. Cohort
reader acceptance is not invoked here.

Producer→consumer adapter test executes the real wrapper `run()` and
feeds that envelope to the validator with no schema translation.

### No-launch through prepare

`COHORT_NO_LAUNCH_VALIDATION=1` still skips prepare consumption.
`prepare-cohort.ps1` emits `client_started=false` /
`scheduled_task_created=false` after contract evidence.
`launch-cohort.ps1` is the only scheduler and refuses a non-exact
prepare receipt. Tests assert that order in source.

## Sampler lineage and inert proofs

| Item | Evidence |
|---|---|
| `c875105` wrapper SHA256 | `67ed8caffd0249487ab84736187453dc7b5cc3acfb66d43634d0aae060b683fa` — Grok 4.5 session `20260908_100744_aa256d` later invalidated by native `-Command` array truncation. Source and probe receipts preserved. |
| `7427b9b` wrapper SHA256 | `888ce381580be7d40c626cd4e570aa06ec0d91ce7d20e1052fc2fd8835a38cbf` — same bytes as `d1f3bf2` on this branch (`7427b9b` is not an ancestor of `1cdc3fb`). Approved Grok 4.5 session `20260908_101945_01e0c8`. Inert primary `wrapper-inert-7427b9b.json`: eight mechanics checks true, `actualInputEvents=0`, injected helper `a28628105…`, `helperStartUtc=null`, real FILETIME identities. Predates publication binding / final pre-spawn recheck. |
| `1cdc3fb` wrapper SHA256 | `c9743b45f7a8f0c0bfbc4461b94529f316b80d9b2e2ebb713224c3bc738d5695` matches worktree and `wrapper-inert-1cdc3fb.json`. Delay `60.131657999998424`; `helperSpawnMonotonicSeconds - monotonicSeconds` equals that delay after JSON round-trip. Eight checks true, `inputCoveragePass=false`, injected helper only, `helperStartUtc=null`, two-role accounting available, helper final sample honestly `process PID … has exited`. Wrapper pid equals fixture target pid — live validator would reject that collision; this fixture is not a managed frontend. |

Live wrapper manifest must embed the **exact** controller publication
object (schema, cellId, qualification, …). The inert fixture stored
only `monotonicSeconds` and would not satisfy the live validator.

Orch mid-review: live BotTest transport will be scheduled `pythonw.exe`
importing this reviewed wrapper plus `windows_process_sample` via
explicit `HostRoot/docs/memory` `sys.path`, with
`popen=functools.partial(subprocess.Popen,
creationflags=subprocess.CREATE_NO_WINDOW)`. That avoids a resident
PowerShell wrapper host and foreground steal. The input helper remains
the single reviewed PowerShell child and is sampled. This transport is
root staging, not a `1cdc3fb` source mutation; it does not change the
source verdict. Manifest, launcher hash, and transport flags remain
root-archived evidence.

## Independent tests run here

| Command | Result |
|---|---|
| `python3 -m py_compile` on wrapper, pair controller, both test modules, `check-cohort-contract.py` | exit 0 |
| `python3 -m unittest discover -s docs/memory/windows-cohort-stimulus-accounting -v` | 16 passed |
| `python3 -m unittest discover -s docs/memory/windows-cohort-pair-controls -v` | 10 passed |

No cargo, no native launch, no helper SendInput.

## Findings

1. **Not blocking this gate.** `sampleIndexes` records every
   `status=available` row, including rows that fail `_valid_sample`.
   A mixed available/invalid series can mark the role available while
   the consumer fail-closes on the invalid linked sample. Real
   `windows_process_sample` rows in the inert envelopes include
   identity, both CPU counters, and positive int RSS. Fail-closed, not
   a silent pass.

2. **Not blocking this gate.** Fallback stage strings remain
   `cohort-candidate-ca56e143` / `cohort-reference-e25f328`. Live
   staging must keep the explicit root environment (orch: candidate
   short-7 `candidate-ca56e14`, not the fallback 8).

3. **Not blocking this gate.** `windows-cohort-pair-controls-report.md`
   still describes helper/target accounting. Authoritative consumer is
   `run-cohort-pair.py` at this head (helper/wrapper, target excluded).

No material defect found in publication clock/boundary preservation,
`-File` typed-array invocation, finite pre-spawn recheck, identity and
sample honesty, bounded cleanup, or post-run envelope checks that
refuse to treat SendInput as host ack.

No per-frame world copy, no new tick-end opcode, no cohort membership
rule change, no invented helper UTC.
