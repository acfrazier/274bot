# Current TUI active-bot attribution: N1 and N16

The current single-bot TUI is below the working one-bot RSS targets in this
diagnostic. The N16 steady RSS and CPU misses remain. The two-point difference
suggests investigating incremental bot ownership next, not a fixed-cost rewrite.
This is a single sequential N16/N1 pair, not an accepted saving, a repeated
comparison, an idle-bot measurement, or a causal allocation attribution.

| Quantity | N1 | N16 |
|---|---:|---:|
| Steady median RSS | 176.322 MiB | 520.779 MiB |
| Native lifetime peak RSS | 176.902 MiB | 597.957 MiB |
| Mean process CPU | 0.048993 cores | 0.561746 cores |
| Mean client iterations per slot per second | 49.669 | 49.479 |
| Steals per slot | 66 | 49–82 |
| Qualification observation span | 600.031 s | 600.043 s |
| Resource sample bracket | 599.009 s | 598.912 s |

N1 completed exit zero on its sole attempt. Both runs qualify with all requested
slots ready, active and progressing. N1 has 588 observation samples. The original
N16 is preserved; it was not rerun. N1 archive SHA256 is
`2fbbc0755a755d623aa9691681eafa6933d7f2e53729083b84d84324761a3a92`,
under `diagnostics/current-tui-n1-1727/current-tui-n1-1727-final.tar.gz`.
All 101 manifest payload files verified after download; originals were hashed
before and after native reader execution. N16 archive and its 93 payload files
were also reverified. The archive manifest itself is additional to payload counts.
Raw bytes and original unsuccessful reader results remain intact.

Both use host c0709ab, client3456edc8, binary SHA256
`a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9`,
System allocator, memory-profile-no-alloc, profiles and allocation counting off,
120x40 real terminal, 120-second warmup, the same cache/nav/catalog and the same
server726/start1761 on Linux6.8.0-139. Exact bound source provenance is identical.
Match-key differences are N, corresponding per-ordinal disabled-renderer entries,
and separately captured host conditions. Account populations differ. N1 uses the
reviewed e707e2d controller selection extension; all1113 frozen runtime-source
files were verified unchanged before and after installing two untracked tools.
Native24controller tests passed. Install receipt and original tool bytes are in
the N1 archive. Mac289 work ran separately, not on the measurement VPS.

Native82057de binding is bound/qualified with managed resources available for
both cells. Raw resource/shaped comparison still reports missing_resource_provenance;
that is not repaired by the separate managed-resource binding. Overhead is
unmeasured. These data supply no p99 simulation/input/decode latency or rendering
proof, and no final budget acceptance. N1 server medianRSS is631.957MiB and
CPU about0.04465cores, accounted separately with controller/launcher/collector/SSH
roles in the evidence JSON; do not subtract helper CPU as measured overhead.

## Two-point interpretation

(RSS16 − RSS1)/15 = **22.964 MiB per added active bot**, above the working16MiB
incremental target. (CPU16 − CPU1)/15 = **0.034184 cores per added bot**.
A line through these two points has intercept153.358MiB and0.014809cores.
That intercept is an extrapolation, not measured zero-bot or idle cost. Two N
values cannot establish linearity, uncertainty bounds or behavior at32/128 bots.
One sample per N cannot estimate run-to-run variation; medians within a run are
not independent repetitions. N16 preceded N1, with no randomized order.

N1 Stop left zero active scripts, V8 isolates/heap bytes and inflight snapshot
bytes; retained RSS was171008000bytes. N16 retained433799168bytes with the same
logical cleanup. These are single teardown observations, not a repeated lifecycle
plateau. V8 logical heap fields in the evidence do not explain resident RSS.

Next obtain independent evidence review, then prepare a bounded per-bot ownership
attribution proposal covering purpose, lifetime, sharing and CPU consumers. No
specific Rust optimization or additional live cell is released by this report.
All remaining panel/latency/lifecycle/scaling and final Grok4.6 gates stay open.

Recomputation source: `diagnostics/current-tui-n1-1727/recompute-n1-n16.py`.
Numerical ledger: `current-tui-n1-n16-attribution-evidence.json`.
