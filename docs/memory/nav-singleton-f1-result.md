# Native singleton F1 feasibility result

F1 completed all four probes in the fixed order: row1 dense/tiled, row2 tiled/dense.
The original full pack and 59-row selector manifest were admitted; only rows1/2
were executed. Total charged CPU29.501340s and active wall40.090448s include
supervisor and children. All child receipts passed unchanged limits and headroom.
Root revalidated entry hashes, exact schedule, samples and aggregates using the
reviewed tooling. This is F1 feasibility, not a 5% gate or percentile verdict.

| Row | Dense hot-route CPU (s) | Tiled hot-route CPU (s) | Observed change |
| --- | ---: | ---: | ---: |
| 1 | 1.713325 | 1.755984 | 2.490% |
| 2 | 3.069559 | 3.426256 | 11.620% |

These are one pair per row and 24 timed calls per child. Row2 is a visible cost
concern to review before more work, not a statistically resolved regression.
Whole-child CPU includes construction, warmup, lookup and supervision and must
not replace the hot-route metric. No all59 estimate or acceptance projection is
made from two rows. Current coordinate-lookup design has already been reviewed;
root will decide whether to continue8385 qualification or test that refinement
only after this evidence review. No F2, acceptance or refinement implementation
is released by this report.

Source/tool:7965c4cbe037c8c7ba0b2b0a1317f852890df4d7; original dense29b7aea,
tiled8385babb, client3456edc. New probe binaries are admitted derivatives, not
the original failed all-row executables. Native reference uses the reviewed
Linux f24 manifest alias, never macOS run05 identity. Concord qualification06
passed all five steps; scheduler224.324s, native hard-AS true,12 comparisons.

Evidence archive: diagnostics/nav-stage-a-native-preparation/concord-singleton-f1-evidence.tar.gz
(173779B,144members), SHA256
102c6d41806620247e3c9d23f19a743f4503ca53b556afd4d7da9f05851cf031.
Root independently verified exact membership and every file hash; unpacked bytes
are under concord-singleton-f1-evidence. Full admitted binary/tool package is
concord-singleton-7965c4c.tar.gz, SHA256
330985b48e016c96dc53455d730ef4bb41c307463c0f13869154e05c8644e8b1.
All remote evidence and earlier failed qualifications remain intact. The F1
supervisor session exited0; no further phase has launched.
