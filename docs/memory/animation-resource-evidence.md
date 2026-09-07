# Animation sharing: resource and allocation evidence

The private animation-base change remains a provisional candidate. Repeated N1
short runs show lower resident memory beyond their observed within-role spread,
but their CPU screen is inconclusive. One longer N1 pair also shows lower RSS
and CPU. A separate native capture confirms removal of the selected repeated
clone allocation stack. None of these results establishes final performance,
latency, lifecycle, rendering, or hardware acceptance.

## Frozen sources and live execution

Client implementation `e17deab9085b97b0af42dc1329d7b88a0c7cb467` and its
ownership/behavior tests were approved by Grok4.5 task `t_db97aa30`, run 242;
see [animation-base-sharing-report.md](animation-base-sharing-report.md).
The matched manifest is
`diagnostics/matched-animation-build-20260907T122005Z/build-manifest.json`.
Its reference explicitly reuses the appearance candidate from client `85266df`,
including shared navigation and the common TUI input-origin hook. It is not
the pre-navigation or pre-appearance reference. The candidate adds private
animation sharing; host source fingerprints, toolchain, features, allocator,
fixtures, and runtime settings match. The reused binary/source lineage is
recorded in the manifest rather than attributed to the later checkout HEAD.

TUI SHA256 reference:
`9eb187b7064e0b1bf10bc82722d2ed1210189ee0a87987399006591dcad8bdc7`.
Candidate:
`31c92ffa53fa33795143190693b7fe2b94acbc2263334dc6c1b22c6a22ae521c`.
The corresponding two panel binaries are frozen but were not exercised by
these TUI resource cells.

Short batch `diagnostics/animation-resource-screen-20260907T122122Z/batch.json`
declared N1 then N16, reference/candidate/candidate/reference at each N,
30s warmup, 120s observation, and the existing 60s normal teardown. These are
real PTY TUI active-sustain runs with input probes, System allocator, and
`memory-profile-no-alloc`. Hot profiles, allocation counting, stack logging,
verbose output and diagnostic sidecars are OFF. The same immutable server
identity and host-condition sidecars bind every clean cell. Campaign workers
were checked idle before each launch. Unrelated user/OS activity remained;
this is not a machine-idle assertion.

All eight managed processes completed exit0. Seven independently qualified;
the last N16 reference did not. Managed process exit0 alone is not workload
success. The failed binding invocation exited1 and its rejection is retained.

| Cell | N | Median RSS MiB | CPU cores | Minimum steal gain | Qualification |
|---|---:|---:|---:|---:|---|
| reference_a | 1 | 292.65625 | 0.037252 | 8 | qualified |
| candidate_a | 1 | 278.09375 | 0.039256 | 11 | qualified |
| candidate_b | 1 | 273.203125 | 0.038904 | 15 | qualified |
| reference_b | 1 | 293.5 | 0.035874 | 6 | qualified |
| reference_a | 16 | 832.367188 | 0.318123 | 4 | qualified |
| candidate_a | 16 | 794.65625 | 0.321232 | 5 | qualified |
| candidate_b | 16 | 797.65625 | 0.320817 | 4 | qualified |
| reference_b | 16 | excluded | excluded | 0 | failed |

The N1 minimum reference median minus maximum candidate median is 14.5625 MiB,
versus 4.890625 MiB largest within-role spread. CPU's observed candidate/reference
ratio range is 1.044355–1.094287, crossing the 1.05 margin. It is inconclusive,
not a demonstrated >5% regression or an accepted CPU pass. The three qualified
N16 rows do not form a completed repeated comparison; no N16 screen is claimed.
`descriptive-resource-results.json` explicitly records this incomplete group.
The unchanged full-quartet `analyze_batch.py` main rejects the incomplete batch;
the N1 group was computed using its `analyze_group` function.

## Preserved reference workload failure

Raw run `20260907T130136Z_tui_n16_active`, receipt
`cells/reference_b_n16_animation/receipt.json`, rejected binding
`reference_b-n16-rejected-binding.json`: ordinal1 `live14789_1` had 11 steals at
both observation endpoints. It remained Running, ingame, scene_state2, with
no reported runtime error. Client loops advanced 5231→10303 and dispatches
200→400, but tile (2662,3310), three lobsters, 330 coins, closed/unloaded bank,
and the banking-for-food paint state were unchanged. This is evidence of a
workload stall, not its diagnosed cause. It occurred on the reference binary.
No retry replaces it and no CPU/RSS result from it is used for comparison.

## One bounded longer N1 confirmation

Batch `diagnostics/animation-resource-confirmation-20260907T131042Z/batch.json`
was declared after the short result: reference then candidate, 120s warmup,
600s observation, existing 60s teardown, same frozen binaries/settings and
profiles OFF. Both completed exit0 and independently qualified with native
ordinal proof and continuous managed resource coverage.

| Role | Raw run | Median resident bytes | CPU cores | Steal gain |
|---|---|---:|---:|---:|
| Reference | `20260907T131101Z_tui_n1_active` | 316751872 | 0.032437940 | 62 |
| Candidate | `20260907T132501Z_tui_n1_active` | 297598976 | 0.027702589 | 55 |

The difference is 19,152,896 bytes (18.265625 MiB); candidate/reference CPU
is 0.85401813. Observation sample spans are 599.143s/599.291s and client
mainloop rates 40.954/41.811 per second. These are one-pair point estimates
without a repeat-variance estimate. They do not erase the earlier short CPU
uncertainty. Reproduce with that batch's `analyze_pair.py`; it freshly binds
both receipts and checks identities, chronological separation, runtime/native
settings and helper accounting. All acceptance flags remain false.

During the candidate cell the operator arranged an RDP connection to the new
Windows target. Root performed small LAN reachability checks and read-only
remote desktop/tool inspection; no campaign builds, tests, reviewers, or
native allocation tools ran during observation. This additional interactive
activity is retained as an ambient limitation of this one pair.

## Separate native allocation confirmation

Batch `diagnostics/animation-allocation-confirmation-20260907T131732Z` used the
same frozen candidate, one real N1 active-sustain TUI, 30s warmup/180s observe,
normal teardown, stack logging lite ON and input probes OFF. It completed
exit0, independently qualified, with 18 additional steals. Raw run:
`20260907T133932Z_tui_n1_active`. `capture_owner.py` took one vmmap summary
and malloc-history all-by-size capture 60s into observation, with exit0,
recorded tool timings, before/after active samples and verified capture hashes.
This perturbed diagnostic is separate from the clean resource pair.

`analyze_animation.py` freshly binds this capture and the earlier N1 attribution
from `incremental-owner-attribution-20260907T095008Z`, verifies capture hashes,
and applies the prior animation-family filter while excluding VM/thread maps.
Only addresses are normalized; full grouped symbol stacks are retained in
`animation-allocation-comparison.json`.

| Animation allocation attribution | Historical | Candidate |
|---|---:|---:|
| Total live bytes in selected animation stacks | 27,809,440 | 5,630,992 |
| Live allocation records | 504,109 | 48,585 |
| Selected nested-Vec clone stack bytes | 21,485,568 | 0 |
| Selected clone allocation records | 454,618 | 0 |

The animation-family allocation difference is 22,178,448 bytes (21.151 MiB).
Candidate stacks also show 391,040 bytes/8,858 allocation records under
`AnimBase::new`, including nested vectors. Those records are not a count of
base objects. Direct `unpack` stacks combine Arc and transform allocation
sites, so no exact surviving Arc/base-object count is inferred. Per-unpack
sharing and final-owner drop are separately established by the private tests.

The historical capture used client451759f2 and predates appearance boxing and
the TUI origin hook; the new binary includes those plus animation sharing.
Animation representation/cache behavior was unchanged by those intervening
changes, but the capture pair is not a clean matched performance experiment.
These are allocation-stack totals, not resident savings or per-bot costs.

## Decision and remaining work

Provisionally retain this bounded private representation, supported by approved
behavior/ownership tests, concrete allocation removal, and the descriptive
resource observations. No further short or longer repeat stage is scheduled
for this candidate. Full performance claims remain parked pending current
profile-ON overhead and latency non-regression, absolute targets, the behavior/
lifecycle/rendering matrix, reference environments, and whole-branch Grok4.6.
The known GPU overlay baseline failures remain failures, not renderer proof.

The operator supplied a native x86_64 Windows production laptop, primarily used
through RDP, with Linux-on-x86_64 as a preferred next environment over Mac CPU
emulation. Windows setup and measurement support are next; neither Windows nor
Linux target acceptance is claimed here. The final Grok4.6 review is still
required after the remaining campaign work.
