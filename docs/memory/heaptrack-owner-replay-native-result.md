# Saved Heaptrack replay: Linux qualification and failed production attempt

The reviewed parser passed native Linux qualification, but its sole production
replay failed the unchanged 180-second CPU guard in raw pass 1. It produced no
owner ranking. The original allocation capture remains failed at the raw-size
guard; neither result establishes a memory saving or closes a campaign gate.

Code: `378e634b103d525aef33b86d22e8238fa1bddc79`, approved by actual Grok 4.5 /
xai-oauth, session `20260908_170931_3239c3`, task `t_2aba75a5`.
Executor: Concord, boot `2217ec26-dc3e-47a3-a700-d3fafb34163f`.
The staged 12 source/fixture files were hash-verified against the exact commit
and saved-fixture inventory. No application, profiler, capture, interpreter,
Windows/Hyper-V job or concurrent native build was started.

## Native verification

`python3 -B -m unittest discover -s docs/memory -p test_heaptrack_owner_replay.py -v`
passed all 21 tests in 13.723 seconds, with no skips. This includes owned-child
RSS pressure and Linux address-space limits, CPU/wall failures and cleanup.
The persistent saved smoke also passed: full normalized peak multiset 5730024
bytes, EOF 407233 bytes / 34 allocations, all 16 output hashes verified, Linux
process monitoring active, child reaped. Its observed peak RSS was 24473600 bytes;
this is tooling qualification, not a production resource comparison.

## Sole production attempt

The manifest SHA-256 was
`7fcd08aed20e44c81dd495f83ae663b167f4392b84c39a79ce927b45165e1595`.
It binds the existing raw/interpreted/oracle/metadata/stderr files and selects
only the first global peak, last actual timestamp and EOF. No requested extra
time or automatic retry was used. Admission found 997117952 available memory
bytes, 11835043840 free disk bytes and no detected competing frontend/build/profiler.

The runner PID 2325 launched child 2326. After 180.095 seconds the child reported
`CPU guard`; the runner killed/reaped the still-exiting owned child (exit -9) and
returned exit 1. Peak child RSS was 90357760 bytes and address space 100413440
bytes. The outer launcher did not time out. Both processes were then verified
absent and the only retained production output was `failure.json`.

The raw pass did not finish: its input hash and raw/interpreted equivalence were
not established by this attempt. The earlier inventory hashes remain provenance
receipts, not a newly verified complete replay. No partial ranking is accepted.

## Evidence and next decision

The 41-file hash-verified export is under
`diagnostics/replay-native-stage-378e634-preparation/native-evidence/`.
Archive SHA-256:
`0d741086ccb6d4e53e11d0376d1cdac8bee3909fdf148826d5f184c4bb1bbaf1`.
The JSON companion retains launch, test, smoke and failure receipts.

Parser throughput is now the demonstrated blocker for this tool on Concord;
memory exhaustion was not the stopping condition. Investigate parser overhead
using small local fixtures before proposing a reviewed implementation change.
No limit increase, production retry, new capture or re-interpretation is released
by this report. Full ownership/lifecycle, matched savings, absolute budgets,
scaling, rendering and final Grok 4.6 gates remain open.
