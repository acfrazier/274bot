# Existing Heaptrack owner replay: capability audit and bounded design

Status: design for same-card Grok 4.5 review, not permission to implement or run
production replay. Task `t_227f43d3`; branch `codex/memory-diagnostics`.
Only the already saved owned Python smoke was probed. No native child, remote
analysis, game, new capture, Rust change, optimization, or acceptance occurred.

## Decision

**Yes, format 3 retains enough information to reconstruct an unsuppressed live
requested-byte multiset by allocation stack at a specified record prefix or an
existing timestamp mark.** It does not retain individual allocation identities
in interpreted output. `+` and `-` reference a reusable `(requested size, trace)`
descriptor, not a pointer or unique allocation. Counts per descriptor suffice
for stack-family byte totals; treating those IDs as unique live objects fails.

This offers a real way to distinguish later captured allocation-stack families
from the startup-dominated global peak using existing data. It is not a finding
that a particular private owner is material. Source mapping of surviving stacks
must still distinguish private/shared/mixed/unresolved families. No thread ID,
bot ID, Rust object graph, publication epoch, or ownership transfer is encoded.
N1 does not turn a process-shared allocation into a private per-bot allocation.

**No, the existing failed N1 capture cannot establish observe-end, script-Stop,
post-join, or snapshot-epoch ownership.** The sole attempt failed the raw-size
guard at 589.61 seconds, before observation-end and Stop. Root's outcome remains
FAILED. Replay can produce only explicitly labelled captured-prefix diagnostics;
it cannot accept the partial capture, measure savings, or close the ledger's
lifecycle discriminator. Do not request another capture as a substitute here.

The existing global peak (root evidence: 14,903 rows, 194 positive,
160,842,811 bytes) is dominated by startup `NavWorld::load_pack` file read
73,438,581, walk decode 65,142,784, and blocked decode 8,142,848 bytes. A later
prefix can answer a different stack-population question without summing
independent peaks. Actual production prefix values remain **unmeasured**.

## Primary source and local evidence identity

Source examined is the official KDE release archive:
https://download.kde.org/stable/heaptrack/1.5.0/heaptrack-1.5.0.tar.xz

Downloaded archive SHA-256:
`a278d9d8f91e8bfb8a1c2f5b73eecab47fd45d0693f5dbea637536413cec2ea5`.
Extracted read-only audit cache:
`diagnostics/heaptrack-replay-source-audit/heaptrack-1.5.0/`.
`CMakeLists.txt:18-23` declares Heaptrack 1.5.0, file format 3. Saved raw and
interpreted smoke both begin `v 10500 3` (hexadecimal numbers). This matches the
recorded installed 1.5.0 release/format, not a claim to have reproduced the
installed distribution binary or ruled out distribution patches. The smoke
contract comparison below independently checks its emitted event semantics.

Source anchors below use the official KDE `v1.5.0` tag; line numbers are from
that release archive. The analyzer downloaded separately from the tag is
byte-identical (`cmp` exit 0) to the archive. Critical source SHA-256 values:

- `src/analyze/accumulatedtracedata.cpp`:
  `d8e8e640cd8e678a9c7eb2bc5d6b65e8b7caa4da0caf8564d28a15c79db69cd9`
- `src/interpret/heaptrack_interpret.cpp`:
  `6fbc20705764f7f33bd3863c4d50728b0ad496f4b620940290ec3d3eea12d3ed`
- `src/track/libheaptrack.cpp`:
  `7ac6b30d046b897dcc92b0e7fc1db7d08a3edfd575eb0e59ba16953b75c4b0c5`

Evidence root is `diagnostics/owner-capture-evidence-2015/`; its
`owner-capture-evidence-2015-manifest.json` binds included files and remote-only
inventory. No remote artifact was opened by this task. Inventory, not locally
recomputed production hashes:

| Remote-only artifact under `current-owner-heaptrack-n1-2003/allocation/` | Bytes | Manifest SHA-256 |
|---|---:|---|
| `alloc.raw` | 2149010072 | `16f51266e56bd065f891ad822a00659a3bd0803aef8f748c82cde5e492b5ca29` |
| `alloc.interpreted` | 810563095 | `cb005f5e7bae1869fbd5c5552fb4e5499fed6b25c43b74c9964c1d11f0ed076e` |

Full local saved smoke is `heaptrack-seam-smoke-2001/capture/`. Its raw,
interpreted, peak-stacks, printer log and interpreter log were rehashed against
the manifest by the companion probe. Exact hashes are in
[existing-heaptrack-owner-replay-probe.json](existing-heaptrack-owner-replay-probe.json).
The frozen runtime remains host `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`,
client `3456edc8dabf7b25ada78110ffa56327af9f67a4`; reviewed capture tooling is
`1fe21610b6446779133b5ff0dc229ba93599ecc0`.

## Record contract, with primary anchors

Base for source links: https://github.com/KDE/heaptrack/tree/v1.5.0

| Record / behavior | Meaning and replay consequence | Primary anchor |
|---|---|---|
| `v version format` | Hex values; require exactly `10500 3` for the initial implementation. No format autodetection from filenames. | [CMakeLists.txt:18-23](https://github.com/KDE/heaptrack/blob/v1.5.0/CMakeLists.txt#L18-L23), [analyzer:449-469](https://github.com/KDE/heaptrack/blob/v1.5.0/src/analyze/accumulatedtracedata.cpp#L449-L469) |
| `s byte_length string` | Implicit 1-based string IDs, 0 absent; length is bytes, not Unicode codepoints. Preserve spaces and full symbols. | [LineReader:54-125](https://github.com/KDE/heaptrack/blob/v1.5.0/src/util/linereader.h#L54-L125), analyzer:243-270 |
| `i address module [function [file line [inline-function inline-file inline-line]...]]` | Implicit 1-based IP IDs, 0 absent. Function-only records are legal; raw unresolved address/module records are legal. Preserve missing locations rather than fabricate source lines. | [interpreter:403-415](https://github.com/KDE/heaptrack/blob/v1.5.0/src/interpret/heaptrack_interpret.cpp#L403-L415), analyzer:283-305 |
| `t ip parent` | Implicit 1-based trace-node IDs; parent 0 ends the chain. Node is the allocation-side frame followed toward callers through parents. Trace IDs remain tied to the raw trace-tree sequence while IPs become symbolized IDs. | [interpreter:561-571](https://github.com/KDE/heaptrack/blob/v1.5.0/src/interpret/heaptrack_interpret.cpp#L561-L571), analyzer:271-282 |
| `a size trace` | Implicit **0-based** descriptor ID; deduplicated by exactly size and trace, not allocation instance. Same descriptor can have thousands of simultaneous live allocations. | [PointerMap/AllocationInfoSet:23-78](https://github.com/KDE/heaptrack/blob/v1.5.0/src/util/pointermap.h#L23-L78), analyzer:399-410 |
| Raw `+ size trace pointer` to interpreted `+ descriptor` | Adds one multiplicity and the requested size. Pointer map is maintained only during interpretation. New descriptors are emitted before use. | [interpreter:572-589](https://github.com/KDE/heaptrack/blob/v1.5.0/src/interpret/heaptrack_interpret.cpp#L572-L589) |
| Raw `- pointer` to interpreted `- descriptor` | Removes one currently mapped pointer and emits its original size/trace descriptor. Unknown raw frees are silently dropped. Interpreted replay must decrement a positive multiplicity, never delete a descriptor definition. | [interpreter:590-606](https://github.com/KDE/heaptrack/blob/v1.5.0/src/interpret/heaptrack_interpret.cpp#L590-L606), [PointerMap:118-157](https://github.com/KDE/heaptrack/blob/v1.5.0/src/util/pointermap.h#L118-L157) |
| `c elapsed_ms` | Integer milliseconds since Heaptrack initialization using `steady_clock`, not Unix time. Timestamp output takes the same serialization lock as allocation records. Timer sleeps 10 ms between attempts, not a guaranteed 10 ms cadence. | [tracker:72-82](https://github.com/KDE/heaptrack/blob/v1.5.0/src/track/libheaptrack.cpp#L72-L82), tracker:260-283,386-397,718-737 |
| `R rss_pages`, `I page_size physical_pages` | Separate process/system metadata, not allocator payload. Never add RSS to requested bytes. | tracker:399-445,498-502; analyzer:426-434,470-472 |
| `X command`, raw `x executable`, raw `m module` | Identity/symbolization metadata. Single owned process only; no bot/thread identity on events. Do not propagate raw command lines into durable reports. | interpreter:532-560; tracker:453-495,574-637 |
| `A` | Attach marker resets first-pass total and marks `fromAttached`. It is **not** an allocation ID, phase mark, or shutdown record. Reject in this direct-preload-only design. | analyzer:444-448 |
| `S suppression` / comments / blank lines | Embedded leak suppression, or non-cost metadata. Comments at the interpreter's end count strings/IPs, not a debuggee completion certificate. | analyzer:412-414,473-483; interpreter:607-609 |

### Pointer reuse, realloc, and coverage caveats

Normal free interception records the free **before** libc free to avoid reuse
races (`heaptrack_preload.cpp:218-234`). Pointer reuse after a recorded free is
legal and is validated in the smoke. In contrast `PointerMap::addPointer`
overwrites an already-live address (`pointermap.h:129-134`); a strict raw audit
must reject that inconsistency, not inherit the interpreter's silent overwrite.

[preload:236-249](https://github.com/KDE/heaptrack/blob/v1.5.0/src/track/heaptrack_preload.cpp#L236-L249)
calls real realloc first and reports only a non-null result.
[tracker:806-823](https://github.com/KDE/heaptrack/blob/v1.5.0/src/track/libheaptrack.cpp#L806-L823)
serializes successful realloc as `- old` then `+ new-size/new-trace/new-pointer`,
even when the address is unchanged. Replay must use these deltas, not a guessed
resize record. Failed nonzero realloc emits nothing and retains the old tracked
allocation. A libc `realloc(p,0)` that frees and returns null is **not reported**
by this hook, so stale tracked bytes are possible. Calling libc realloc before
the bookkeeping also differs from the free-before-libc protection; this audit
does not prove absence of concurrent pointer-reuse races in the collector.

No realloc opcode or return code survives in interpreted records. They cannot
identify which `-/+` pairs were realloc, distinguish unchanged-address resize
from free/new, or prove logical object continuity. Do not report allocation age,
per-instance survival, snapshot equality, or allocation graph identity from a
size/trace multiset. A family can have the same byte total at two marks with
completely different objects.

[tracker:827-892](https://github.com/KDE/heaptrack/blob/v1.5.0/src/track/libheaptrack.cpp#L827-L892)
shows initialization, pause/resume, null-pointer and recursion guards. Self-
allocations, allocations before hooks are ready, paused periods, custom pools,
non-interposed functions, and untracked mappings are not restored by replay.
The pause API does not emit a phase record. Require direct-preload provenance
and no intentional pause/attach/reinitialization; still describe results as the
**recorded tracked population**, not all live process allocation bytes.
Fork children are disabled (tracker:307-309,604-625); preload clears collection
environment for exec children (preload:194-196). Server/helpers stay outside.

### Time, initialization, shutdown, truncation

There is no arbitrary exact-time query hidden in the CLI. Printer options at
[heaptrack_print.cpp:560-620](https://github.com/KDE/heaptrack/blob/v1.5.0/src/analyze/print/heaptrack_print.cpp#L560-L620)
include no time filter. The analyzer library does have `filterParameters.minTime`
and `maxTime`, but its implementation skips both `+` and `-` outside the interval
(analyzer:238,306-309,358-361,415-425). Starting with nonzero `minTime` discards
allocations that were already live before that time. It is not a baseline-
carrying live-set API and is not the algorithm proposed here.

Use **mark-prefix** semantics: at a chosen actual `c T` record, replay every
preceding event from the beginning and snapshot the multiset at that record.
For a requested number not present in the stream, report the preceding actual
mark and following mark, not an exact population at the requested millisecond.
Events between marks have ordered records but no individual precise timestamp.
Even equal-valued timestamps must retain line/byte-offset identity; milliseconds
are quantized. Before the first mark, events have only the initialization-to-
first-mark bracket. The canonical analyzer assigns events to the preceding mark
(initially zero); its global-peak timestamp is that bucket label, not a precise
allocation wall-clock instant. Replay order is collector serialization order,
not proof of atomic real-heap state across concurrent libc calls.

For **captured-prefix-end**, consume all complete valid records in the saved
file and report the last event offset and last `c` mark separately. If events
follow that mark, their end time has no subsequent bound in the stream. Do not
synthesize the analyzer's `last_timestamp + 1` final callback as an observed
millisecond (`accumulatedtracedata.cpp:486-491`).

Shutdown writes `c`, `R`, flushes and closes, but no unique success/termination
record ([tracker:311-325,354-375](https://github.com/KDE/heaptrack/blob/v1.5.0/src/track/libheaptrack.cpp#L311-L375)).
A final `c/R` is also ordinary timer output. LineReader uses `getline` without
requiring a newline (`linereader.h:29-41`); analyzer and interpreter tolerate
some parse errors and still return success. Buffered writes can leave missing
suffixes on abnormal exit; `LineWriter::flush:247-266` also does not require
`write` to return the full buffer size. Exit 0, a newline, a trailing `c/R`, and
interpreter summary comments are each insufficient to certify capture completion.

Strict parsing must reject malformed/truncated records and cannot silently cut
away an invalid suffix. Whole-record suffix loss cannot in general be detected
from interpreted syntax alone. Hashes bind the saved bytes, not their coverage.
A valid, provenance-bound failed artifact can be discussed only as its recorded
prefix, with `capture_complete=false` and explicit uncertainty about omitted
suffix/events. It is never an accepted lifecycle sample. If raw validation or
conversion reconciliation fails, do not issue even a ranked prefix result.

### Suppressions and canonical printer comparisons

Unsuppressed live requested bytes are the principal replay domain. Keep all
`S` entries and report whether any were present, but do not drop their costs.
Builtin, embedded and user suppressions are applied to **leaks** by
[analyzer:843-943](https://github.com/KDE/heaptrack/blob/v1.5.0/src/analyze/accumulatedtracedata.cpp#L843-L943),
zeroing matched trace costs and subtracting them from `totalCost.leaked`.
This is a presentation policy, not evidence of frees. A future exact canonical
leak comparison must use `--disable-builtin-suppressions
--disable-embedded-suppressions`, no user suppression file, or independently
reconcile all suppressed bytes. Default rounded `407.23K` alone cannot prove
exact equality; raw-event/descriptor accounting supplies the exact smoke value.

The nonmerged global peak is deliberately common-time: first pass finds a
strictly larger global total, second pass copies all trace live costs at that
peak (analyzer:338-356). Reproduce the **first** strict maximum in event order,
not each stack's independent maximum or the last equal plateau. Trace rendering
skips initial operator-new frames and stops at designated frames; keep original
trace IDs as well as normalized printable stacks so rendering collisions cannot
lose bytes. `heaptrack_print.cpp:809-835` writes those per-trace peak costs.

Massif has recursive tree-writing code, but it is not proof of useful owner
children here: timestamp callbacks run on FirstPass only (print:518-525), while
per-trace live costs/`handleAllocation` update on later passes (analyzer:338-344).
The peak snapshot buffer and threshold logic are at print:395-418,506-515.
This explains why an aggregate-looking Massif export is not a demonstrated
boundary owner tree. No new Massif execution or patch is proposed.

## Executed small offline falsification

Command (no game, no profiler, no native child; Python stdlib only):

    python3 docs/memory/existing-heaptrack-owner-replay-probe.py

The companion is intentionally hardwired to the saved smoke and caps every
input at 2,000,000 bytes; it is **not** the formal production parser. Its JSON
receipt is checked in. It hashes inputs, independently replays raw pointers,
compares the entire normalized raw event sequence with interpreted events,
checks references/sized strings/multiplicity, and compares global peak with
the saved canonical flamegraph. It does not pretend to cover adversarial parser
cases or distribution-wide hook correctness.

Observed checks, exit 0:

| Fact | Recomputed smoke result |
|---|---:|
| Raw / interpreted bytes | 389215 / 281592 |
| Allocation descriptors / trace nodes / IPs / strings | 3291 / 13943 / 1972 / 139 |
| Allocation calls | 6388 (canonical printer and interpreter agree) |
| Raw frees / interpreted matched frees | 6356 / 6354 |
| Unknown raw frees discarded by interpreter | 2 |
| Pointer addresses reused after free | 1070 |
| Maximum simultaneous multiplicity of one descriptor | 2048 |
| EOF outstanding tracked allocations | 34 (canonical interpreter agrees) |
| EOF unsuppressed requested bytes | 407233 (printer rounds to 407.23K) |
| First common-time global maximum | 5730024 bytes, interpreted line 27144, previous mark 50 ms |
| Canonical peak stack rows / positive rows | 2951 / 2212; sum exactly 5730024 |
| `PyByteArray_Resize` bytes at that maximum | 4196352 = 2048 * 2049 |
| Actual mark at or before requested 500 ms | 497 ms, line 27231, 5730024 bytes, 4962 outstanding allocations |
| Last actual mark | 1064 ms, line 32299, 407233 bytes, 34 outstanding allocations |
| Bytearray-family bytes at EOF | 0 |

The unknown raw frees mean even this clean smoke is not an all-allocation
inventory. Their omission is explicit and raw/interpreted matched sequences
still agree. Requiring every raw free to have a prior tracked allocation would
incorrectly reject the existing known positive; allowing descriptor underflow
in interpreted output would incorrectly hide a broken replay.

A useful falsification caught an upstream counting distinction: interpreter
reports **637** temporary allocations, printer **638**. Interpreter uses actual
last pointer and resets it to zero; analyzer uses descriptor ID and also resets
to zero, but descriptor zero is valid (`interpreter:596-605`, analyzer:369-389).
The probe reproduces both conventions; it does not force them equal or infer
lifetimes from the temporary count. This does not alter matched-free byte totals.

The earlier 5,435,240-byte smoke is an alternative fixture from the plan, not
mixed with this one. All assertions here use the newer 5,730,024-byte fixture.
The 497 ms result demonstrates recovery of a non-EOF mark population; the EOF
family difference demonstrates why printer leaks are not an active-phase proxy.

## Proposed formal replay algorithm — next task only after actual design approval

1. **Bind input and policy.** Require a fixed manifest with raw/interpreted sizes
   and SHA-256, source/tool/binary identity, collection failure reason, interpreter
   receipt/stderr, suppression policy, and requested cutoff definition. Read only
   the owned immutable saved files. No symbol downloads or re-interpretation with
   a different binary. Verify hashes before accepting results. Initially accept
   uncompressed direct-preload 1.5.0/format-3 only; reject `A`, duplicate `v/X`,
   concatenated sessions, unknown opcodes, malformed records and unsupported
   initialization modes. Preserve legitimate blanks/comments; enforce exact
   numeric field arity and legal optional IP groups, lowercase hex/ranges,
   string byte lengths, bounded UTF-8 rendering, and references before use.
2. **Raw-conversion audit.** In a bounded streaming pass reconstruct raw
   pointer-to-(size,trace), flag already-live pointer overwrite, count unmatched
   frees separately, and hash the normalized sequence of matched `+/-` events
   including timestamp records. Compare it with an independently generated
   interpreted normalized-event digest and record counts. This guards malformed
   raw lines silently skipped by native interpretation. Do not store one entry
   per historical allocation. Unknown raw frees are reported as missing coverage,
   not charged negative bytes; reject unexplained conversion mismatches. This
   cannot prove absence of unrecorded events such as realloc-to-zero or lost
   whole-record suffixes, so the tracked-population qualifier is permanent.
3. **First interpreted pass.** Stream all records, construct append-only compact
   string/IP/trace/descriptor tables and multiplicity counters. `+i`: increment
   count and checked total by `size[i]`; `-i`: require positive count, decrement
   and subtract. Descriptor 0 is valid; trace/IP/string 0 means absent. Parent
   must precede child, preventing cycles. Maintain global peak bytes and the
   first strict-maximum event offset, not copied snapshots at every new high.
   Record timestamp offsets, monotonic nondecreasing times, final event offset,
   EOF counts, conversion digest, maximum counters, diagnostics, table sizes.
4. **Choose cutoff mechanically, not by interesting owner totals.** For the
   bounded production diagnostic default to the last actual timestamp mark in
   the saved stream, plus EOF as a separately labelled comparison; neither is
   called observe-end or Stop. A separately requested capture-relative time
   selects the last mark at/before it, including ordinal if times tie, and reports
   the next mark. Reject negative, out-of-range, or no-preceding-mark requests
   rather than silently clamp. Preserve `requested_time`, `actual_mark`, offsets
   and bracket. Common-time global peak is a validation checkpoint, not a later
   owner-ranking substitute. No scanning all times for the largest private owner.
5. **Second interpreted pass.** Replay deltas from the start, never start at the
   cutoff or zero a baseline at warmup. At the preselected finite set of offsets
   (peak, last mark, EOF; at most four including one explicit requested time),
   aggregate `multiplicity * requested_size` by original trace ID. No per-instance
   allocation IDs are needed or invented. Require count/byte sums to agree with
   global running totals and raw final pointer sums, underflow/overflow absent,
   and both interpreted passes to have the same full hash and counts.
6. **Render/classify only after arithmetic passes.** Resolve allocation-side
   frames through the parent chain with bounded depth and cached provenance.
   Keep exact trace ID, stack, descriptor sizes, allocation count, live count,
   requested live bytes, and cutoff offset. Group into the capture plan's first
   useful frozen Rust function or explicit allocator-boundary family. Retain
   original stack rows and unresolved/mixed rows so every byte belongs to exactly
   one family; do not sum parent inclusive costs. Use full unshortened symbols
   for classification, canonical normalization separately for printer comparison.
   Map source at the frozen commits into ledger domains: shared cache/interface,
   per-client Client/World/entities, GameSnapshot families/publication shells,
   script fingerprint/encoded buffers, nav/status queues, infrastructure, or
   unresolved. A symbol alone never establishes all objects' ownership or
   publication-shell coexistence. Mixed private/shared callsites remain mixed.
7. **Outputs and fail-closed result.** Emit one bounded JSON receipt and full
   bounded TSV stack/family tables for the chosen cutoffs, sorted by bytes then
   stable ID. Receipt includes `capture_complete=false`,
   `population=recorded_tracked_requested_bytes`, `cutoff_kind`, mark/offset/bracket,
   source/input hashes, validation results, unknown-free/unknown-symbol bytes,
   peak/leak comparison, source-owner anchors, process role, and resource usage.
   Never emit a successful partial top-N table if output caps are exceeded;
   failure emits only a small failure receipt. Production expected bytes/counts
   are deliberately not invented. The only known exact production oracle here
   is the saved common-time peak sum 160842811; validate its full normalized
   positive-stack multiset too, not just that scalar.

### Required tests and rejection conditions before production use

These are acceptance tests for the **next implementation**, not tests claimed
executed by this audit:

- Re-run the saved smoke checks above exactly, including 34 EOF allocations,
  407233 unsuppressed bytes, 5730024 peak and 4196352 known family. Compare full
  normalized canonical peak-stack costs, accounting for operator-new/stop-frame
  rendering and stack collisions. Obtain exact unsuppressed canonical leak costs
  from the existing fixture only if needed in that approved task; no new capture.
- Tiny synthetic contract fixtures: descriptor 0; two simultaneous identical
  descriptors; mixed sizes with same trace; free/reuse of an address; same-address
  and moved successful realloc represented as `-/+`; failed realloc/no events;
  zero-size allocation count; zero-return realloc coverage limitation; frees
  after a selected mark; allocation before cutoff still live at cutoff; same
  bytes at two cutoffs from different instances; equal peak plateau uses first.
- Timestamp fixtures: no first mark, repeated marks with distinct offsets,
  nonmonotonic mark rejection, before-first/after-last rejection, event tail
  after last mark, and caller request between marks returning a labelled bracket
  rather than invented exact time. A window-only replay must fail the
  pre-window-survivor fixture.
- Input adversaries: truncated last numeric/string line, incomplete IP triple,
  overlong line/string, extra fields, invalid hex/overflow, unknown/free-out-of-
  range descriptor, descriptor underflow, future/cyclic trace reference, bad
  string/IP reference, changed hash, duplicate session/version, `A`, unknown
  opcode, raw duplicate-live-pointer, mismatched raw/interpreted digest. Each
  must terminate nonzero without owner ranking. Whole-record suffix deletion
  with a changed hash fails identity validation; with a separately authorized
  prefix manifest it remains explicitly incomplete, not magically detectable.
- Suppression fixture must show unchanged live multiset but a different default
  canonical leak presentation. Embedded/builtin/user suppression accounting
  cannot be silently conflated. Preserve 637-vs-638 temporary semantics separately.
- Resource fixtures must exceed each declared table, line, memory, CPU, wall,
  output and free-disk guard and produce a bounded failure receipt, not success.

### Proposed bounds, not measured production requirements

Use a compact-array implementation, streaming input, and no raw/interpreted
copies, expanded event log, per-timestamp owner tree, or per-event backtrace
string. Three sequential passes maximum: raw validation and two interpreted
passes. Time O(raw records + interpreted records + selected output trace depth),
with bounded hash-map operations; storage O(distinct strings/IPs/traces/size-
trace descriptors + maximum simultaneous raw pointers), not O(allocation events).
A descriptor-count design avoids storing millions of historical pointer IDs.

For a later explicitly authorized executor: no more than one analysis child
at a time, no game/build/profile overlap, one worker thread, admission
MemAvailable >=768 MiB and free disk >=1 GiB. Each pass: CPU limit 180 seconds,
wall limit 300 seconds, RSS ceiling 512 MiB, child-only address-space ceiling
768 MiB on supported Linux. Total pass allowance <=900 seconds; stop at first
failure, no automatic retry or limit increase. Check time/resource guards at
bounded intervals (at most 0.5 seconds externally); reap only the owned child.
No environment or limits changed in the controlling process. A platform that
cannot enforce the declared bounds is not a released executor.

Preallocate/charge compact tables against a 256 MiB table budget: at most
2,000,000 trace nodes, 500,000 IPs, 1,000,000 descriptors, 2,000,000 raw live
pointers, 200,000 strings, and 64 MiB total string bytes; the combined byte
budget applies even below individual ceilings. Maximum line 1 MiB, rendered
stack depth 512, numeric fields uint64 with checked multiplication/signed
aggregate range. These are rejection caps, not estimates that production fits.
Input caps raw 3 GiB and interpreted 1 GiB admit the manifest sizes; reject larger
inputs before allocation. Output total <=32 MiB including all stack tables and
receipt, no silent truncation. Scratch <=64 MiB, no new multi-gigabyte derivative.
Require free disk to stay >=512 MiB. Persist resource measurements and cap failures.
The smoke's tiny runtime does not prove these CPU/memory caps sufficient for
production; exhaustion yields a reasoned unavailable result within this design.

## Claim boundary and handoff

The plan and owner ledger are unchanged. The audit narrows an earlier CLI
limitation: a saved format-3 event stream has replayable prefix costs even though
the printer exposes no lifecycle-bound time filter and the global peak is startup-
dominated. That is a capability, not an accepted private-owner measurement.

A reviewed future replay can rank **captured-prefix, source-mappable allocation
families** if validation and coverage hold. It may find only unresolved or mixed
families; then private-owner discrimination is unavailable from these artifacts.
Even a clear private family at the last mark does not prove survival through the
missing observe-end or Stop boundaries, exact vector capacities, snapshot epoch
identity/equality, per-instance lifetime, N16 scaling, or causal RSS ownership.
V8/custom pools, mmap, GPU storage, allocator page retention, RSS and process CPU
remain distinct or unknown. Leaked means recorded-not-freed-at-file-end only.

Verification: companion probe ran exit 0 and regenerated its receipt; primary
analyzer archive/tag `cmp` passed. Source/release hashes and saved-fixture checks
are above. Formal parser implementation, production artifact replay and any
optimization are deferred until actual Grok 4.5 design approval and a separately
scoped task. This task requests that review and stops.
