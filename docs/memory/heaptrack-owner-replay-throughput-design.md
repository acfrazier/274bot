# Saved Heaptrack replay throughput: diagnosis and bounded replacement design

Task `t_491f8993`; design submission, not implementation or executor release.
Branch verified: `codex/memory-diagnostics`. Engine, runner and tests are unchanged
from `378e634b103d525aef33b86d22e8238fa1bddc79`; root's native-result Markdown/JSON
are unchanged from `6bee87a`. Root owns those reports and STATE.

## Decision

Propose a **single-threaded compiled streaming child**, retaining the Python
owned-child supervisor and the existing three-pass/output contracts. Do not ship
the diagnostic monkeypatches as a repair. The measured narrow Python corrections
remove substantial overhead but do not supply a credible capacity margin under
the fixed Concord caps. This is a reason to review a bounded compiled port, not
proof that every possible Python implementation is incapable or that Rust will
fit. No compiled replacement was written or timed here.

The measured bottleneck is record-by-record Python work, including the real
resource guard, not evidence of RAM shortage. Merely sampling the guard is
insufficient. The original production failure remains failed raw pass 1; it has
no owner ranking, completed raw hash, or production conversion validation. The
original capture remains failed at its raw-size guard, without observe-end,
Stop, post-join or publication-epoch ownership. No retry, capture,
re-interpretation, remote operation, host/client change or larger limit occurred.

## Evidence audit against original receipts

Evidence root: `diagnostics/replay-native-stage-378e634-preparation/native-evidence`.
The separate audit script reads bounded local receipts, exported tooling and the
small saved smoke, not the remote production streams. Its machine-readable
results are `heaptrack-owner-replay-throughput-audit.json`.

- Rehashed all **41 manifest entries**, matching every byte count and SHA-256.
  There are 42 files including the export manifest itself: no 41/42 discrepancy.
  The separately rehashed archive is
  `0d741086ccb6d4e53e11d0376d1cdac8bee3909fdf148826d5f184c4bb1bbaf1`, matching root.
- Root JSON's `native_tests`, `native_smoke`, `production_launch` and
  `production_failure` objects exactly equal the corresponding raw receipts.
- `native-tests.log` contains all 21 named tests, all `ok`, no skips, and
  `Ran 21 tests in 13.723s`. The enclosing launch receipt's 13.917025 seconds is
  subprocess elapsed time, not a contradiction of unittest's duration.
- Rehashed all **16 smoke payload files** referenced by its receipt. `receipt.json`
  is an additional file, not a seventeenth file in that payload manifest.
  Smoke receipt verifies peak 5730024 bytes, requested 497/507 ms bracket,
  EOF 407233 bytes / 34 allocations. Native runner reports three completed
  phases, child 2316 reaped, peak RSS 24473600 bytes, Linux monitoring enabled.
- Production launch binds manifest
  `7fcd08aed20e44c81dd495f83ae663b167f4392b84c39a79ce927b45165e1595`, runner 2325,
  available memory 997117952 bytes, free disk 11835043840 bytes, conflicts `[]`,
  and one permitted attempt. Completion says runner exit 1, no launcher timeout.
- Actual failure: `child: CPU guard`, phase 1, child 2326, exit -9, reaped,
  wall 180.095009595 seconds, peak RSS 90357760 bytes, AS 100413440 bytes.
  The 180.095 value is **runner elapsed wall**, not a recorded final child CPU
  value. The CPU reason is supported by the child's real `PhaseGuard.check`
  path; the report does not supply consumed records/offset or final CPU seconds.
  Do not turn elapsed wall into a throughput measurement or percentage completed.
- No material numerical discrepancy found in root's report. Two audit limits
  should remain explicit: the export has no separate raw PID-absence readback
  proving root's `runner_and_child_absent_afterwards=true`, and no independent
  remote directory listing proving that only `failure.json` existed. The owned
  runner's failure cleanup/reap code and exported failure receipt support those
  claims, but this offline audit cannot freshly observe the remote processes or
  directory. Similarly, `conflicts: []` is a saved admission observation, not
  proof of uninterrupted absence throughout the run. No root history was edited.

Native all-21 qualification was real but did not qualify production throughput.
The audit does not rerun those native tests or claim the saved inventory hashes
are newly validated full production input hashes.

## Local experiment and falsifiable hypotheses

Commands from this checkout (no parameters accepting arbitrary trace inputs):

    python3 -B docs/memory/heaptrack_owner_throughput_probe.py > docs/memory/heaptrack-owner-replay-throughput-local.json
    python3 -B docs/memory/heaptrack_owner_throughput_audit.py > docs/memory/heaptrack-owner-replay-throughput-audit.json
    python3 -B -m unittest discover -s docs/memory -p test_heaptrack_owner_replay.py -v

Probe source and engine/runner hashes, platform, individual measurements,
function call profiles and exact result signatures are retained in the JSONs.
Python 3.9.6, macOS 15.7.9 arm64. Each generated or fixed smoke input is checked
against a 2,000,000-byte ceiling before replay. No production file is opened.
Generated fixtures are deleted after use; generator and their hashes are retained.
Measurements are serial, warm-cache, three repetitions per mode. No other test or
benchmark was deliberately run concurrently with these timings; background OS
activity was not controlled. cProfile runs are separate from capacity timings.
An initial exploratory run included a counting wrapper around the baseline guard;
reported final measurements remove that wrapper and bind `PULSE=guard.check`
exactly as the owned child does. Those exploratory timings are not used below.

Ranked hypotheses and observations:

1. **Per-record guard syscalls are material.** `Input.__iter__:141` calls PULSE
   for every line; runner `PhaseGuard.check:146-153` imports resource, calls
   getrusage, monotonic and process_time, and performs three checks every time.
   Compare unchanged baseline with real guard calls every 1024 PULSE invocations,
   still checking every phase boundary. This improves the fixtures but does not
   account for all cost. It is not an unguarded parser shortcut.
2. **Parser dispatch/temporary objects dominate remaining work.**
   `parse_record:82-93` reconstructs an arity dictionary every numeric line;
   `number:34-36` performs Python regex dispatch/cache lookup and int conversion
   on every token. A diagnostic fixed-arity numeric specialization hoists arity
   maps and compiled regex, leaves string/IP/metadata parsing on the original
   path, and keeps length/newline/separator/arity/lowercase-uint64 checks. It
   changes no tracked source. Its positive-result parity is measured, not a
   claim that all adversarial behavior has been qualified for release.
3. **Hashing or file I/O alone explains the failure.** Not supported by these
   warm-cache fixtures: hashlib update and buffered readline are small compared
   with Python parsing and guard work. Cold Concord storage is not measured.
4. **Large pointer state or rendering explains raw-pass failure.** The tiny-map
   churn fixture is already slow. Rendering cannot cause phase-1 failure: it
   occurs in phase 3. Large-map CPU effects remain unmeasured, not ruled out;
   small tests do not prove table ceilings will fit the production populations.

### Unprofiled results

All times below are median process CPU seconds. `sampled` changes only the guard
cadence; `sampled+numeric` adds the numeric specialization. Raw signatures,
including event/trace/metadata hashes, counts and final population, agree across
all measured modes. Saved-smoke first-pass/canonical signatures also agree.

| Fixture | Baseline | Sampled | Sampled+numeric | Baseline / corrected MB per CPU second |
|---|---:|---:|---:|---:|
| 50,000 short-pointer alloc/free pairs, 700048 bytes, 100006 lines | 0.450264 | 0.343656 | 0.264867 | 1.555 / 2.643 |
| 50,000 wide-pointer alloc/free pairs, 1850048 bytes, 100006 lines | 0.462281 | 0.353574 | 0.271128 | 4.002 / 6.824 |
| Saved smoke, all three passes + canonical comparison + output | 0.661865 | 0.528193 | 0.460679 | not a single-stream throughput |

The synthetic cases keep **one simultaneous pointer** and one trace. Their
short/wide tokens deliberately show why bytes/second is workload-dependent.
They do not model hundreds of thousands of current pointers, diverse callsites,
long symbols, table growth or native cache misses. Saved-smoke baseline phase
CPU medians are 0.124322 / 0.173798 / 0.363736; corrected medians are
0.071050 / 0.113146 / 0.276469. Phase 3 includes canonical comparison and writing
all lossless output, not just the second interpreted read. The probe exercises
real phase checks but runs in-process, not through the Linux supervisor; parent
admission, publication rehashing and /proc monitoring are not in these timings.

The tight red-capable guard loop replays only the generated short fixture under
one lowered one-second CPU phase. Baseline emits `CPU guard` at 1.000011 CPU s;
sampled+numeric emits it at 1.001860 CPU s. Its counting wrapper exists to report
PULSE calls for this diagnostic, unlike the direct baseline timing above. This
proves real parser/PULSE failure still occurs; it does **not** prove the sparse
cadence meets a maximum wall latency on adversarial lines or on Concord.

### Call/CPU attribution, not profile-derived capacity

On the short fixture's separately profiled baseline, total raw-pass cumulative
profiler time is 0.882250 s (cProfile's default elapsed timer is not a CPU sampler,
and profiling changes execution cost):

| Function / path | Calls | Cumulative seconds |
|---|---:|---:|
| parse_record | 100006 | 0.352215 |
| Input iterator | 100007 | 0.270394 |
| number (included within parse_record) | 200008 | 0.154860 |
| PhaseGuard.check (included within Input / phase boundaries) | 100008 | 0.149994 |
| emit_digest | 100002 | 0.062401 |
| require | 1800107 | 0.046361 |

Do not sum overlapping cumulative rows. Each line pays multiple Python calls,
checks, list/token allocations, and often a normalized digest packing call.
`re._compile` here is mostly the regex cache lookup path, not evidence of actual
regex recompilation per token. Per-record arity dictionary creation is real.
The saved-smoke profile additionally exposes canonical traversal/classification
and output; `Output.write:616` calls disk_usage for every output row, so a raw-only
optimization leaves phase-3 overhead. Raw per-record calls are nevertheless a
measured reason for a compiled streaming loop, not a request for a broad bot
architecture refactor or more memory.

### What this says about fixed caps

Inventory raw bytes / 180 CPU s require **11.938945 MB/CPU-s**; interpreted bytes /
180 require **4.503128 MB/CPU-s** in *each* interpreted pass, before allowance for
third-phase canonical reconciliation/output. These are necessary average rates
computed from bound byte sizes, not estimates of actual record counts.

Even the corrected wide-pointer, one-live-entry local case is below the raw
requirement. The corrected short-token case is farther below. Concord's saved
smoke raw phase took 0.372113 CPU s, versus this Mac's baseline 0.124322, but that
is a historical small-fixture comparison, not a portable multiplier or prediction.
There is no production progress offset to estimate the needed speedup accurately.

Conclusion: the measured bounded Python changes are not a justified retry
candidate. Further batching/loop fusion might improve Python, so impossibility
is not established. Prefer one bounded compiled-port qualification over an
open-ended sequence of Python micro-optimizations. A compiled port must earn its
own correctness/resource/throughput evidence and may still fail the unchanged
caps. Larger limits, parallel passes, PyPy substitution or a new executor are not
fallbacks authorized here.

## Concrete implementation proposal, after design approval only

### Boundary and interface

Add an isolated diagnostic Rust package under the **proposed new path**
`docs/memory/heaptrack-owner-native/`, with its own `[workspace]`, Cargo.lock and
single `heaptrack-owner-child` executable. Do not add it to the root workspace,
link game crates, or change client/host code. Existing root Cargo.toml supplies
no hashing/JSON dependency policy; the implementation must pin reviewed `sha2`,
`serde`/`serde_json` and Linux `libc` dependencies in this isolated lockfile,
record compiler/target/features and executable SHA-256, and build outside any
measurement. No claim is made that these new dependencies are installed now.

Root's read-only 21:31 UTC inventory reports `/usr/bin/cc`, `/usr/bin/c++` and
`/usr/bin/g++` on Concord, but no rustc/cargo in PATH or `/home/acfrazier/.cargo/bin`,
and no `/usr/include/openssl/sha.h`. Therefore this Rust proposal has an explicit
**native build/toolchain staging prerequisite**, not a deployable Mac artifact.
After design/code review, root must separately approve a Linux Rust toolchain
installation/stage or a compatible Linux builder, pin dependencies/compiler and
prove the exact Linux executable's provenance and target compatibility before
qualification. An arm64 macOS executable is not a Linux build. If root declines
that prerequisite, return the design decision for review; do not silently switch
to C++ or assume OpenSSL development headers exist. C++ with a reviewed pinned
hash/JSON implementation is an alternative design, not an implemented fallback.
Windows/Hyper-V availability does not select either executor preemptively. No
toolchain install, dependency fetch, cross-build or native staging occurred here.

Reuse the Python supervisor's admission, retained direct Popen PID, selectors,
Linux sampling, cleanup and pending-directory publication. Launch the native
executable directly, not a Python wrapper retaining another child or an FFI
per-record callback. It accepts the same manifest path/hash, lower-only limits,
output directory and fixture-mode policy. Freeze an additive native engine
identity field in runner.json; keep `heaptrack-owner-input/v1` and semantic
`heaptrack-owner-replay/v1` output unchanged. Never silently select an engine
based on installed binaries or fall back to the old engine on error. Require
explicit native executable absolute path plus expected hash in the reviewed
launch configuration. Preserve the original Python engine as the small-fixture
reference, not a production auto-retry path.

The child independently validates the manifest/duplicate JSON keys, frozen
provenance, receipt/stderr, saved printer options and exact counters. Set
child-only rlimits before trace open, preserve one thread, and use the current
bounded phase 1/2/3/4 + done protocol. No per-record IPC, Python objects or event
journal crosses the boundary. Failure may add bounded last-completed-line,
byte-offset, record counters and CPU/wall values to the error object; supervisor
must validate and retain those optional fields under the protocol cap. If the
child is killed before reporting them, record progress unavailable, not guessed.
Never include raw X command lines or pointer addresses in failure metadata.

### Exact three-pass state machine

1. Validate regular-file/no-symlink/declared-size policy and strict v10500/format3
   grammar. One reusable buffered byte reader and at most a one-MiB pending line;
   hash each input byte in its consuming pass. Incremental SHA-256 may batch
   bytes without changing digest order. No prehash, whole-file mmap, extra raw
   read, event log or re-interpretation. Check dev/inode/size/mtime/ctime around
   each read pass and the full hash/byte length at EOF.
2. Raw: pointer -> `(size, trace)` for current pointers only. Reject zero or
   already-live pointer and forward trace parent/reference. Unknown frees count
   separately; matched frees use original size/trace and remove current pointer.
   Match the exact big-endian `opcode + u64...` event and trace digests, and
   length-framed X/I/R/S metadata digest. Preserve raw temporary-pointer semantics.
   Verify EOF pointer byte sum and count. Release the pointer arena before
   interpreted pass 1; carry only bounded scalar/digest results forward.
3. First interpreted pass: append original UTF-8 strings, IP variable groups,
   parent trace nodes and distinct size/trace descriptors; keep live/call counts.
   Descriptor zero is valid; other table zero sentinels remain absent. Require
   every definition before use. Checked signed aggregates and checked uint64
   fields; no wrap, saturate or float conversion. Track first strict global peak
   position only, last actual mark, last event and one optional requested bracket.
   Never copy the live table on every increasing peak. At EOF reconcile raw
   event/metadata/trace digests, definitions count, counters and population.
4. Select only first peak, last actual mark, EOF and at most one declared requested
   time. Duplicate timestamps retain ordinal/offset identity; request selects
   last mark <= request and first subsequent greater mark. No scan for interesting
   private owners, no baseline reset or invented last+1 time.
5. Second interpreted pass: reset counts, validate every definition against its
   retained table, replay from the beginning, snapshot only those selected
   positions. Aggregate exact descriptor/trace costs with checked arithmetic.
   Reconcile all pass results and full input hash against the first pass. Reuse
   identical-position snapshots where applicable without changing cutoff labels.
6. In phase 3, compare **all positive full canonical stack costs** against the
   already bound peak oracle, adding canonical rendering collisions only for
   this comparison. Preserve original strings/trace identities for ownership and
   lossless output. Retain Heaptrack operator-new rewrites throughout the chain,
   STOP names, inline/source basename formatting, templates/operators, zero trace
   `??`, unresolved addresses and IP-zero termination. No scalar-only oracle.
7. Classify with the existing frozen RULES/DOMAINS and first-useful-frame policy,
   checking unresolved frames across the entire original stack. Keep broad Client
   construction mixed, source domains distinct from instance ownership, unknown
   bytes explicit. Preserve suppressions without applying them. Write identical
   deterministic TSV schemas/sort ties/JSON-escaped UTF-8 cells, all descriptor
   and zero-byte live counts, and receipt claim boundaries. Publish only after
   complete child success and parent full file/hash/budget verification.

### Data layout and resource plan

Use contiguous typed arrays/arenas, not per-record owned strings. Parse numeric
fields directly from borrowed byte slices: exact ASCII lowercase 1..16 hex,
checked accumulation, exact separators and arities, sized strings measured in
bytes with UTF-8 validation. Do not use whitespace splitting that accepts tabs,
uppercase, signs, 0x prefixes or extra spaces. IP optional groups keep original
arity and inline order. A native port must not inherit a more permissive JSON or
number parser by accident.

Charge capacities **before** growth, with checked byte calculations, alignment,
container metadata and transient old+new growth buffers included. Concrete table
shapes: strings `(u64 offset, u64 length)` plus byte arena; IP offset/length pairs
plus u64 field arena; traces `(u64 ip, u64 parent)`; descriptors size/trace/live/
calls u64 columns plus per-trace calls. Current-pointer slots hold u64 key/size/
trace and explicit occupancy metadata; descriptor uniqueness slots hold
size/trace, with full-key comparisons, never hash-only identity. Use a seeded
bounded open-addressing map with load <= 0.70, checked capacity growth and a
4096-probe fail-closed collision guard; document this additional unavailable
reason. Erase/reuse tombstones with bounded rehash so storage depends on capacity,
not historical pointer identities. Charge rehash's simultaneous buffers or fail.
Do not preallocate every individual maximum at once.

At most four snapshot populations, canonical key storage, original graph union,
family membership and sort indices are also charged to the **same 256 MiB table
budget**. Bound serialized/rendered stack bytes by one MiB and original trace
depth by 512. Cache only bounded selected-trace results; no all-event expanded
backtraces or all-mark owner trees. Allocation failure is failure, never a
truncated ranking. The count ceilings do not promise all maxima jointly fit.

Unchanged ceilings: raw 3 GiB, interpreted 1 GiB, oracle 64 MiB; 2,000,000 traces,
500,000 IPs, 1,000,000 descriptors, 2,000,000 live raw pointers, 200,000 strings,
64 MiB string bytes; table 256 MiB; output including receipts 32 MiB; scratch
64 MiB; CPU 180 s and wall 300 s **per pass**, total wall 900 s, RSS 512 MiB,
child AS 768 MiB. Phase 3 includes normalization/classification/output. Admission
MemAvailable >=768 MiB and disk >=1 GiB; ongoing disk >=512 MiB. No increase,
spill-to-large-derivative, extra thread or automatic retry on any limit.

### Guards without per-record syscalls

Retain the external Linux <=0.25-second selector wait and owned-child /proc
CPU/RSS/AS/thread, disk and scratch checks. A native cheap work counter checks
monotonic time at least every 1024 records **or 64 KiB consumed**, whichever
comes first; every 0.1 s invoke full child CPU/RSS/wall checks. Force full checks
at phase entry/exit, before success, and at output/reconciliation boundaries.
Inside a long symbol/line/stack or map-probe loop, poll after bounded bytes/nodes/
probes too. A 1024-record count alone is not a wall-time guarantee. External
monitor remains authoritative for blocking I/O and stalled inner operations;
Linux RSS/AS/CPU limits are not replaced by cadence sampling.

Keep per-record semantic/accounting and byte-budget checks synchronous. Output
size/scratch charging remains per write before allocation/publication; batch
expensive statvfs checks at bounded output blocks/full guard intervals and retain
external free-disk enforcement. Phase-boundary checks must reject an already
exceeded phase before resetting clocks or sending the next phase. Actual guard
latency and kill/reap must be proved on Linux; portable timing cannot prove it.

## Exact parity and migration gates

Keep all existing 21 tests as the reference suite, with their assertions, rather
than renaming a native smoke to mean parity. Native port needs a fixture-only test
adapter exposing record parsing, aggregate/snapshot results, canonical rendering
and classification (no such test seam usable on production manifests). Feed the
same fixture bytes to both engines and compare structured values or exact TSV
bytes, excluding only declared engine identity, resources, paths and timings.
The existing suite imports Python functions, so **running it unchanged alone does
not test the native implementation**. The implementation must supply this adapter
and a per-test mapping before exact-code review.

| Existing test name (prefix `test_`) | Native parity obligation |
|---|---|
| strict_records_preserve_string_bytes | Full bytes, UTF-8, optional IP arity and lexical rejection |
| three_pass_baseline_and_first_peak | Exact first peak line/offset, survivor and requested bracket |
| saved_smoke_full_canonical_multiset | All existing exact counts, 637/638, 4196352 family, full stack oracle |
| lossless_output_and_fail_closed_output_budget | Original graph, cutoff TSVs, escaping and no ranking on exhaustion |
| owned_portable_runner_smoke | Direct native child protocol/output under fixture-only mode |
| synthetic_realloc_multiplicity_zero_and_survivor | Descriptor zero, reuse/realloc deltas, zero bytes, repeated marks |
| time_rejections_and_unbounded_event_tail | Every invalid request, missing/nonmonotonic mark, real tail |
| record_adversaries | Every lexical/string/IP/module/line adversary |
| reference_and_session_adversaries | Every future reference, duplicate/session/attach/raw-pointer adversary |
| conversion_hash_suffix_and_canonical_not_sum | Hash/definition mutation between passes and wrong equal-total stacks |
| each_table_numeric_input_and_depth_cap | Every lower-only cap and checked overflow; no signed/unsigned drift |
| suppression_does_not_change_population | Suppression is metadata, not a free or canonical leak oracle |
| normalization_collisions_new_stop_inline_unresolved | Exact new/stop/template/inline/IP-zero/unknown normalization |
| resource_guards_and_parent_unchanged | Every sample threshold plus parent environment/limits unchanged |
| real_owned_wall_kill_reap_and_bounded_failure | Real native child stall, timeout and cleanup |
| resource_failure_reports_exact_guard_and_usage | Specific bounded reason/resources, only failure receipt |
| ownership_unknown_callers_are_not_lost | Unknown whole-stack callers and non-frozen source remain unknown |
| real_owned_cpu_guard | Actual native parse/guard CPU exhaustion, not a Python substitute |
| manifest_and_receipt_rejections | Duplicate keys, all bindings/modes/counters/options, warning stderr |
| linux_native_memory_guard_small_child | Real owned RSS pressure and native monitor/reaping |
| linux_native_as_is_bytes_in_owned_child | Real native child AS limit in bytes and allocation failure |

Add migration-specific tests: chunk boundaries inside every field/newline/UTF-8
sequence and exactly at line cap; raw and normalized hash batching identity;
uint64 max versus signed aggregate overflow; table capacity/rehash transient and
collision limit; repeated allocate/free churn with bounded resident map capacity;
zero IP/trace and deep-stack limits; stable tied output ordering; full original
stack reconstruction; every phase-end guard rejection; long-line/render/sort
stall coverage; native abnormal exit, incomplete/duplicate/malformed protocol,
wrong executable hash and no fallback. Assert file-open counts: raw once,
interpreted twice; oracle once. Run seeded randomized small raw/interpreted pairs
through both engines with exact counts/digests/checkpoints/output comparison.
A changed failure message must map to the same static failure category, not turn
rejection into acceptance. Do not preserve an uncovered defect as compatibility:
report it for review with a minimal fixture before changing contract semantics.

## Bounded native qualification, not a production retry

This card ends with design review. Root separately releases implementation,
actual exact-code Grok 4.5 review, then a native qualification card. Design review
alone grants neither implementation nor remote execution from this card.

For that future qualification, freeze executable/source/lock/compiler hashes and
keep the old failed replay and capture directories read-only and untouched. Use
a **fixture-only qualification driver**: no arbitrary input path, no production
scope branch, no production manifest mounted/copied, every trace/oracle <=
2,000,000 bytes, fixed allowlisted saved fixture hashes or generated fixtures.
Use a new absent output directory and a finite explicit case list; no retry loop,
production command appended after tests, or success hook that calls the runner
with production inputs. The reviewed implementation can have a production CLI,
but the qualification driver must not expose it.

Run once per declared case, stop on unexpected failure:

1. All mapped 21 tests plus the added migration tests, no Linux skips. Expected
   negative guard cells are test successes only if they fail for the intended
   reason and the retained child is reaped with no published ranking.
2. Existing saved smoke through the real native child/supervisor. Rehash complete
   outputs and compare full canonical multiset and graph/cutoff values with the
   unchanged Python reference. No new interpreter or capture needed.
3. Predeclare small generated churn fixtures at 250,000 / 500,000 / <=1,850,048
   bytes, short/wide numeric variants; additional <=2 MB cases with pointer
   diversity, unique descriptors/trace nodes, long symbols/inline chains and
   near-cap lines. Include each phase and output; no concatenated sessions or
   repeated headers to fake a large trace. Three serial timed repetitions per
   case, each in a fresh owned child with lower-only CPU 30 / wall 60 seconds;
   one finite qualification allocation of <=900 wall seconds. Record all results,
   maxima, medians, bytes/records, capacities, clocks and sampling observations.
4. Lower-only real CPU/wall/RSS/AS/table/output/scratch failure tests; use controlled
   disk/admission samples where actual disk depletion would be unsafe, label them
   simulated. Check external sampling intervals and long-inner-loop stalls,
   phase-3 budget, child death/reaping, no leaks in parent policy or protocol.

Predeclared rejection criterion: correctness, guard, identity or cleanup failure
blocks release; throughput failing even the necessary inventory-derived average
rates on the simple fixture blocks a retry recommendation. Passing those rates
on small fixtures is **not production capacity evidence**. Record a separate
readiness judgment with explicit table/population/I/O uncertainty. Do not read
even a production prefix merely to refine qualification under this scope.
Root must make a new explicit one-attempt decision, with fresh native admission,
exact manifest and new output path, before any production replay. Neither tests
nor a compiled implementation authorize that decision automatically.

## Verification and handoff

Local real execution: probe exit 0, all within-mode result signatures equal;
audit exit 0, all export/smoke hashes and four report objects agree. Unchanged
Python regression suite ran 21 tests in 5.858 s: 19 pass, two explicit Linux-only
skips. This is separate from original native 21/21, no skips. No affected Rust
crate exists in this documentation/diagnostic-only diff, so no host/client build
or tests were run. Two attempts at inline analysis tools were blocked by the
headless execution policy; the checked-in bounded audit script performed the
analysis instead. No permissions, agent configuration or limits were changed.

Commit only this design, the two bounded diagnostic scripts and two JSON result
files. Same-card `reviewer` handoff follows. No new cards/delegation; no formal
parser replacement here. Root's final campaign acceptance and whole-branch
Grok 4.6 review remain separate and open.
