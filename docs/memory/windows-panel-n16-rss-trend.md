# Native N16 RSS time trend

2026-09-07. This supplements the reviewed two-mode diagnostic; its medians are
accurate descriptions of the sampled windows, but neither window establishes
a stationary RSS plateau. Original recordings and failed host bindings remain
unchanged. This is not a new run or an accepted performance comparison.

Recomputed by `python3 docs/memory/n16_rss_trend.py`; exact source hashes, byte
values, endpoints and row partitions are in windows-panel-n16-rss-trend.json.
Each population contains 119 observe rows. Thirds split consecutive row indices
into 39, 40 and 40 samples; they are a descriptive trend, not a new qualifier.

| Mode | First RSS MiB | Last RSS MiB | First-third median MiB | Middle-third median MiB | Last-third median MiB |
|---|---:|---:|---:|---:|---:|
| One renderer | 1039.652 | 798.863 | 1096.758 | 945.057 | 859.131 |
| Focused plus background | 2474.266 | 1338.422 | 2327.516 | 1900.828 | 1555.229 |

The overall median mode difference is 951.547 MiB. The corresponding descriptive
third-median differences are approximately 1230.758, 955.771 and 696.098 MiB.
These are separate runs, not synchronized observations or a causal subtraction.
Observe begins at elapsed103.116s in the one-renderer cell and122.445s in the
background cell. Do not extrapolate a stable per-renderer cost or asymptote from
these two decaying traces, or select only their lower endpoints as a budget pass.

The RSS decrease is not accompanied by decreasing logical GPU descriptor totals:
one-renderer totals are almost constant and background totals slightly increase.
V8 total heap also ends larger than it began in both cells. That does not locate
the lost resident pages. The recordings do not distinguish CPU allocation release,
allocator behavior, driver allocations and Windows working-set trimming well
enough to assign a cause. Private committed bytes and resident-page ownership are
not supplied by the current process sampler. Its Windows host-pressure field is
explicitly unavailable, not evidence of healthy or zero pressure.

The next owner census must include transition peaks and retained capacities over
time. Collect separately identified native private-commit/working-set evidence
where available, and use the approved longer confirmation to assess stability.
Do not change the existing qualification or budget rules to make these diagnostic
windows pass. Final resource, lifecycle, latency and whole-branch gates remain.
