# Stage A: native qualification passed; first real baseline did not complete

The first real 59-selector baseline process ended with SIGKILL after 90.231s,
under a declared 90s hard CPU limit. It emitted completed load/input-drop/lookup
phases, but no completed hot-routes marker or summary. The wrapper exited1,
preserved the failure, and stopped the schedule: no candidate and no real
counting diagnostic ran. Stage A is unresolved; this is neither a tiled failure
nor a matched performance result. Do not rerun this release or increase its caps.

## Qualification and provenance

Reviewed tool `f24de7cbcaaa3f419fc5485b77c4a117543a6afc`, same-card
`t_fa5a3462` round3 APPROVED actual Grok4.5/xai
`20260909_092339_3d490f`. The between-repetition input/route binding defect in
rejected6c5a867 was corrected, independently reproduced and requalified.

Frozen dense29b7aea779322c8611f83dc193e939ca7d756f75,
tiled8385babb23fd15b876506d4a3f6154984a6b2df1,
client3456edc8dabf7b25ada78110ffa56327af9f67a4. Original host158/client196
Git objects were separately packed and independently verified. All four Linux
builds use original production function bytes plus admitted diagnostic modules,
release/System for clean, separate counting targets, and original lock provenance.

Both the builder and Concord passed the17-test guard suite with no skips and
`native_hard_as_qualified=true`, all-uniform/all-dense/gated generated clean and
counting qualification, and the complete12-process generated stand-in release
protocol plus source/lock/tool/binary mutation and Rust self-tests. These are
qualification results, not real performance acceptance. Concord has no compiler;
all496 transferred files and four executable hashes were verified without a
rebuild or rewritten admission. Builder and Concord evidence are distinct.

| Binary | SHA256 |
|---|---|
| Dense clean | fe0c89bf4c46abed700852f120412dbff673afad4ce1067a53e22fbac63ce43a |
| Tiled clean | 08d92e97132b40cbc74ed55698c24a39c1dd95be8cd064413408ba0e3d98aa48 |
| Dense counting | a08fcee760428f803892f24231efd92ffc1f0debc96343c58cf0a5f5bfd1d6fd |
| Tiled counting | 15dd17b1017e9f2f75976bd276a12c8a29f8240f3f254ace97cbaabb30317e4e |

## Frozen real attempt and failure

Root release STATE211fe20, authorization SHA
2c14748c27676284462b89088dfd5984345c547c406509e3109b40182df17edf.
Original274 pack73438581 bytes, SHA
2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30.
All reviewed40+19 selectors were concatenated unchanged in original order,
SHA49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125.
The full row provenance is in `diagnostics/nav-stage-a-real-proposal-01`.

Concord Linux6.8.0-139/glibc2.39,2CPUs,2014852KiB RAM, no swap,
boot2217ec26-dc3e-47a3-a700-d3fafb34163f. Preflight13:46:36Z had967068KiB
available,14667911168B disk free,98.16% idle and0steal over3s, no competing
campaign process. An existing separate service remains resident (~642MB);
it was not stopped or tuned. This is not the whole application reference profile.

Fixed six processes per arm, alternating AB/BA, with120s wall/90s CPU/1GiB sampled
RSS/4GiB AS/256KiB output per process. No limit was increased. The first process
`00-dense` returncode-9, failure`exit`, output916 bytes, sampled group peak
149721088B, hard-AS active. Final process CPU was not captured, so the exact kill
source is not directly observed. The timing and configured limit are consistent
with the hard CPU guard. Postflight13:50:52Z records global OOM kills since boot0,
959576KiB available,0swap and no remaining probe processes. No OOM claim follows.

The partial stream records retained-input read61.558ms and completed
decode/conversion/world construction70.346ms, input dropped while world lives,
and hot lookup40.363ms. These are single incomplete-process diagnostics, not
accepted cold/CPU/RSS values. There is no marker separating route warmup from
the timed route batch, so the partial stream cannot identify which route or
which of those two intervals was active at termination. No route mismatch was
reported; there is also no completed real route aggregate from this run.

## Preserved primary evidence

Under `diagnostics/nav-stage-a-native-preparation` in this checkout:

- `builder-native-evidence-f24de7c.tar.gz`:5507455 bytes,667 verified files,
  SHA5b82a2c856485adc4e76cb06a760050455849885c2853acbad02c3bf1e8bb621.
- `concord-stage-a-attempt-01.tar.gz`:5198802 bytes,678 verified files,
  SHAabffe5b9c367b44ebbd0456beeb7dec9e9f986de0ce096ead5c9a4281870f943.
  Includes the complete original input copies, all Concord qualification,
  source/binary admissions, authorization/preflight, failed process output and
  receipt, wrapper failure, and postflight. Original Git pack files are in the
  separately preserved builder/source payloads.
- Companion inventories and `root-*-archive-verification.json` record independent
  full member length/hash checks. `builder-metadata` and
  `concord-attempt-01-metadata` contain readable extracted small evidence.
  Full input/binary bytes remain in the archives; no recapture is required.

## Proposed next decision: isolate the existing startup/load gate

This is a proposal for independent review, NOT another measurement release.
Do not retry the failed59-case schedule, relax caps, alter routers or rebuild
these binaries for this proposal.

Use the SAME admitted probes and original pack in a new owned, relocated copy
of qualified source/binaries/admissions. Select ONLY the existing reviewed row40
from the earlier40-case corpus:

`3222 3218 0 1855 1280 0 0 0 0 0`

Its destination is outside this pack (origin1856,1280), and all three original
API lanes returned NoPath in the preserved correctness proof. This deliberately
fast control permits the unchanged full probe to finish load, lookup, route and
world-drop markers without the expensive59-case routing workload. Preserve the
original field meanings, model/host lanes, state and zero bank-fetch option.
No new CLI mode, diagnostic source edit or production change is needed.

Before launch, freeze a NEW authorization/route hash/hardware preflight and
same6-per-arm alternating order. Keep all existing caps. Copy qualification
receipts with explicit relocation provenance and verify all admitted bytes;
do not overwrite old runs, rebuild or re-admit binaries. Both arms use this
identical one-row control. Run the full clean schedule once; optional separate
counting diagnostics may check actual layout/narrow reads only after it succeeds.

Evaluate ONLY the already-declared startup peak (+8MiB) and cold-load median
(+10%) gates with the same baseline-repeat/noise policy. Keep all other emitted
metrics as diagnostics; the one-row route CPU/p99 is NOT the59-case route gate
and must never be promoted to full Stage A or performance acceptance. Cold
ownership is fresh-process input/decode; OS file cache is not flushed.

If either startup/load gate clearly fails, park/review this representation on
that necessary gate without spending more on a long routing schedule. If these
gates pass or are inconclusive, Stage A remains open and the full routing work
needs a separate, explicitly reviewed resource/phase plan. Do not infer a pass,
raise the route budget, select a favorable route subset or restart until favorable.
This is a phase-specific screen of an existing necessary gate, not a replacement
for the failed full-corpus attempt or for StageB resident/lifecycle acceptance.
