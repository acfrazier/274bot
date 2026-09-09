# Native nav uniform-tile census: independent report

Status: design-selection evidence for `t_71e88dd7`, not implementation release or
memory acceptance. One already-completed real-input census was audited locally;
no census rerun, new input scan, Cargo build, remote command, profiler or live
workload was run on this card. Companion: [tiled-storage design](nav-tiled-storage-design.md).

## Provenance and independent checks

Evidence root is `diagnostics/nav-census-preparation/` in this checkout. Exact
real receipt: `real-result/real-census/nav-census-g0lkh5lm/receipt.json`.
`real-result/real-run-launch.json` and `real-run-completion.json` bind the same
arguments, start time and boot; completion records exit 0. These are the actual
filenames (not a nested `real-run-launch/completion.json`).

| Identity | SHA256 / revision |
|---|---|
| Retrieved real-result archive | `bd376c12339f0a6a7c4c1bd66960ea7c6bdf70726f07066d87358837a1b69f1a` |
| Retrieved qualification archive | `a1ca00e4b6c1f15629beffd6e0db32b4ffbd5e1bff0994a14b963530d410a478` |
| Real receipt bytes | `f7ea748812be097f83fd3e321173890eb5f2bbc1dc847bb01cf96d11294399a8` |
| Real navpack, 73,438,581 bytes | `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30` |
| Native census executable, 1,149,608 bytes | `c2e83b352abd6608a560fb26f59b2ade09ffe664c1d6bd8d20dded27ee7c367e` |
| Reviewed diagnostic tooling | `3d32602a7151f15089a286e6d9375a6ad3d746da` |
| Frozen decoder host | `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf` |
| Frozen compiled client | `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| Frozen source-input digest | `3d89f9277d7f09b8baf161a89f1f1bd2a9c02ffb57dc3ce7a88084c7e7958fd1` |
| Stage-manifest file | `3d1478b07faa54184f290d9de4e24e74e3c10254f3446eb80067c5e046a6d26a` |

The source-input digest and stage-manifest file hash are different identities.
The native receipt reports frozen host HEAD, not the later diagnostic-tool
commit; the staged diagnostic files and their manifest bind that tooling.
Parent `t_c20cf6f3` approved 3d32602 in actual Grok 4.5/xai session
`20260908_221202_bf852a`, before root's native qualification/release.

This independent pass ran the local read-only helper
`python3 diagnostics/nav-census-preparation/independent-report-audit.py` (exit 0).
It hashed both archives and the real receipt, recomputed A/T/D and histogram
sums from the complete saved JSON, and independently rebuilt the source digest
from local frozen Git blobs: 41 host inputs plus 172 client inputs, all 213
also equal current working bytes. The digest framing is ordered filename, NUL,
file bytes, NUL, host list then client list; prefixes/extensions match the
reviewed `build.rs:93-109`. No decoder execution was needed. `git diff` of nav
and api against the frozen host was empty. Current host-consumer audit anchor
is `f2195a1ce186ac63b4c5516ea80d8bd5736b9ae0`; the consumer design uses current
host-play/panel source, not a claim those later consumers ran in the census.

Archive hashes establish identity of retrieved evidence, not independent
execution of native tests or a rehash of the remote executable/input. Those
bindings come from saved native build and supervisor receipts. No allocation
trace field ownership is inferred from them. The helper is local diagnostic
scratch, not a third committed deliverable. Initial execute_code and inline
Python tool invocations were blocked before execution by headless approval;
the file-based read-only audit completed, without changing approval settings.

## Native qualification, including failures

All paths in this section are beneath `linux-qualification/`.

- `build/build-receipt.json`: first offline locked release build exited 101.
  `build/build.log` identifies missing locked `zlib-rs` through flate2 1.1.10.
  It was not a successful initial build.
- `fetch-locked.json`: `cargo fetch --locked` exited 0, source pre/post verified.
  This repaired dependency cache availability only. No source or limit change
  is recorded. `build-after-fetch/receipt.json` records subsequent offline
  locked release build exit 0 in 56.09864664077759 seconds, executable identity
  above, and `source_pre_post_verified=true`. Its status correctly says
  `built_not_qualified`, not live or performance acceptance.
- `qualification/receipt.json`: seven stock fixtures, zero failures/errors/skips;
  native generated-plane input also passed the supervisor's non-fixture REAL
  identity-binding path (exit 0). This is generated input, not the real census.
- The same wrapper's overall status is FAILED. Its broad receipt glob matched
  both the child receipt and the outer rejection receipt after the RSS case.
  Do not erase that assertion failure or describe this as a clean first pass.
  `qualification/rss/nav-census-7nqwlkuh/receipt.json` separately proves the
  400 MiB allocation child was killed by RSS guard, returncode -9.
- `qualification-remaining/receipt.json` preserves/references that prior result
  and continues only unfinished CPU/AS qualification: CPU guard rejection,
  outer exit 1 at 30.083648681640625 seconds; AS child observes soft/hard limits
  both 536,870,912 bytes, rejects a 600 MiB allocation, exits 23 and yields
  supervisor failure/exit 1. `address/nav-census-90u_168g/address-proof.json`
  is the direct limit/rejection proof. Final status is
  `qualified_generated_only`, not a replacement for the failed original wrapper.

This supports the root's release of one real input using the same qualified
binary and unchanged limits. The card does not independently rerun stock tests,
kill/reap probes or qualification children. Reviewed success-path process-group
cleanup is parent evidence, not newly observed lifecycle proof on this card.

## Real receipt and accounting

Linux `6.8.0-139-generic`, x86_64, glibc 2.39; supervisor status `ok`,
`fixture=false`, exit 0, elapsed 0.5047980620001908 seconds. Limits remain
128 MiB input, 30 CPU seconds, 60 wall seconds, 384 MiB RSS, 512 MiB child AS,
1 MiB combined output budget. Recorded stdout is 6,097 bytes, stderr zero.
Sampled RSS is 149,487,616 bytes and sampled address space 150,958,080 bytes;
`sampled_rss_is_not_cumulative_peak=true`. These are sampling observations of
this diagnostic including its input/decode overlap, not runtime steady RSS,
not a cumulative peak guarantee and not candidate savings.

Origin `(1856,1280,0)`, width 1,792, height 9,088, four levels. Both dimensions
are multiples of 32, so this input has no partial boundary tiles. Flags absent.
Graph: 2,143 ordinary edges, 39 teleports; banks: 68. These counts do not measure
the nested allocations or indices and are excluded from the element estimate.

| Level | Total tiles | Uniform tiles | Dense tiles |
|---|---:|---:|---:|
| 0 | 15,904 | 14,402 | 1,502 |
| 1 | 15,904 | 15,019 | 885 |
| 2 | 15,904 | 15,455 | 449 |
| 3 | 15,904 | 15,650 | 254 |
| Sum | 63,616 | 60,526 | 3,090 |

Actual offline walk length/capacity = 65,142,784 u8 elements. Actual offline
blocked length/capacity = 1,017,856 u64 elements (8,142,848 element bytes).
This directly binds fields/capacities in this decoder instance only.

    A = 65,142,784 + 8 * 1,017,856 = 73,285,632 bytes
    T = 4 * ceil(1792/32) * ceil(9088/32) = 63,616
    D = 1,502 + 885 + 449 + 254 = 3,090
    directory = 8*T = 508,928 bytes
    dense payload = 1,152*D = 3,559,680 bytes
    hypothetical candidate elements = 4,068,608 bytes
    potential element reduction = 69,217,024 bytes

Thus A = 69.890625 MiB, candidate elements = 3.880126953125 MiB,
reduction = 66.010498046875 MiB. Each level's partition and all arithmetic
reproduce independently and agree with `real-result/root-recompute.json`.
The predeclared 16 MiB screen in
`saved-prefix-symbol-attribution-design.md:123-132` is met. This selects ONE
32×32 representation design; it does not pass allocation/RSS/runtime gates.

## Reporting defect and why the selection still holds

The raw field `uniform_pair_histogram` is mislabeled: all 512 entries count
ALL VALID CELLS by `face | blocked<<8`, including cells in dense tiles.
Its sum is 65,142,784, not 60,526 uniform tiles. The frozen diagnostic
`src/main.rs:132-140` increments inside the cell loop; tile uniformity is
computed separately by `first`/`same` and counted once at `148-161`.
`root-recompute.json` explicitly preserves the root's initial failed assertion
expecting a uniform-tile histogram, followed by the source-corrected accounting.

The required uniform-tile-by-pair breakdown was NOT delivered. It cannot be
recovered by dividing cell bins by 1,024: dense-tile cells contaminate each bin.
This is a real reporting-contract shortfall, not a clean census pass in every
respect. The immutable raw report/tool are unchanged. This report interprets
the existing field correctly rather than pretending it was fixed.

The shortfall does not undermine the fixed screen: A is measured capacities;
T is geometry; D comes from a separate full-pair equality test per tile. The
8-byte uniform descriptor can encode every one of the 512 possible pairs and
its size is independent of their popularity. No dictionary-frequency or
uniform-pair-specific compression claim is being made. Source inspection and
saved per-level totals support selection with this explicit limitation. This
is independent saved-output arithmetic/source review, NOT a second independent
scan proving D against the real pack. Exhaustive candidate-vs-dense data
validation remains mandatory after a separate release.

## Not established

No candidate allocation/layout has been instantiated; no resident reduction,
CPU improvement or startup reduction is measured. Headers, allocator rounding,
conversion/input overlap, raw debug sidecars, graph/bank indices, paint reach
storage and page retention remain outside the screen. Earlier heaptrack trace
IDs remain unbound to these fields; numerical equality is not historical field
identity, per-bot ownership or free-event proof. Existing Arc sharing is a
source fact, not evidence of N duplicate maps removed. N1/N16 slope, lifecycle
plateaus, JagFX and final target-hardware budgets remain open. Review here is
accounting/design review only; implementation needs the companion's explicit
API decision and a separately authorized, frozen experiment.
