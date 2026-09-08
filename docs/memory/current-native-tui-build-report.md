# Current native TUI build and qualification

Frozen runtime source: host `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`, client
`3456edc8dabf7b25ada78110ffa56327af9f67a4`. The Hyper-V Linux builder produced
ELF SHA256 `a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9`
(95,434,872 bytes), using locked offline release build, `memory-profile-no-alloc`,
System allocator, and snapshot-dedup disabled. All 1,113 frozen Git file bytes
were verified before build, after the failed test attempt, and on the VPS.
The original builder checkout and executable are retained unchanged.

The affected native suite passed API 5, host 213 (one ignored), host-play 182,
and script 46 tests before a TUI preparation fixture stalled. Root's gdb
backtrace showed test teardown joining a worker in client initialization retry.
Root recorded the process identity and terminated that test process; the
original suite exit 101 and its complete log remain evidence of failure.

The reviewed correction `bca8476` adds only cfg(test) per-session spawn
suppression and fixture assertions. Tests still call the real preparation
function; production compilation contains neither the field nor the guard.
Earlier rejected runtime-seam and copied-algorithm drafts are retained in Git.
Grok 4.5/xai-oauth session `20260908_120656_7b735e` approved the final source.
A separate native builder worktree applied net patch SHA256
`3bf08949fee5f44a2c6dc82525004a9cc0f47759526d43bda6f77cc9f33a51e4`
over c0709ab. `cargo test --release --locked --offline -p tui --features
memory-profile-no-alloc --lib -- --test-threads=1` passed all 92 tests, with
zero ignored, exit 0. This qualifies the tested production path with an isolated
unit fixture; it is not new live gameplay or performance evidence.

The operator rebooted Concord. Root verified kernel `6.8.0-139-generic`, new
boot `08c031f9-c44f-42e5-ac32-821bbdec7759`, no pending reboot marker, SSH on
22111, and Concord PID 726/start `linux_proc_start_ticks:1761`. All 688 fixture
files match the retained manifest. Loopback ports 80/43594/8898 listen; HTTP
CRC returns the same 40 bytes as before reboot. Old PID 152004 is historical.
Native ldd resolves every staged executable dependency. The sole calibration
cell subsequently completed as recorded below.

Controller bf05cb1 passed Grok 4.5/xai-oauth session
`20260908_120556_cf96a2`, but its native Python suite subsequently failed one
of 17 tests: the guard test patched a module function after the constructor
had captured it as a default parameter. The Mac pass depended on missing
/proc, so it did not establish deterministic guard cancellation. Corrective
card t_31bad671 delivered correction 94675d1, approved by actual Grok 4.5
session `20260908_121357_334dfc` and integrated as d41fe28. The native suite
then passed all 17 tests. Two direct managed cleanup tests separately passed
on the VPS, establishing owned process termination. The original failed log
remains preserved; the early short-child test alone is not termination proof.

Raw artifacts are retained under primary `diagnostics/current-tui-calibration-build-c0709ab/`:
29-file verified original build archive, fixture-bca8476-proof, the initial
failed controller test log, and VPS staging receipt. VPS preparations are in
`/home/acfrazier/274bot-campaign/calibration-c0709ab-incoming/`; frozen checkout
is `workspace-c0709ab/host`. The retained original build manifest predates
completed test qualification; subsequent native logs supply the results above.

The sole TUI N16 calibration `current-tui-n16-1616` completed with frontend
and controller exit 0, all 16 slots qualified at both phase boundaries, and
49–82 successful steal gains per slot. The observed phase lasted 600.043 s;
580 resource samples bracket 598.912 s. Raw steady median RSS was 520.779 MiB,
native peak RSS 597.957 MiB, and mean CPU 0.561746 cores. Steady RSS and CPU
exceed the respective 512 MiB and 0.5-core working budgets.

The verified 93-file archive is preserved under
`diagnostics/current-tui-n16-1616/current-tui-n16-1616-final`, SHA256
`7612191cfe88f61303e1d0409952032a552c53ff87370f90b03fd4d17a2941c4`.
The original native reader rejected the explicit empty frontend-process
observation; correction 82057de passed Grok 4.5 review and native replay now
binds the same artifacts successfully. See current-native-tui-calibration-report.md
for the distinct raw-resource gate and overhead limits. Raw files and original
rejected outputs remain unchanged.
No matched saving, absolute budget, lifecycle, or final campaign gate is accepted.
Profile-off cadence/latency and instrumentation overhead remain unmeasured.
