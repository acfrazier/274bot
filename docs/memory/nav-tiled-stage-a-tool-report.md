# Stage A tooling report

## Corrective implementation — t_fa5a3462

Status: final local generated tooling qualification passed; independent same-card
review pending. NO real navpack read/run, SSH/native Linux execution, game,
account, frontend or performance acceptance. No production code changed.
The rejected aab094f6 report below is retained as historical evidence, not an
accurate description of the replacement tool. Interrupted Luna main.rs edits
are preserved as `nav-tiled-stage-a/prior-attempt-main.rs.txt` and `.diff`.

### Delivered and verified

- Independent immutable dense 29b7aea779322c8611f83dc193e939ca7d756f75 and
  tiled 8385babb23fd15b876506d4a3f6154984a6b2df1 nav/api sources, identical
  client 3456edc8dabf7b25ada78110ffa56327af9f67a4. No live path dependency.
  Original Git blobs, host helper spans, lockfiles, compiler binaries, diagnostic
  tools, source trees and four executable hashes are admitted. Separate clean /
  counting release target per arm; admitted builds refuse recompilation.
- Exact original host route/bank/radius helper spans and immutable ccff4bb facts
  0..6. JSON/TSV selector validation fails closed. The three explicit lanes match
  the correctness tool: model-only, tele-model+facts, host options+facts+radius
  with bank 995x10/1712x1. This is NOT an invented combined model+host API.
- Explicit System global allocator; separate opt-in counting variant. Actual
  directory/pool boxed lengths, requested bytes, usable allocation sizes and
  layout assertions. Narrow warmed collision reads exclude route workspaces and
  observer allocations; their counting results are not timing/RSS evidence.
- Child-side cold file read and completed decode/conversion, retained input,
  input-dropped/world-live, hot lookups, warmed route batches, last-world-drop and
  post-drop milestones. No encode allocation/output or retained dense export.
  Current resident bytes come from Mach task_info or Linux statm; ru_maxrss is
  separately named peak, including explicit pre-input startup peak. CPU is whole
  probe user+system; monotonic operation durations exclude marker dwell.
- Copied ccff4bb bounded supervisor preserves process group cleanup on success,
  timeout, nonzero, output overflow, early EOF and leader exit with inherited
  pipes. CPU/AS/core/file guards are actually set; Darwin AS remains unqualified.
  RSS polling is a group bound, not a hard instantaneous cap. Limits were fixed
  before final runs: 120s wall, 90s CPU, 1 GiB sampled RSS, 4 GiB Linux AS,
  256 KiB output. No failed run caused automatic budget relaxation.
- Real-mode capability exists but requires an explicit root authorization and
  its SHA256, hardware/order, reviewed-correctness release, native guard evidence,
  both generated qualifications, four admissions, tool/manifest and input/route
  hashes. The entire release/copy/launch/paired-aggregate path was exercised with
  generated stand-ins only. No root release is manufactured by the tool.

### Actual execution evidence

Final run `nav-tiled-stage-a/run-04`, guards `guards-04`:

- Rust 1.98.0 (88d9e12ae), Cargo 1.98.0, aarch64-apple-darwin,
  macOS 15.7.9; full version and executable hashes in prepared/admission receipts.
- All four release builds passed: dense clean 70.043s, tiled clean 71.260s,
  dense counting 69.820s, tiled counting 75.515s. These are build durations,
  not performance measurements.
- 16 Python unit/guard tests passed in 2.099s with no skips. They assert effective
  CPU/core/file limits, platform-specific AS status, CPU/RSS/output failures,
  EOF/leader/descendant cleanup, unsafe input/selector rejection and baseline
  repeat-noise math. Linux allocation-denial branch exists but was not executed
  on this Mac; native_hard_as_qualified=false is retained explicitly.
- Both arms × both variants × all-uniform/all-dense/gated synthetic packs passed
  phase/counter/layout and paired numeric aggregate checks. Each run consumes
  840 timed route calls. Uniform/dense: 48 successful routes, 792 NoPath,
  288 walked tiles. Gated: 280 consumed routes (bank results include final
  routes), 592 NoPath, 32 BankSession results and 256 transport legs.
- Candidate uniform layout: 16 directory entries / 128 requested bytes,
  zero pool entries. All-dense: same directory + 16 pool entries / 18,432 bytes.
  Gated: 4 entries / 32 bytes, zero pool. WorldCollision size 120 bytes,
  DenseTile alignment 8. Reported usable sizes equal those requested on this
  local allocator; these numbers are NOT resident accounting or real-pack sizes.
- Narrow counting read windows observed zero allocations/requested bytes in
  both arms for all three generated fixtures. Positive counting self-test also
  passed, preventing an inactive allocator from qualifying as zero allocation.
  Clean counters are inactive and their zero fields carry no allocation claim.
- Four Rust self-tests passed: facts 0..6, empty/nonempty aggregate consumption,
  essence option and allocator mode, current-vs-peak semantics. For dense clean,
  mapped/touched/unmapped current RSS was 2,818,048 → 36,372,480 → 2,818,048;
  the peak remained 36,372,480. This is a synthetic sampling test, not nav saving.
- Integration tests passed: both-arm source/lock/tool/binary mutation rejection,
  four malformed inputs per arm, invalid radius per arm, phase reordering,
  release-binding mutations, and a full twelve-process JSON-selector stand-in
  AB/BA release run. Its cold-load and route-CPU gates classified inconclusive
  from baseline repeat noise. Peak/p99 arithmetic reported pass on the tiny
  fixture; NONE of these labels is real Stage A acceptance.

Archive `nav-tiled-stage-a/evidence/run-04.tar.gz`: 183 evidence files,
177,882 bytes, SHA256
`adfc37caf581aeac755f019dd0e0e5b11c8c4bb1cfa75abaef1c5c3953a323f0`.
Every archived member was read back and hash-verified. Adjacent inventory and
summary include raw build/launch receipts, phase streams, failures intentionally
exercised, source/admission manifests, stand-in results and guard output.
Bulky materialized source/targets remain ignored locally; source inventories
and frozen original Git commits enable reproduction. Earlier successful
`run-03` remains locally as pre-warmup/native-guard-hardening evidence, not final
qualification. Rejected aab094f6 and interrupted changes were not rewritten.

### Remaining limitations and release boundary

The proposed manifest freezes six repetitions per arm and AB/BA order, peak
increase <=8 MiB, cold median <=10%, route CPU <=5%, route p99 <=2ms, with
baseline max-minus-min repeat noise yielding inconclusive results. Fresh process
ownership is not cold filesystem cache; no cache flush or allocator purge occurs.
Route timing includes scalar aggregation/result destruction and getrusage observer
overhead. The three API lanes have deliberately different semantics, documented
per field. Collision-read allocation evidence is bounded warmed lookup evidence,
not a claim about all allocations during route search. Exact live layout plus
verified original conversion/drop source establish absence of a retained dense
shadow; no per-allocation ownership tracer has been invented for route workspaces.

Native Linux guard and real-input/hardware/corpus qualification remain root-owned,
after correctness gates and this review. Sampling can miss short spikes; retain
the separate kernel process HWM and never substitute ps peak for current RSS.
Allocator usable bytes do not include all page rounding/resident retention or
graph/bank/Arc overhead. Post-drop resident pages need not return to the OS.
No Stage B, resident saving, or campaign resource acceptance is claimed.

Tool-policy restrictions blocked execute_code and a Python -c inspection call;
ordinary checked-in Python scripts supplied the real results instead. No
fabricated outputs, production edits, delegation, remote writes or live runs.

## Historical rejected report (aab094f6; superseded, retained verbatim)

Status: prepared tooling only; no real pack, native host, SSH, account, game,
server, cache, frontend, or performance run was executed.

Files are confined to `docs/memory/nav-tiled-stage-a/`. The Rust probe admits
only the two explicit synthetic modes (`all-uniform` and `all-dense`), rejects
malformed or unsupported ten-field selectors, and records state provenance for
presets 0..6. Presets 4 (`563x50,556x150,554x50; level 6=25`), 5 (carried
1712x1 only), and 6 (995x5000 only) are retained as distinct accepted selector
values; they are not remapped to the original bitmask.

The proposed manifest uses nine fresh processes per arm/cell, alternating dense
and tiled order, separate targets, System allocator, release mode, and fixed
wall/CPU/RSS/output limits. The predeclared Stage A thresholds are peak increase
<=8 MiB, cold-load median increase <=10%, route CPU increase <=5%, and route p99
increase <=2 ms. Repeat noise can classify a comparison inconclusive.

Accounting is deliberately milestone-based. The input byte buffer is retained
through decode/conversion and dropped after world drop; route results are reduced
to counts and a tick sum. The probe reports logical cell and blocked-word
counts, not an invented candidate allocation estimate. Collision-specific
allocation cannot be isolated from route workspace allocations here, so no
zero-read-allocation claim is made. The counting-allocator variant remains
separate and is not mixed into clean timing/RSS.

Qualification evidence is limited to source compilation and Python validation;
local synthetic output, when run, is tooling qualification only and cannot
establish memory savings or Stage A acceptance. Darwin has no hard address-space
qualification in this tooling. A future real-input launcher must add explicit
root authorization binding path/hash/source/lock/tool/binary and an owned output
directory; this card does not provide that release.
