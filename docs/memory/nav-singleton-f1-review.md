# Native singleton F1 evidence review

**Task:** `t_96957a0c`
**Reviewer:** profile `grok46` (artifact-only independent review)
**Branch:** `codex/memory-diagnostics` (workspace HEAD not moved; tools reviewed at `7965c4cbe037c8c7ba0b2b0a1317f852890df4d7`; root report at `03df80d`)
**Scope:** F1 identity, authorization/contract, collect/bind, recomputed costs, qualification-06 bindings, and one next bounded decision. Not F2/acceptance, not live/native/SSH, not a 5%/p99 verdict, not direct-owner artifacts swept into `03df80d`.

## Verdict

**F1 evidence accepted. No evidence blocker.**

Four real probes completed in the authorized order, under unchanged child limits and headroom, with supervisor-inclusive charged cost 29.501340 CPU / 40.090448 wall. Independent archive, contract, selector, collect, and hot-route recomputes match the root report. This is F1 feasibility only.

## Recommended next bounded action

**Release only a generated implementation/experiment for the already-reviewed coordinate-lookup refinement first.** Do not launch 8385 F2 from this review. Do not waive gates or change search/layout.

Row 2 hot-route CPU is a visible cost concern (tiled 3.426256 s vs dense 3.069559 s, +11.620%), present on all three lanes (+14.431%, +9.247%, +13.962%). Two rows cannot establish a regression, and the design’s redundant index/division is source-level, not proved executed cost. A generated clean comparison is the bounded way to test that hypothesis without spending the remaining cumulative F1+F2 envelope (1759.910 wall / 1470.499 CPU / 114 children against 1800/1500/118). If that experiment shows no useful benefit, park the refinement and then finish current 8385 F2 under those remaining caps. Root decides; this card launches nothing.

## Independent verification

| Check | Result |
| --- | --- |
| F1 archive `concord-singleton-f1-evidence.tar.gz` | SHA256 `102c6d41806620247e3c9d23f19a743f4503ca53b556afd4d7da9f05851cf031`, 173779 B, **144** tar members. Extracted tree 144 files. Manifest lists 143 payload files (does not list itself); every listed hash matches disk and tar. |
| Tool/binary package `concord-singleton-7965c4c.tar.gz` | SHA256 `330985b48e016c96dc53455d730ef4bb41c307463c0f13869154e05c8644e8b1`, 5890431 B, **942** members. |
| Authorization | SHA256 `d6da3b78dfdc419c53dea6c25bc131aebea9c3a380f7e12d858d2b4f0d194820`. `schema=stage-a-singleton-v2`, `phase=F1`, `mode=real`, `released=true`, scope F1-only. |
| Original pack / full selector | Contract `input_sha256=2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`, `input_bytes=73438581`, `routes_sha256=49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125`. Concatenation of the 59 archived normalized shards hashes to that same TSV digest. All four probes report `input_bytes=73438581`. Pack bytes are not in the 173779 B archive (expected). |
| Production identity | dense `29b7aea779322c8611f83dc193e939ca7d756f75`, tiled `8385babb23fd15b876506d4a3f6154984a6b2df1`, client `3456edc8dabf7b25ada78110ffa56327af9f67a4`. |
| Caps | Child `wall=120,cpu=90,rss=1GiB,address=4GiB,output=256KiB`; headroom 80 wall / 60 CPU; ceilings F1 `[480,360,4]`, F2 `[1800,1500,118]`. Unchanged vs `7965c4c` `sharded.py`. |
| Claim / checkpoint / result | Claim SHA `f25ef0b0…` status `claimed`. Checkpoint SHA `b1d72bfc…` `validated` completed 4. Result SHA `30d9e6ec…` `complete` completed 4, scope `feasibility only`. |
| Four entries / order | `000-1-1-dense`, `001-1-1-tiled`, `002-1-2-tiled`, `003-1-2-dense`. Slot/name/file SHA/record bindings match `bind_entries`. |
| Collect | Reimplemented `validate_data` / `raw_samples` / aggregate equality against raw `.out` files. 24 raw samples per child, schema `stage-a-raw-v1`, order `sweep-row-lane/8/3`, p99 matches nearest-rank. Dense/tiled aggregates equal per row. Lookup checksum `1227404844284` and `lookup_count=1572864` identical on all four. |
| Selectors | 59 distinct 10-int rows, 32 B each, hashes equal `contract.shard_sha256`. Row 1 `3222 3218 0 3213 3424 0 0 0 0 0`; row 2 same coordinates with `state=3`. |
| Qualification 06 | Five steps returncode 0: guards 2.880 s, clean 218.344 s, counting 219.141 s, integration 15.096 s, scheduler 224.324 s. `qualified=true`, `native_hard_as_qualified=true`, 12 comparisons, scope `generated/local tooling only`, platform Linux glibc 2.39. Each `old_sha256` equals the Linux f24 reference manifest (not macOS run-05); each `new_sha256` equals the archived qualification `.out`. |
| Tool bindings | Package + q06 + F1 contract scheduler hashes equal `git show 7965c4c`: `sharded.py` `8bfda30e…`, `test_sharded.py` `849d31dd…` (7965 fixture, not 007), `test_sharded_guards.py` `45dd91f9…`, `qualify_sharded.py` `e55bbf57…`. |
| Reference alias | Evidence `native-reference-manifest.json` byte-identical to `root-linux-reference-manifest.json`; 12 local `linux-f24-reference/` outs match that manifest. Authorization `root_reference_review_sha256` equals `nav-native-reference-admission-review.md` `168ad018…`. |
| Admissions | Four Concord admission SHAs in the authorization match members of the 7965c4c package (not stored in the F1 evidence tar). |
| Preflight | Concord Linux, idle 97.99%, steal 0, no competing work, available 976994304 B. |
| Direct-owner files in `03df80d` | Out of scope; not reviewed. |

## Recomputed costs

Hot-route CPU is `phases['hot_routes']['cpu_ns']`. Whole-child CPU is receipt rusage. They are not interchangeable.

| Row | Dense hot-route CPU (s) | Tiled hot-route CPU (s) | Change | Dense child CPU (s) | Tiled child CPU (s) | Dense wall (s) | Tiled wall (s) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1.713325 | 1.755984 | 2.490% | 2.859623 | 3.340869 | 2.388497 | 2.764473 |
| 2 | 3.069559 | 3.426256 | 11.620% | 4.716646 | 5.718851 | 3.857028 | 4.626719 |

Lane CPU (ns), diagnostic only:

| Row | Arm | Lane 0 | Lane 1 | Lane 2 | Sum (s) |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | dense | 536883000 | 631750000 | 544592000 | 1.713225 |
| 1 | tiled | 558774000 | 645297000 | 551816000 | 1.755887 |
| 2 | dense | 521262000 | 1576356000 | 971854000 | 3.069472 |
| 2 | tiled | 596487000 | 1722115000 | 1107540000 | 3.426142 |

Process CPU minus hot-route (construction/warmup/lookup/drop, not the primary metric): row1 dense 0.452 s, tiled 0.795 s; row2 dense 0.575 s, tiled 0.988 s. Tiled `decoded_converted_retained_input` is ~0.42 s vs dense ~0.06 s. That construction extra must not be folded into the hot-route delta.

Charged F1 budget (supervisor + waited children + 0.1/1 publication tail): CPU 29.501340 s, wall 40.090448 s, supervisor CPU 12.647823 s, waited-children CPU 16.753525 s, unknown charge 0, stopped false. Per-launch child_cpu sum 16.635989 s is a different rusage delta and is not the waited-children field. Checkpoint after the fourth child is 27.387942 CPU / 35.042910 wall, before the publication tail.

Headroom: max child wall 4.627 s << 80; max child CPU 5.719 s << 60; all receipts `returncode=0`, `address_guard_active=true`, limits exactly the contract.

Construction-complete peaks: dense 149553152 B, tiled 153616384 B, delta 4063232 B (< 8 MiB). Diagnostic only; not a six-pair peak gate. Startup peaks ~15.2 MiB both arms.

No 5%/p99 classification and no all-59 projection are made. Result scope is `feasibility only`.

## Confidence and uncertainty

High confidence that this F1 run is the authorized four-probe feasibility on original pack/59-row selector with 7965c4c tools and the Linux f24 reference alias. Independent collect from raw outs agrees with root readback; hot-route figures agree to the nanosecond.

Medium confidence that the coordinate-lookup refinement will move row 2’s +11.620%. The slowdown is real on this pair and all lanes, but two rows are not a regression, inlining may already remove the redundant arithmetic, and row 2’s `state=3` vs row 1 `state=0` already changes absolute cost on both arms.

Residual: original 73 438 581 B pack bytes were not locally rehashed (absent from both the F1 evidence tar and the 5.9 MB tool package). Identity is contract-bound plus per-probe `input_bytes`. That is not a blocker. Generated q06 smoke lives outside this archive; real F1 child outs were collected here.

## What this does not approve

- F2, acceptance, or any real further pack schedule
- 5% CPU / 2 ms p99 / +8 MiB peak gates
- Treating whole-child or construction CPU as the primary metric
- A new search, layout, allocation, or public API change
- Relabeling 8385 binaries after a later refinement
- Direct-owner native preparation files

## Verdict line

**F1 evidence accepted.** Recommend generated coordinate-lookup experiment first; remaining 8385 F2 caps are recorded, not released.
