# Native navigation differential: complete bytes, partial route coverage

The Linux dense/tiled comparison passed for every cell in the frozen **274**
pack and the 40 predeclared route selectors. Both processes produced exactly
2,119,504,803 identical bytes. This closes the tested full-pack storage/wire
comparison; it does **not** establish successful teleport or bank-fetch coverage,
Stage A/B performance, or a resident saving. Independent report review is pending.

## Frozen inputs and execution

| Item | Identity |
| --- | --- |
| Tool | `760d3acbc429931447454af41902e43de8ac5b2e` |
| Dense host | `29b7aea779322c8611f83dc193e939ca7d756f75` |
| Tiled host | `8385babb23fd15b876506d4a3f6154984a6b2df1` |
| Same client | `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| Actual pack | 73,438,581 bytes; SHA256 `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30` |
| Real selectors | 40 rows; SHA256 `7ffaf7e73b02c79b3b079e57df99a31924f404a8cfdbae3bf7fe60a9c72b78b7` |
| Root authorization | SHA256 `45351c23027d8d8558c3bb9d2e04634d8bb0267972ce5a167d3a7499702c44e2` |
| Native platform | Linux 6.8.0-139, x86_64, glibc 2.39; builder boot `d63ba1f3-9677-4e61-9d4c-f14a8cf502ac` |

The pack was originally staged from `/Users/acfrazier/.274bot/274bot.navpack`
for the prior census. Root rehashed that local file again when the operator
asked which revision this was; its identity still matches. The isolated 289
client campaign has not replaced this pack or the frozen client submodule.

The existing reviewed run-03 archive supplied source/corpus bytes to a new
native directory, without old Mac targets, admissions, or results. Root verified
all 13,412 staged files before native builds. The independent targets built
offline, each with exit 0: dense 30.514 s and tiled 30.391 s. Original source
hashes and new executable admissions were verified again after execution.

The native tool directory is
`/home/builder/274bot-campaign/nav-differential-760d3ac-1125/host/docs/memory/nav-tiled-differential`.
The real comparison used its `native-run/real-release`, created once by the
reviewed wrapper. Root released correctness only in host STATE commit `2df31b9`.
No retries, cap changes, production startup, or real-input tool changes occurred.

## Native qualification before the real pack

All 12,953 generated inputs passed on Linux. Complete dense/tiled output was
306,044,401 bytes, SHA256
`cbb6f4e9e5fa4f8cc48ae811388a4f81f1f5a9d935a24361ef2ff5c3fbb3ffe0`,
identical to reviewed Mac run-03. The generated one-input protocol fixture
produced 128,271 identical bytes, SHA256
`52327362cb8d91460a0dd45fbc19845f9cb340339d0d19503f5ce88c5b525d68`.
Root independently checked every input length/hash, selector/tool hashes,
source/executable admissions, full byte equality, and frame/input inventory.

The dedicated native **guard suite** ran 18 tests in 2.154 s, all passing with
zero skips. `native_real_qualified` is true. This is the guard suite count,
not the larger Mac all-test count in the tool implementation report. Native
guards include actual hard address-space enforcement; no Darwin AS skip was
imported as qualification.

Root additionally exercised the positive Linux `real` wrapper on the generated
459-byte `routes-open.bin`, with the unchanged REAL limits. A separate owned
copy of the qualified native artifacts and admitted binaries was used, without
recompilation. Its results match the generated protocol fixture byte-for-byte.
That directory and authorization explicitly say **generated fixture only**;
the executable argument `real` does not turn it into production data.

## Real result and independent audit

The pack contains 65,142,784 logical cells: origin (1856,1280,0), dimensions
1792 × 9088 × four planes. Its wire has 2,182 edges including 39 teleports.
The comparison includes full derived collision reads, blocked words, canonical
encoding, decoded roundtrip reads/encoding, ordered graph/requirements/banks,
and original frozen router/host route calculations for every selector.

| Observation | Dense | Tiled |
| --- | ---: | ---: |
| Exit code | 0 | 0 |
| Complete output bytes | 2,119,504,803 | 2,119,504,803 |
| Guard wall time | 122.083 s | 131.346 s |
| Sampled group peak, diagnostic only | 305,471,488 B | 167,051,264 B |

The wrapper completed in 254.722 s. Limits remained 900 s wall, 800 s process
CPU, 1 GiB sampled group RSS, 4 GiB process AS, and 4 GiB output per arm.
These debug diagnostic timings and peaks are **not** matched runtime or cold-load
performance results. They include exhaustive serialization/roundtrip work,
different representation access costs, and diagnostic buffer overlap.

Root separately compared both entire files, rehashed the input/selectors and
admitted source/binaries, and parsed the framing. There are two complete
977,141,760-byte cell frames and two 8,142,848-byte blocked-word frames. Both
canonical encodings contain 73,438,581 bytes and match the input hash for this
particular canonical pack. Generated tests separately cover noncanonical input.
All 40 fixed-selector frames match the predeclared rows; the final completion
frame is present. Full output SHA256 is
`44f55bfaa3493aec13d334a2ad19709dcc566c8c2cb25484bd5ad75b2305973d`;
real result JSON SHA256 is
`a04699c6654ef074bacd16c21898b3173c721ab5c15936dd498c519fca3ff182`.

## Route coverage and remaining gaps

Actual results include successful long walks, Door, Ladder, Stairs, Glider,
session-gated EssenceExit, radius approaches to a blocked destination, and
NoPath controls. The no-session essence-exit case fails while the latched
Aubury-session host route succeeds. Varps distinguish the glider requirement.
Running/walking explicit model results are compared; the copied host calculation
intentionally keeps its original running model.

The mainland glory destination and toll-gate cases took ordinary routes, even
with teleport/bank options enabled. **No successful Teleport leg or BankSession
was observed.** Setting an option is not positive execution coverage of it.
The fixed state puts item 1712 in equipment while these pack teleport edges
require it as an item. The fixed quest name `Rune Mysteries` also differs from
the actual entry requirement `Rune Mysteries Quest`; those entry cases prove
missing-requirement behavior only. These are corpus limitations, not established
production router defects. No selectors were changed after either arm started.

The operator has supplied `~/experiments/rs2b0t/e2e/nav*` to strengthen this
coverage. Static mapping task `t_3cd7b8af` will identify concrete routes and exact
state prerequisites before a separately reviewed/released next proof. Do not
execute the foreign live harness incidentally: it includes server/account setup.
The design's remaining successful bank/teleport proof and Stage A/B stay open.

## Retained evidence

Local artifacts live in
`diagnostics/native-nav-differential-preparation/results-760d3ac/` in this checkout:

- `native-run.tar.gz`: 32,608,837 bytes, SHA256
  `234202ea8551503bff537d65f21c5f347b36db0bc59222dbb5198781ddcbdc70`.
  Contains all 13,437 archived run files, including full generated/real outputs,
  frozen source and input bytes, and receipts. Targets/binaries are excluded;
  their native hashes remain in admissions.
- `native-metadata.tar.gz`: 72,498 bytes, SHA256
  `41fb41cdb0cfd656e058175d3d89c2456162cf5fb197cae7f1e113fe26c62ebb`.
  Contains 31 supplemental root/native guard/generated admission receipts.
- Companion manifests list every archived entry hash/length. Root verified all
  entries locally by streaming both archives: 4,949,483,502 bytes of main
  evidence plus 1,281,699 bytes of metadata. The verification receipt is
  `root-local-archive-verification.json`; small metadata is extracted in `metadata/`.
- `root-real-audit.json` and `metadata/root-real-predeclared-plan.json` retain
  complete selector-to-outcome summaries, scope, limits, and known coverage gaps.

Real input/raw artifacts stay in local diagnostic storage, not public Git.
Mac evidence and prior failures are unchanged. The separate 289 trace worker
ran on Mac during this Linux correctness work; no clean performance claim is made.
