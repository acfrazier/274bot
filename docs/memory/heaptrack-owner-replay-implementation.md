# Bounded saved Heaptrack owner replay — implementation

Task `t_2aba75a5`, branch `codex/memory-diagnostics`. Implements the approved
`existing-heaptrack-owner-replay-capability.md` design at `0bc2151`.
Status: implemented and locally tested, submitted for same-card Grok 4.5 review.
This is not a Linux executor release, production replay, or memory acceptance.

The original sole N1 capture remains **FAILED: raw-size guard**. There is still
no observe-end, Stop, post-join, or publication-epoch owner measurement. No
production prefix values, candidate, savings, or campaign budget acceptance are
claimed. No host/client/runtime source, STATE, capture-outcome report, remotes,
Windows, Hyper-V, game, capture, interpretation, or remote execution was changed
or started by this task.

## Owned files and entry points

- `heaptrack_owner_replay.py`: strict format parser, packed tables, raw audit,
  two interpreted passes, canonical comparison, and bounded lossless outputs.
- `heaptrack_owner_runner.py`: manifest/provenance validation, directly owned
  child, phase/resource guards, output validation, publication/failure cleanup.
- `test_heaptrack_owner_replay.py`: small synthetic/adversarial fixtures, saved
  controlled fixture, runner tests, and explicitly Linux-only qualification tests.
- `heaptrack_owner_smoke.py`: convenience entry point for the existing saved
  2001 fixture only. Uses the evidence export's existing hash manifest.
- `heaptrack-owner-replay-implementation-smoke.json`: compact observed receipt.
- This implementation report.

The old `existing-heaptrack-owner-replay-probe.py` and its receipt remain untouched
and remain a tiny research probe, not the production implementation.

Production entry is the runner, not a direct call to `analyze`. Imports have no
process side effects. Everything is Python stdlib; tested with Python 3.9.6 on
Darwin. No dependency installation or Rust build is needed for these offline
instrumentation files. Host/client crate tests are not substituted for parser
proofs and were not run because those crates were not changed.

## Accounting and validation

Exactly three input passes: raw once, interpreted twice. Hashing occurs during
those passes, rather than adding an input hash prepass. Each interpreted pass
must have identical full hash, definitions, metadata, event digest, counts,
peak/cutoff positions and final population. The small canonical peak file is a
separate oracle input, streamed once; no raw/interpreted derivative or event log
is written. No capture, re-interpretation or symbol lookup is invoked.

The raw audit tracks only currently mapped pointers and charges retained map
capacity at its high-water mark. It rejects duplicate-live/null allocations and
bad trace references. Matched allocation/free events are SHA-256 normalized as
opcode plus big-endian uint64 size/trace; timestamp events are included. A second
digest compares raw trace address/parent definitions with interpreted IP-address/
parent definitions. Metadata digests compare X/I/R/S without persisting raw X
command lines. Unknown raw frees are counted separately, not subtracted and not
silently treated as a complete inventory. No historical pointer set is retained;
therefore the production implementation does not claim a lifetime-wide distinct
pointer/reuse count. The synthetic fixture proves that free/reuse is legal.

Descriptors are zero-based reusable size/trace definitions, with checked positive
multiplicities, not allocation instance IDs. Duplicate descriptor definitions,
underflow, overflow, future references, invalid arity/hex/UTF-8/sized strings,
unsupported versions, attach/session duplication, unknown opcodes and incomplete
final records fail. Trace/IP/string zero is absent; descriptor zero is valid.
Parents must precede children. The raw audit and both interpreted passes reconcile
EOF bytes/counts. Trace rows include cumulative allocation calls for the full
trace; descriptor rows include size, live count, calls and requested bytes.

The first strictly larger global total determines the validation peak. Only its
position is retained in pass one, not a new snapshot for each record high. The
second interpreted pass reconstructs its population from the beginning. The
full canonical positive stack-to-cost multiset is compared, not just its sum.
Normalization reproduces Heaptrack 1.5 operator-new trace-node rewrites, stop
frames, template shortening, unresolved addresses, file basenames and inline
frames. Rendering collisions sum only for canonical reconciliation; original
trace IDs remain separate in all output accounting.

The default diagnostic cutoff is mechanically the last actual c record, with
EOF separately labelled. At most one requested millisecond selects the last mark
at/before it, retaining ordinal, line and zero-based byte offset, plus the next
actual mark. Repeated marks retain separate positions. Before-first/after-last,
negative and invalid requests fail rather than clamp. EOF reports the last
observed mark, last actual event and whether an event tail has no later time
bound. There is no synthesized last+1 timestamp or window-only baseline reset.

## Output and owner mapping

Only runner-owned `.pending` output is written before success. The child finishes
all arithmetic/canonical checks first. The parent requires exit zero and the
complete phase protocol, rehashes every output against the child receipt, checks
the total output budget including `runner.json`, then renames `.pending` to
`result`. A validation/resource failure returns nonzero, removes pending rankings,
and leaves a small `failure.json` with the static reason and available resource/
owned-child-reaping evidence. Existing output directories are refused, not
cleaned or overwritten.

`strings.tsv`, `ips.tsv`, and `traces.tsv` form a lossless stable-ID stack and
provenance graph for the union of selected populations. Original parent links,
IP address/module/function/file/line/inline fields and unshortened UTF-8 strings
are preserved. JSON-escaped TSV cells preserve whitespace and bytes. This avoids
repeating expanded stacks at every cutoff. Each cutoff has complete sorted stack,
family and descriptor TSVs (requested bytes descending, stable ID as tie-breaker).
Zero-byte live allocations retain their counts too. There is no successful top-N
or truncation fallback when the output cap is exhausted.

Families select the first useful application Rust frame or an explicit allocator/
native boundary. The conservative mapping is anchored to host
`c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`, client
`3456edc8dabf7b25ada78110ffa56327af9f67a4`, and the current frozen owner ledger.
Exact NavWorld load-pack source matches are process-shared-or-transient, not
private. Broad Client construction remains mixed shared/private. Exact World
construction can identify a source-per-client domain, not an instance owner.
Snapshot/script source-file matches name a ledger domain but leave ownership
unknown; a symbol alone does not prove shell coexistence or object ownership.
Unmapped source/native allocations remain explicitly unknown. The first useful
frame is not skipped merely to find a more interesting deeper owner.

The saved Python fixture is not attributed to the frozen Rust ledger. Its source
ownership remains unknown. A partially unresolved full stack contributes to
`unknown_symbol_bytes` even when its first useful frame is resolved. Family sums
must partition every cutoff's requested bytes and live counts exactly.

Suppression records are preserved and never applied to live requested bytes.
The default canonical leaked presentation is deliberately not an exact leak
oracle. The synthetic suppression fixture demonstrates that suppressed presentation
is not a free; it is not claimed as a new native printer run. The existing smoke's
exact EOF value is established by independent raw/interpreted accounting. No
rounded printer leak string is promoted to exact equality. Interpreter temporary
637 and analyzer temporary 638 remain separate conventions.

## Resource enforcement

The approved constants are rejection ceilings, not evidence production fits:

| Domain | Limit / enforcement |
|---|---|
| Raw / interpreted | 3 GiB / 1 GiB, manifest and regular-file size checks before streaming |
| Canonical oracle | Additional bounded 64 MiB input; includes existing saved peak file sizes |
| Lines / numeric / depth | 1 MiB; lowercase uint64 fields; checked signed aggregates; 512 trace nodes |
| Tables | Combined 256 MiB conservatively charged packed arrays, string arena, pointer-map capacity, descriptor keys and selected output state |
| Individual tables | 2,000,000 traces; 500,000 IPs; 1,000,000 descriptors; 2,000,000 live raw pointers; 200,000 strings; 64 MiB string bytes |
| Per pass | 180 CPU seconds, 300 wall seconds; third pass includes rendering/output |
| Total | At most 900 wall seconds; no automatic retry or larger limits |
| Child memory | Linux RLIMIT_AS 768 MiB **in bytes**, RSS ceiling 512 MiB, `/proc` monitoring |
| Child files | RLIMIT_FSIZE 64 MiB; internal total scratch 64 MiB and output 32 MiB |
| Admission | Linux MemAvailable >=768 MiB, free disk >=1 GiB |
| Ongoing disk | >=512 MiB free; checked by writer and external monitor |
| Monitor | At most 0.25 s selector wait; checks owned-child CPU/RSS/AS/thread count, wall, disk and scratch; child also checks at bounded records and output rows |

Packed append-only arrays are charged with growth allowance; Python pointer/set/
selected-row structures carry conservative per-entry charges and are additionally
contained by RSS/AS. The combined table ceiling may reject below any individual
table count. Output exhaustion never weakens the table or coverage contract.

The runner creates only one directly retained child, uses no shell, starts no
helper pipeline or worker threads, changes environment/rlimits only in that
child, and kills/reaps only that child on failure. Child per-phase CPU checks are
supplemented by a total RLIMIT_CPU fail-safe and Linux external CPU sampling.
SIGTERM/KeyboardInterrupt flow through cleanup. Parent output verification is
bounded by the 32 MiB output ceiling. The controller's limits/environment are
unchanged by the run.

Portable mode is restricted to controlled saved fixtures with each input <=
2,000,000 bytes. It exercises real child CPU/RSS high-water/wall checks and
file/output handling on Darwin, but does not claim Linux `/proc`/AS enforcement.
Native-only tests are visibly skipped, not silently treated as passed. Root must
ensure no game/build/profile overlap and exclusive campaign analysis scheduling
on its selected executor; this tool does not infer the meaning of unrelated
system processes or authorize stopping them.

## Executed verification

Commands, run from the campaign checkout:

    python3 -m unittest discover -s docs/memory -p test_heaptrack_owner_replay.py -v
    python3 -m py_compile docs/memory/heaptrack_owner_replay.py docs/memory/heaptrack_owner_runner.py docs/memory/heaptrack_owner_smoke.py docs/memory/test_heaptrack_owner_replay.py
    python3 docs/memory/heaptrack_owner_smoke.py diagnostics/heaptrack-owner-replay-implementation-smoke

Results: 21 tests, two explicit Linux-only skips, suite exit 0; py_compile exit 0;
saved-smoke runner exit 0 and owned child reaped. Initial tests failed before the
parser, three-pass path, output writer and runner existed. Targeted follow-up
regressions failed on lost unresolved caller provenance and generic resource
failure receipts, then passed after the fixes. No installed ruff/pyright CLI was
found; syntax checks and the tool's edit diagnostics are clean. Static scan found
no shell=True, os.system, eval/exec or pickle in the tooling.

Coverage includes descriptor zero and multiplicity; mixed sizes; zero-size counts;
free/reuse; same-address and moved realloc deltas; no-event failed-realloc
semantics; pre-cutoff survivors; same-byte/different-instance populations; first
peak plateau; repeated marks/brackets/tails and time rejection; parser/session/
reference/hash/raw-conversion/canonical adversaries; every table/input/line/depth
ceiling; numeric overflow; suppression accounting and normalization collisions.
Resource tests use the same lowered guard code with tiny fixtures, controlled
sample threshold crossings for admission/disk/AS/thread checks, real owned CPU
and wall exhaustion, and actual RSS/table/output/scratch failure receipts.
They verify pending rankings disappear, the child is reaped, and parent limits/
environment remain unchanged. Linux RSS pressure and native AS tests are the two
skips; they are not represented as a Darwin proof of Linux enforcement.

Observed controlled fixture:

| Check | Result |
|---|---:|
| Raw / interpreted bytes | 389215 / 281592 |
| Allocation calls / matched frees / unknown raw frees | 6388 / 6354 / 2 |
| Descriptor maximum multiplicity | 2048 |
| First strict global peak | 5730024 bytes, interpreted line 27144, offset 250615, preceding mark 50 ms |
| Full canonical peak comparison | 2951 rows, 2212 positive, all normalized positive costs reconcile |
| Known PyByteArray_Resize at peak / EOF | 4196352 / 0 bytes |
| Requested 500 ms | actual 497 ms, next actual 507 ms; 5730024 bytes, 4962 live allocations |
| Last actual mark | 1064 ms, line 32299, offset 281552 |
| Last mark / EOF population | 407233 bytes, 34 live allocations |
| Interpreter / analyzer temporary counts | 637 / 638 |
| Table charge high-water | 7082334 bytes |

Durable local full tables/receipts:
`diagnostics/heaptrack-owner-replay-implementation-smoke/replay/`.
The compact checked-in smoke receipt binds its manifest, child receipt and exact
engine/runner SHA-256. Observed owned-child wall was 0.6969008339999999 seconds;
third-phase cumulative `ru_maxrss` was 21528576 bytes on Darwin. The smoke command
overlapped small offline tests: these are correctness/guard receipts, not a
production capacity benchmark or campaign resource comparison.

## Root-owned next qualification, not executed here

After actual same-card review, root selects a Linux executor, verifies exclusive
headroom/no game-build-profile overlap, stages the exact reviewed files and small
saved fixture, and runs the same suite (including both Linux-only tests) plus a
native owned-child guard smoke. Review approval alone does not release an
unqualified executor. Windows/Hyper-V remain untouched pending the operator's
restart confirmation. No new capture or re-interpretation is released.

The runner accepts one fixed externally SHA-256-bound JSON manifest:
`schema=heaptrack-owner-input/v1`; input roles `raw`, `interpreted`, `peak`,
`receipt`, `stderr`, each with absolute `path`, exact `bytes`, `sha256`;
`analysis_path` selects the saved receipt's analysis object; `provenance` carries
source/binary/capture-tooling/process/mode/failure identities; explicit
`suppression_policy`; and `requested_time` (null or one integer). Duplicate JSON
keys are rejected. See `smoke_manifest` for a concrete fixture manifest, and
`load_manifest` for the exact schema/production frozen identities. Do not insert
fixture provenance for production. Receipt/stderr are separately hash-bound;
interpreter/printer exits, printer rendering options, interpreted output size,
strict interpreter stderr and exact interpreter counters must reconcile. Tool
version is pinned by both streams' `v 10500 3` contract and saved tool receipt;
this does not claim reproduction of the installed distribution binary.

For a later root-approved existing-data run, the command shape is:

    python3 -B docs/memory/heaptrack_owner_runner.py --manifest /absolute/reviewed-input.json --manifest-sha256 EXACT_MANIFEST_SHA256 --output /absolute/new-output-directory

Do not use `--portable-fixture` for production. `--limits` accepts only stricter
positive integer limits for bounded fixtures, never an increase. Existing raw
2149010072 bytes and interpreted 810563095 bytes remain remote under the approved
Concord inventory; neither was opened here. The eventual production oracle must
reconcile the entire saved normalized positive peak population, not merely the
known 160842811-byte sum. Resource or reconciliation exhaustion yields unavailable
prefix diagnostics, not a successful partial ranking, automatic retry, larger
capture, or optimization permission. Final whole-branch review and campaign
budgets remain separate.
