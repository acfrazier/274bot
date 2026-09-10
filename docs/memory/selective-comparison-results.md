# Bounded main/selected comparison

This was one sequential macOS attempt per build at N1 and N32. Baseline host
`54cfcf8a`/client `4f2048ea`; selected host `9527cc63`/client `daccb4ba`.
The identical temporary Rust driver uses APIs already present on main and the
ordinary Thiever scenario with ten food, level-50 stats and no sustain option.
It runs real clients through headless Play, without a UI. No production change
was backported to baseline. Both executables were built, copied and hashed before
the four cells; the temporary driver was removed from both product checkouts.

Metadata binds executable/source hashes, driver hash, server/catalog revisions,
cache path, fresh selected navpack, argv and isolated HOME/fixture state. No
local compilation or competing selected frontend ran during these cells. The
existing campaign libproc sampler supplies current RSS and Mach-time-corrected
process CPU, with process identity checked on every sample. Each eligible cell
has a fixed 60-second observation after every original scenario reports Passed.
The driver requires every slot to remain ready and Running and to increase
player updates and XP over observation. Stop must leave no slots.

| Cell | Outcome | Mean current RSS | Process CPU cores | Player updates | XP gained |
|---|---|---:|---:|---:|---:|
| Main N1 | Passed, 60.193 s | 289.88 MiB | 0.02999 | 100 | 234 |
| Selected N1 | Passed, 60.143 s | 260.59 MiB | 0.02100 | 100 | 141 |
| Main N32 | Passed, 60.233 s | 3267.72 MiB | 2.63358 | 3200 | 8890 |
| Selected N32 | Failed before observation | Ineligible | Ineligible | Ineligible | Ineligible |

N1's observed RSS difference is 29.29 MiB (10.10%) in this single short pair.
Useful work passed on both builds, but XP productivity differed materially.
The lower selected CPU reading is therefore not an equal-productivity CPU win.
There is no repetition/spread estimate, UI or script latency measurement, scalar
experiment isolation, or large-fleet capacity conclusion. Both sides explicitly
share the seed world through the common API, so this pair does not measure the
retained harness's new seed-sharing benefit. Historical gains are not added to it.

The selected N32 run exited 1 at about 153 seconds: all original scenarios had
Passed, but actor `live13d71_27` was Paused at the immediate pre-observation
readiness check. All actors had recent player updates and no script/start error.
The driver did not retain the exact slot status fields used by its ready predicate.
The bounded diagnosis subsequently found actor 27's valid server save at the
exact Maze random-event spawn `(2891,4597,0)`, matching the server teleport and
unchanged host random-event definitions. This strongly supports an actual Maze
teleport interrupting qualification; it does not prove the exact pause duration
or recovery. No observation interval exists and no N32 savings are accepted.
The failure is retained, with one bounded read-only diagnosis in
[selective-comparison-diagnosis.md](selective-comparison-diagnosis.md). No favorable
rerun or weakened predicate replaces it. Separately, the retained sustained N32
TUI run proved all 32 bots restocked exactly 22 food and increased steals after
return; that functional diagnostic is not a replacement performance comparison.

Raw cells: `selective-integration-evidence/compare-{baseline,selected}-n{1,32}`.
Per-actor deltas, phase times, sample counts and exact arithmetic are in
`selective-integration-evidence/common-comparison-analysis.json`; each cell keeps
`metadata.json`, `markers.jsonl`, `process.jsonl` and `run.log`.
