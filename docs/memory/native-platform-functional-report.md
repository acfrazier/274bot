# Native platform functional evidence

Date: 2026-09-07

This report records the first actual managed N1 diagnostics on the native
Windows ConPTY host and the Concord native Linux host. The receipts are
functional evidence only. They are diagnostic runs, not matched performance,
resource-budget, latency, renderer, or deployment acceptance.

## Executive result

Both corrected native runs launched the TUI against the isolated 4c95f87
server fixture, reached an active scene, accepted terminal writes, completed
the bounded observation, and exited cleanly. Each workload was qualified for
its diagnostic purpose and showed real gameplay progress: steals, eating, and a
bank trip. The two runs share the same host/client source labels, nav-pack hash,
server commit, and cache content identity.

The evidence does not establish a matched native-settings result. The root-side
binding attempt is unavailable on both hosts because the server sidecar omitted
the required `port_listen` field (`server_port_listen_missing`); the
configuration key was also omitted. The raw receipts remain unchanged. No
performance or savings claim is accepted.

## Windows ConPTY run

Evidence roots:

- `docs/memory/diagnostics/windows-native-conpty-b4b686f/managed-conpty-b4b686f-b/`
- `docs/memory/diagnostics/windows-native-conpty-b4b686f/20260907T164915Z_tui_n1_active/`

The corrected run (`native-conpty-n1-b`) started at
2026-09-07T16:49:15.916052Z, ended at 16:53:04.574860Z, and reported launcher,
collector, and sampler exit code 0. The diagnostic metadata identifies a real
ConPTY terminal at 120x40 and sends 119 key writes. The terminal probe is
transport evidence: input writes were made and the separate native ConPTY
probe verified `isatty`, console APIs, key receipt, and true exit 42. The
input-probe endpoint itself is explicitly only “terminal write only”; it is not
a native input acknowledgement or latency result.

The qualification receipts show one active slot over the 30-second warmup and
approximately 120 seconds of observation. The final qualification paint shows
12 steals, 1 food eaten, 22 food remaining, and 1 bank trip. The slot remained
running and had no recorded slot error. This is enough to distinguish a real
active gameplay run from a transport-only smoke test, but not enough to prove
long-lifecycle stability or a target workload.

The run used the frozen TUI binary with SHA-256 prefix
`9f7d5a7ac9a9bcd5...` and the recorded host/client commits `b4b686f` and
`3456edc`. It used `memory-profile-no-alloc`, with allocation counting off. The
Windows renderer profile was off and `render_policy` was `none`; therefore the
run proves no GPU backend, completed-frame cadence, or renderer responsiveness.

The first managed attempt (`native-conpty-n1-a`) is a preserved prelaunch
failure, not a gameplay failure. It did not launch the frontend because the
root spec supplied `C:\Users\BotTest\.274bot\js-cache` where the fixture
required the actual `ENGINE_DIR/data/pack/client` inputs; the receipt records
`No such file or directory` for `js-cache/versionlist`. The separate Python
receipt at `python-managed-968846c` also preserves a Unix-only import/fixture
failure. The corrected Python test receipt at `python-managed-b4b686f` records
151 tests, exit 0, with seven platform skips. These tooling failures must not
be conflated with the corrected TUI gameplay result.

### Windows process and resource semantics

The explicit managed sampler roles were:

- game server PID 24224;
- controller PID 19272;
- bootstrap PID 23116;
- launcher PID 15580;
- collector PID 12964.

The frontend was PID 20800 and had a stable Windows creation-filetime identity.
A ConPTY `conhost` child (PID 15812, child of launcher PID 15580) was observed
in the native investigation but was not continuously included in the managed
role map. That omission limits process-coverage interpretation; it does not
invalidate the frontend's recorded clean exit.

Windows RSS is current process working-set bytes obtained through
`GetProcessMemoryInfo`, not peak working set, pagefile usage, private usage, or
an inferred process-tree total. CPU is from `GetProcessTimes`. The sampler
measured only explicit root PIDs, did not discover descendants, and reports
waited-child current RSS as unavailable. Windows host pressure was also
explicitly unavailable rather than fabricated as zero or healthy. These
semantics make the values diagnostic observations, not an accepted memory
budget.

## Concord native Linux run

Evidence roots:

- `docs/memory/diagnostics/concord-native-20260907a/native-linux-n1-a/`
- `docs/memory/diagnostics/concord-native-20260907a/20260907T165530Z_tui_n1_active/`

The corrected run (`native-linux-n1-a`) started at
2026-09-07T16:55:30.785997Z, ended at 16:59:31.332129Z, and reported launcher,
collector, and sampler exit code 0. The independent workload qualification is
true, with no errors, 119.575 seconds of observation, 9 steal gains, and a
median frontend resident value of 185,229,312 bytes. Its own receipt states
that this means workload qualification only.

The qualification paint spans approximately 120 seconds after warmup. The
final paint shows 11 steals, 1 food eaten, 22 food remaining, and 1 bank trip;
the slot remained running without a recorded error. The diagnostic samples
contain 230 records including seed and observation records. The run accepted
120 terminal writes. As on Windows, these are terminal writes rather than
acknowledged native input events, so input delivery and end-to-end latency are
not proven.

The binary is the reviewed native Linux build with SHA-256 prefix
`f00e7fb18e28c013...`, from the Concord build receipt. The build evidence
records stable reviewed inputs, host/client source labels `b4b686f` and
`3456edc`, and the unchanged system allocator. The Linux command requested
scheduling, render, responsiveness, and fine-responsiveness profiles, but the
metadata records `render_policy: none`, `gpu_completion_profile: false`, and
`gpu_tracked_bytes: 0`. TUI rendering was absent in this run; no GPU backend,
scanout, or completed-frame claim may be inferred from the requested profile.

### Linux process and resource semantics

The explicit managed roles were:

- game server PID 152004;
- controller PID 152149;
- SSH session PID 152148;
- launcher PID 152151;
- collector PID 152152.

The frontend was PID 152160 with a stable Linux `/proc` start-ticks identity.
The server identity is PID 152004 with start ticks `241214967`, server commit
`4c95f87`, and Node `24.19.0`. The server identity's point sample reports
639,713,280 resident bytes, measured from `/proc/<pid>/stat` RSS pages times
page size, and cumulative CPU of 46.97 user seconds plus 4.21 system seconds.
This is a current RSS point sample for that explicit server PID, not a process
map total or a budget result.

Linux sampling used `/proc` current RSS and cumulative CPU for explicit root
PIDs. The collector also recorded Linux PSI memory pressure from
`/proc/pressure/memory`. Waited-child current RSS is unavailable; child CPU is
only cumulative/delta `RUSAGE_CHILDREN` for reaped children and does not include
still-running descendants. The process collector is therefore not a tree walk
and does not silently fold frontend children, server descendants, or helpers
into the named role values.

## Cross-platform functional findings

What is established:

1. The isolated native server fixture was usable by both corrected runs.
2. The TUI reached active gameplay on both native terminals.
3. Both runs produced real input writes and real terminal output; the Windows
   transport probe additionally proved ConPTY key receipt.
4. Both runs made gameplay progress, including eating and a bank trip.
5. Frontend, collector, and launcher completion were recorded with exit code 0
   where the corresponding receipts report it; the controller was a bound,
   stable live role during each run, not a completed exit-0 process.
6. Cache provenance and binary/source receipts were recorded without changing
   or re-hashing the raw evidence.

What remains unavailable or unproven:

- root-side matched binding and server `port_listen` evidence;
- acknowledged native input coverage and input/decode paired latency;
- long-lifecycle or N>1 gameplay reliability;
- complete descendant/helper process coverage, especially Windows ConPTY
  `conhost`;
- accepted RSS/CPU savings, absolute budgets, or final target capacity;
- native GPU backend, renderer completion, scanout, or TUI rendering behavior;
- full matched settings coverage and a valid cross-host performance comparison.

The observed process RSS, CPU, scheduling, decode, and responsiveness fields
are retained as diagnostic observations. They must not be promoted to accepted
savings or final budgets merely because the launcher and collector exited 0.

## Prioritized next evidence correction

Priority 1: correct the native server-side binding contract by recording the
server's actual `port_listen` configuration and binding the existing server
identity to that explicit listen evidence before another native evidence cell.
The correction should be a bounded receipt/preflight change only: preserve the
failed `server_port_listen_missing` artifacts, reject absent or mismatched
listen data, and then reuse the existing frozen binaries and fixture. Until
that receipt exists, do not call either native run matched or use its resource
series for a performance decision. This is a specific evidence-contract fix,
not an indefinite rerun plan.

## Source receipts

Windows: `managed-conpty-b4b686f-b/cells/native-conpty-n1-b/receipt.json`,
`.../cell_report.json`, `.../20260907T164915Z_tui_n1_active/metadata.json`,
`.../samples.qualification.jsonl`, `.../input-probes.jsonl`,
`.../conpty-native-b4b686f/receipt.json`.

Linux: `native-linux-n1-a/cells/native-linux-n1-a/receipt.json`,
`.../cell_report.json`, `.../workload-qualification.json`,
`.../independent-binding.json`, `.../server-identity.json`,
`.../20260907T165530Z_tui_n1_active/metadata.json`,
`.../samples.qualification.jsonl`, `.../input-probes.jsonl`.
