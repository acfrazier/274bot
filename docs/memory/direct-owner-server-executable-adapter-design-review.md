# Direct-owner server executable identity adapter — design review

Task `t_f227a361`. Independent grok46 architecture review of the concrete
native `/proc/PID/exe` blocker recorded at `4ad17ab`. Not implementation,
not a host-permission grant, not live authorization, not same-card
controller review.

Writes are this report and
`diagnostics/direct-owner-managed-extension/server-executable-adapter-design/`.
No production, controller, test, plan, or STATE edits. No native, SSH,
sudo, permission, sysctl, service, or process-signal commands. Source-only
bounded probe only. Account-timing clarification at `5729e78` is operator
approved and is not revisited.

## Verdict

A smallest legitimate optional adapter exists. It is not available on
Concord today.

Live image proof on 6f2b410 is `os.readlink('/proc/<pid>/exe')` inside
`linux_executable_basename`. That call raised `PermissionError` errno 13
for PID 726 at the same euid as the server. `/proc/stat` sampling still
works. `sudo -n readlink` requires a password. Plan §5.3 forbids
permission and service changes without further authority.

No better existing read-only image source is supported by controller
source or the recorded native facts. `comm`, `ExecStart`, cmdline, and
receipt echo are not live image proof. Maps/`map_files` were not observed
readable and are typically under the same ptrace gate; this review does
not claim them.

The adapter is an explicit direct-mode opt-in that keeps default
`os.readlink` unchanged. When selected, it runs one fixed absolute argv
with a validated PID, bounded timeout/output, and a strict single
absolute path. A later operator sudoers line may grant exactly that
`readlink -n /proc/<ADMITTED_PID>/exe` path. This card does not install
it, and PID 726 from the failure JSON is not a live identity.

Without that later grant plus implementation, review, and native
qualification, direct-owner preflight remains failed-closed on this host.
That is a blocked live path, not impossibility of a safe design.

## What was inspected

- `docs/execution.md`
- Question `docs/memory/direct-owner-server-executable-access-question.md`
  SHA-256 `e8bfd651535d2614df9d1b5095eac8a0d49a97f1c612544cfbe0f9c9dc7f6cf4`
- Failure
  `diagnostics/direct-owner-managed-extension/server-executable-access/actual-controller-failure.json`
  SHA-256 `0cd3d2ba1138385373b05b8dc9612169e9a42891087fc59c614271ea904edd1e`
- Plan `docs/memory/direct-per-bot-owner-capture-plan.md` §5 item 3
  including the approved account clarification and the prohibition on
  cache rebuild, account reset, server restart, installation, or
  permission change
- Controller 6f2b410 blob `2c00e549317bbf7cce2222047e38470f0dde1e69`,
  identical on this checkout: `docs/memory/run_managed_cell.py`
  SHA-256 `012d7007a79e30567c7a3f8b9ad86cd3d2b49bf3d409a395af0a83290aaf35b1`
- `docs/memory/server_resources.py`
  SHA-256 `a773ed84536063fe8bb3ae8e42ec3b9cdbe484c9ba934e76b541245b96d2e4b5`
- Design `docs/memory/direct-owner-managed-extension-design.md` §6
- Frozen production diagnostic tar
  SHA-256 `2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d`
  (controller member is **not** 6f2b410; it lacks
  `linux_executable_basename`. Live blocker is the 6f2b410 controller.)
- Local source-only probe
  `diagnostics/direct-owner-managed-extension/server-executable-adapter-design/`

No Concord `/proc`, sudo, or service was touched by this card.

## Recorded blocker

Root reproduced `linux_executable_basename(726)` as `CellError`
"direct owner server executable identity unavailable" caused by
`[Errno 13] Permission denied: '/proc/726/exe'`.

Also recorded and treated as fact:

- euid 1000 matches server uid 1000
- `linux_proc_start_ticks:595` from `/proc/726/stat`
- systemd `MainPID=726`, `User=acfrazier`, `NoNewPrivileges=yes`,
  `AmbientCapabilities=cap_net_bind_service`
- `/proc/stat` resource sampling succeeded
- `sudo -n readlink` required a password
- no live capture, no sudoers/sysctl/service change, no privileged
  frontend

Inference, not proof of complete kernel policy: extra capability /
non-dumpable targets commonly make `/proc/PID/exe` require ptrace
credentials even at the same uid. This review does not claim Dumpable,
Yama, or hidepid were read on Concord.

## Current identity contract (6f2b410)

`preflight` never signals the server. Direct mode, in order:

1. `process_sampler` reads `/proc/PID/stat` only (`server_resources._linux_sample`).
   Sidecar PID/start must match that sample.
2. Host snapshots, cache snapshot, then `probe_direct_server`: TCP
   connect `127.0.0.1:43594` with a 2s timeout. This is the "current
   probe". It does not read the image.
3. `executable_basename(int(spec['game_server_pid']))` — today
   `os.readlink('/proc/<pid>/exe')` then `_basename`.
4. `validate_direct_admissions` requires the live basename to equal
   `release.context.server_executable_basename` and the health receipt
   evidence `{pid, start_identity, executable_basename, probe}`.
5. 50ms sleep, second host snapshot, second stat sample, **second**
   `executable_basename` call. Start identity and basename must be
   unchanged. Swap/OOM counters must be unchanged.

Call sites: default kwarg line 971; live calls lines 1052 and 1070.
Tests already inject `executable_basename=`. There is no sudo in the
controller. Capture sampling never rereads `/proc/PID/exe`.
`run_diagnostic._linux_direct_children` reads exe of the controller's
own children to find the frontend; it is not the PID 726 path.

`execve` does not change starttime. That is why basename is sampled
twice. Start identity alone does not detect a live exec.

## No better existing read-only option

| Source | Why it is not live image proof |
| --- | --- |
| `/proc/PID/stat` / start identity | Already works. Not the executable. |
| `comm` / stat field 2 | Task and question forbid it. Truncated, settable. |
| systemd `ExecStart` | Configured command, not the mapped file. Forbidden. |
| `/proc/PID/cmdline` | Argv, rewritable. Not used by 6f2b410 for this check. |
| Health receipt basename | Independent live read must match it. Echoing the expected value is forbidden. |
| `/proc/PID/maps` or `map_files` | Not observed readable. Typically the same ptrace restriction. Not claimed. |
| Direct `os.readlink` | Observed errno 13. |
| `sudo -n readlink` today | Observed password required. |

Granting `CAP_SYS_PTRACE` to the controller, clearing
`AmbientCapabilities`, setting dumpable, restarting the server, or
changing sysctl/service policy are all forbidden by this brief and by
plan §5.3.

## Smallest legitimate adapter

Name: `sudo-readlink-v1`. Direct mode only.

Default remains `linux_executable_basename` (`os.readlink`). Unavailable
helper stays a failed preflight. No silent fallback from proc to sudo.

Opt-in: exact enum on the direct `capture_contract`, for example
`server_executable_identity: "sudo-readlink-v1"`. Absent or
`"proc-exe"` keeps current behavior. Any other value fails closed.
Do not use environment, `$PATH` lookup, or a plugin/helper path.

Core argv, constructed only after `type(pid) is int and pid > 0`
(reject bool/str/float/zero/negative):

```
/usr/bin/sudo -n -- /usr/bin/readlink -n /proc/<pid>/exe
```

Invariants:

- `subprocess.run` list argv, `shell=False`, `stdin=DEVNULL`,
  stdout/stderr pipes, `timeout=2.0`, stdout cap 4096 bytes, no `-S`,
  `-E`, `-t`, `-i`, `-s`, no password collection, no PTY.
- Direct wrapper may only pass `spec['game_server_pid']`.
- Native qualification must confirm `/usr/bin/sudo` and
  `/usr/bin/readlink` (GNU `-n`) on Concord before any grant. This
  review did not.
- Parse stdout as one UTF-8 absolute path: no NUL/newline/CR/space/tab,
  no `//`, `.`, `..`, no trailing ` (deleted)`, not `/` alone. Then
  existing `_basename` of the final component.
- Nonzero sudo, timeout, empty/oversize/malformed output, or password
  required → `CellError('direct owner server executable identity unavailable')`.
  Do not return comm/ExecStart/expected basename.
- Around each sudo: existing `/proc/stat` sample. Start identity must
  equal the preflight/sidecar/release value before and after. If it
  changes, discard the path (PID reuse during the helper).
- Keep the two preflight basename calls. Do not cache across the 50ms
  gap. Current TCP probe stays independent.

Rejected shapes: arbitrary shell, configurable helper path, root
frontend, trusting comm/ExecStart, expected-value echo, cap/guard
weakening, server restart, sysctl/service edits, wildcard sudoers.

## Stale PID, start binding, exec

PID 726 in the failure JSON is a historical sample. Plan §5.3 already
says old PID 726 is not a live identity.

A PID-pinned sudoers line becomes wrong after restart and dangerous
after PID reuse: root `readlink` would disclose some other image.
Mitigations, all required:

1. Grant only the then-current admitted PID, never `[0-9]*`.
2. Bind `linux_proc_start_ticks:…` plus release `boot_id` around each
   read.
3. Replace the sudoers file when PID changes; do not leave the old PID
   granted.
4. Second basename sample still catches exec during preflight.
   Mid-capture exec remains undetected by start identity; that is
   existing 6f2b410 behavior and is not expanded here.

## Current probe and both preflight samples

Identity after the adapter is still the conjunction of:

- PID + start identity (stat, both ends of preflight)
- executable basename from actual `readlink` of `/proc/PID/exe` (both
  ends; sudo only if opted in)
- current TCP probe to `127.0.0.1:43594` inside the release time window
- health receipt matching all three

The adapter replaces only the exe-read mechanism. It does not weaken
comparison against the release context.

## Ancillary cost

Each opted-in preflight does two short sudo/readlink processes. 6f2b410
does those calls before `Popen`. They are not a sampler role and must
not run in warmup, observe, or teardown.

Record helper wall time and exit class in `direct_preflight` as
preflight ancillary. Do not fold them into capture CPU/RSS or the
server series. Server RSS/CPU stay the distinct `/proc/stat` series.

Sudo currently sits inside the before/after swap/OOM equality window.
`pswpin`/`oom_kill` movement from a symlink read is unlikely; if it
happens, existing preflight already fails closed. Do not hide that.
Later implementation may move the two exe reads just outside that
counter pair without dropping the two-sample identity contract. Do not
invoke sudo from `direct_guard_sample` or process accounting.

## Sudoers assessment

Wildcard `/proc/[0-9]*/exe` would let this user read any process image
as root. Reject.

A helper binary is a plugin path. Reject.

Exact candidate, **not authorized**, after native path/PID confirmation:

File `/etc/sudoers.d/274bot-direct-owner-exe-identity`, owner
`root:root`, mode `0440`:

```
acfrazier ALL=(root) NOPASSWD: /usr/bin/readlink -n /proc/ADMITTED_PID/exe
```

Validate: `visudo -cf` on that file. Matching argv is the adapter argv
above. Post-grant check (still not this card):

```
sudo -n -- /usr/bin/readlink -n /proc/ADMITTED_PID/exe
```

must exit 0, print one absolute path, and not prompt.

If `requiretty` is set, `sudo -n` without a TTY fails closed; that is
acceptable. Do not add `!requiretty` in this grant.

Full candidate text:
`diagnostics/direct-owner-managed-extension/server-executable-adapter-design/candidate-sudoers.txt`.

## Test scope (later implementation; not this card)

Keep existing injection tests, including changing-executable fail-closed
and malformed `/usr/bin/python3` basename rejection.

Add tests for the adapter only:

- default path still uses `os.readlink`; PermissionError → CellError
- opt-in unknown enum fails; non-direct with the field set fails
- argv is the exact six-element list; `shell=False`; no `-S`/`-E`
- PID bool/str/zero/negative rejected; path uses decimal `str(pid)`
- mock success `/opt/acme/bin/server` → `server`
- reject empty, newline, relative, `..`, `//`, NUL, oversize,
  ` (deleted)`, ExecStart-like argv strings, expected-value echo
- sudo exit 1 / timeout / password-required → identity unavailable
- bracketing start-identity mismatch discards stdout
- two preflight calls still required; change between them still fails
- helper not registered as a capture role
- TCP probe still independent of the helper

Source-only parser/argv cases in this card's probe: 14 parse + 7 argv,
all pass (`parser_all_pass` / `argv_all_pass` true).

## What this does not approve

- Installing sudoers, changing capabilities, sysctl, or the unit
- Implementation in `run_managed_cell.py` or tests
- Live owner release, capture, or treating PID 726 as current
- Weakening live `/proc/PID/exe` proof to comm/ExecStart/cmdline
- Account-timing revisit
- Navigation CF2 or performance acceptance

Authorized next path, not started here: implement the opt-in adapter
and tests; same-card `reviewer`; native qualification of sudo/readlink
pathnames and the then-current PID/start; **then** ask the operator
for the exact filled-in sudoers line.

## Verdict line

Safe optional `sudo-readlink-v1` adapter is designable: explicit
direct-mode opt-in, fixed `/usr/bin/sudo -n -- /usr/bin/readlink -n
/proc/<validated-pid>/exe`, bounded parse, start-identity brackets,
preflight-only ancillary cost. No better existing read-only image
source is in evidence. Live remains blocked until later
implementation, review, native qualification, and a separate operator
grant of the exact PID-pinned sudoers line. This card grants none of
those.
