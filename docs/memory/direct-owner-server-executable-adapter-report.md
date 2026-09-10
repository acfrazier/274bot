# Direct-owner server executable identity adapter implementation

Task `t_74e0c73e`. Implementation of the optional adapter approved by design commit
`901a40991e8a0952c08af18a254fffc3e4e1b3de`. This task did not run a live cell,
connect to Concord, invoke real sudo, alter the server, install a sudoers file, or
change any host permission.

## Result

The direct-owner controller now supports an explicit
`--server-executable-identity {proc-exe,sudo-readlink-v1}` selector. The selector is
valid only with `--direct-owner-capture`. It is copied into the direct
`capture_contract` only when explicitly supplied.

Absent selector and explicit `proc-exe` both retain the existing
`os.readlink('/proc/<pid>/exe')` path and existing error behavior. The absent
selector does not add a field to the capture contract, release context, or release
spec binding, so the existing default receipt schema and bytes remain unchanged.
An explicitly supplied selector is required in both the root release context and
root spec binding; every admission receipt must repeat that exact context. A
release generated for one explicit selector is rejected if the spec is switched
to the other.

`sudo-readlink-v1` validates `type(pid) is int and pid > 0`, then starts only:

`/usr/bin/sudo -n -- /usr/bin/readlink -n /proc/<pid>/exe`

It uses list argv, `shell=False`, `stdin=DEVNULL`, bounded stdout and stderr pipes,
no PTY, no password input, no environment- or PATH-selected helper, and a 2.0
second monotonic deadline. A nonblocking selector drains both pipes while retaining
at most 4096 bytes from each. It detects a 4097th byte while storing no more than
the cap. It never calls `communicate()`.

The stdout parser requires strict UTF-8 and exactly one non-root absolute POSIX
path. It rejects empty or relative output, NUL, all whitespace, backslash, `//`,
`.` and `..` components, a ` (deleted)` suffix, oversize output, and argv-like
strings. Only after those checks does it apply the existing basename validator.
Every launch, exit, timeout, overflow, parse, and identity failure exposed by this
adapter is `CellError("direct owner server executable identity unavailable")`.
It never substitutes the expected basename, `comm`, `ExecStart`, or another image
source.

Each opted-in executable read is bracketed with the existing Linux `/proc/PID/stat`
sampler and a fresh `/proc/sys/kernel/random/boot_id` read. The numerical PID,
start identity, and boot identity must match the admitted sidecar/preflight/release
identity before and after the helper; the path is discarded before use on any
mismatch. Both pre-existing executable reads remain, as do the independent TCP
probe and the 50 ms host/swap/OOM counter window.

Successful helper wall time, exit class, stdout byte count, stderr byte count, and
both identity brackets are recorded only under
`direct_preflight.server_executable_identity.reads`. The helper is created only in
preflight before frontend `Popen`; it is not an ambient/capture/sampler role and is
not invoked in warmup, observation, or teardown. Its cost is not added to the
server resource series or capture series.

## Bounded cleanup reconciliation

The implementation does not use the design draft's unbounded
`subprocess.run(..., PIPE)`/post-`communicate` length check. It places each helper in
a new session and, on every timeout, overflow, nonzero exit, malformed result, or
other failure, sends TERM then KILL only to that helper process group and performs
bounded waits to reap the direct child. Generated timeout and stdout/stderr
overflow tests use actual child processes (through an injected factory, never real
sudo) and verify the direct child is reaped within the bounded wall allowance.

The fixed `/usr/bin/readlink` command is not expected to create descendants. This
portable Mac work cannot prove cleanup of a hypothetical privileged descendant
that deliberately escaped the new helper session or became unkillable across the
sudo credential boundary. No host privilege was widened to manufacture such a
proof. Fresh Linux qualification after review must retain this boundary and must
not claim more than direct-child reap plus same-process-group signalling.

## Tests and evidence

Evidence directory:
`diagnostics/direct-owner-managed-extension/server-executable-adapter-implementation/`

- `adapter-focused-tests.log`: 12 adapter/selector/binding/identity/generated-child
  tests passed in 2.173 s, exit 0. No real sudo invocation.
- `portable-regressions-excluding-baseline.log`: 97 affected portable tests passed,
  one platform skip, exit 0. The exclusion is exactly
  `test_default_collector_stops_after_pad_before_generated_stop_and_c`.
- `portable-affected-tests.log`: the unfiltered candidate suite ran 98 tests and
  reproduced that one collector-ordering failure.
- `head-baseline-affected-tests.log`: an isolated `git archive` of HEAD
  `901a40991e8a0952c08af18a254fffc3e4e1b3de` ran the corresponding 86-test suite
  and reproduced the same collector-ordering failure (`stop request ~6.35 s` versus
  generated Stop `~0.66 s`). This establishes the failure as pre-existing on this
  Mac rather than accepting a lucky rerun.
- `py-compile.log`: all four changed Python files plus the bounded regression runner
  compiled successfully, exit 0.
- The scoped four-file source/test diff passed `git diff --check`. Added lines
  contain no `shell=True`, `os.system`, eval/exec, pickle load, or hard-coded
  credential assignment.

Current source SHA-256 at verification:

- `run_managed_cell.py`: `bc9ec652f2bd149b76a322e24c5419d5f88fec21937d78bc4d9db636bab6d7b5`
- `run_current_tui_calibration.py`: `7d4ee523f6913cbda4b434eb0890867937ae88f13521d908a6e552ef9ccd5576`
- `test_run_managed_cell.py`: `5d573340c5c19bf14494c2781e6a37359c88f1e1553934323724f30c829c48e0`
- `test_current_tui_calibration.py`: `b6a40fb9f852569d977ed60743f5e0c5e274c0e46b9d70fbdbdd8aa046853838`

## Native and permission boundary

Portable implementation and tests are complete. They do not establish that
Concord has `/usr/bin/sudo` and GNU-compatible `/usr/bin/readlink -n`, do not prove
a successful privileged read of the current admitted server PID, and do not grant
permission. Root must perform the separately required fresh Linux suite and exact
path qualification after same-card review. Only after that may root prepare the
current PID-pinned sudoers text and ask the operator to apply it. The historical
PID 726 is not authorization or a current live identity.
