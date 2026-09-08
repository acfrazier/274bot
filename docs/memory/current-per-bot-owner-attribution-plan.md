# Current per-bot ownership attribution plan

Task t_1bb2dfd4. Corrected by root after source review of the initial8764d4a
proposal; the corrective worker stopped on a provider broken pipe without edits.
This plan is design only. Its first execution stage uses source and archived
evidence. It does not release another live cell or an optimization.

## Question and measured boundary

Which concrete live owners and recurring work paths merit a bounded change for
the current TUI? The approved diagnostic pair has N1/N16 steadyRSS176.322/520.779
MiB and CPU0.048993/0.561746cores. The descriptive active difference is22.964MiB
and0.034184cores per added bot. N16 misses the512MiB target by8.779MiB and the
0.5-core target by0.061746core; incremental memory misses16MiB by6.964MiB.
These are observations from one sequential N16/N1 pair, not accepted performance,
repeatability, a linear growth law or a measured zero-bot intercept.

Authority: performance-finish-plan.md §§1,3,4A–C,5–6. Evidence: current-tui-n1-n16-
attribution-report.md, its evidence JSON, and independent reviewde0a165 (actual
Grok4.5 session20260908_135008_08321e). Raw resource/shaped gates remain
missing_resource_provenance and profiling/helper overhead remains unknown.
Managed resources are separately bound. None of this waives the final gates.

## Correct lifecycle and accounting

The profile-off fixture calls only `Play::script_stop` at observe end
(crates/host-play/src/memory.rs, Run::poll around1069–1078), then samples during
the existing60-second teardown. `finish_cohort_shutdown` returns immediately
when the cohort is absent (around2016); its stop_slot/join loop belongs to the
cohort-enabled path. Both final rows still have ready=N. They are **after script
Stop, with clients and snapshots still alive**, not samples after client joins.

The finalRSS171008000/433799168bytes therefore does not establish allocator
retention, nor does zeroV8/inflight mean every live owner was dropped. There is
no sampled post-join resident value in this pair. Process exit reclaims the
address space but cannot supply a surviving-process post-join RSS sample.

Keep domains separate:

- V8 used/total gauges are logical heap bytes; allocator payload/capacity walks
  are logical allocated storage; RSS/smaps describe resident mappings/pages.
  Do not add or subtract these as an explanation of resident memory.
- A steadyRSS minus finalRSS arithmetic difference is a temporal observation,
  not proof of which field freed pages. The medians and final rows have different
  populations/times. No causal V8 band or allocator band is established.
- client_tick_total_ns, script_tick_total_ns and UI timers measure elapsed work
  spans. They can overlap across threads and are not per-owner processCPU.
- Server/helper resource series stay separate. Server medianRSS is631.957MiB
  atN1 and624.594MiB atN16; these are not client costs or measured overhead.

## Current owners and enabled paths

Frozen runtime: hostc0709ab, client3456edc8, binarya0c6eb0b; System allocator,
memory-profile-no-alloc, snapshot-dedup OFF, scheduling/responsiveness/render
profiles OFF, TUI RasterMode::Off. Root checked351 Rust/Cargo files against the
freeze; only TUIbin.rs differs, in reviewed cfg(test) fixture code. Receipt:
diagnostics/current-per-bot-owner-preflight/current-source-delta.json. Verify
this relationship again before compiling any layout probe; do not mix source
lineage or substitute an unrelated cached rlib.

| Owner | Current source / consumers | Sharing and lifetime |
|---|---|---|
| Cache, decoded interfaces and initial mut template | Play cache; host::prepare_client; Client::from_shared | Process shared Arcs, with per-client COW writes; not N full cache copies |
| Navigation world / immutable animation and sound data | Play.world and client shared load paths | Shared fixed owners; earlier savings are not new credit |
| Client tables, World squares/sprites/stamps/collision/heights | Client and core/world.rs; simulation and later rendering attachment | Per client, retained while slot thread lives; actual occupancy/capacity is not in these raw counters |
| Host GameSnapshot | SlotLoop::snapshot; after_drain/rebuild_dirty; guardian/auto-run consumers | Per-slot shell after packet drain |
| Host-play nav_snapshot | observe_rebuild_snapshot and observe callback; script encoding, wire and navigation | Per-slot shell before current frame drain, rebuilt on tick edges |
| V8 isolate, fingerprint and encoded buffers | script/slot.rs and isolate_fb.rs | Per active script; Stop clears relevant script owners; no resident-byte attribution follows |
| NavBot, wires and status publication | Play slot maps | Per-slot state; distinguish script Stop from slot stop/join |
| TUI and harness infrastructure | crates/tui and host-play memory publisher | Process-level work; instrumentation overhead unknown |

TUI has no resident per-slot renderer in this pair. Do not apply historical
panel renderer census totals to it. Snapshot widgets/side-tabs/loc families are
private vectors in the measured feature-off build; the two publication epochs
must remain distinct. The source-present snapshot-dedup feature uses content
comparison and separate shells, but is OFF here. Do not reimplement it or call
it an accepted/default saving. Appearance boxing, shared navigation and earlier
representation changes already exist; inspect current code rather than revive
historical proposals. Historical allocation reports supply stack-family leads
only, not current occupancy or RSS accounting.

Recurring per-slot work includes client mainloop/packet handling, host snapshot
rebuilds, tick-edge nav snapshot rebuild/encoding, V8/script execution, guardian
and navigation. Existing elapsed timers may help locate leads; a CPU-dominance
claim requires actual CPU attribution or a controlled comparison.

## O1 Phase A: source and archive ledger

After same-card design review, execute one bounded offline ledger task. No game
server, live frontend, profiling run, service change or performance claim.

1. Freeze the source paths and build identities used for the ledger against
   the c0709ab/client3456 runtime. Preserve the original archives. Reuse existing
   raw counters and lifecycle source proofs rather than rerun calibration.
2. Enumerate retained fields for Client/World, the two GameSnapshot shells,
   fingerprint/buffers and shared owners. Record purpose, consumers, sharing
   boundary, observe/after-script-Stop lifetime, fixed capacity versus dynamic
   occupancy, and the precise missing runtime facts.
3. Obtain relevant layout sizes with a tiny compile-only probe or focused
   offline fixture build. Record target triple, compiler, feature graph and
   exact dependency/source provenance. A Mac layout is not automatically a
   Linux ABI measurement; nativeLinux compile-only output is preferable where
   required and belongs on the existing builder, not the measurement VPS.
4. Compute exact fixed reservation formulas where source guarantees capacity.
   For dynamic owners, distinguish a structural worst case from an observed
   fixture size. A representative snapshot is neither the real workload's
   occupancy nor a valid upper bound without an explicit enclosing invariant.
   Serialized length cannot stand in for vector capacity or nested allocation.
5. If existing frozen raw artifacts contain real payload/occupancy, account
   only covered owners. Otherwise mark those fields unmeasured. Do not fabricate
   actual dual-shell overlap by counting one fixture twice; equal content across
   distinct epochs must be observed/proven. Existing payload helper functions
   may assist accounting but do not enable dedup in the measured configuration.
6. Recompute current logical gauges and after-script-Stop observations in their
   own domains. Record available elapsed work counters with overlap limitations.
   No post-join column may contain data from these after-script-Stop rows.

Deliver docs/memory/current-per-bot-owner-ledger.md and a compact machine-readable
ledger with exact formulas, source/build anchors, coverage and unknowns. A rank
is allowed only within an appropriate measured/bounded domain. Structural sizes
can eliminate an impossible candidate or identify a promising lead; they do not
prove an RSS or CPU win. If dynamic occupancy prevents an ordering, that explicit
result completes Phase A and triggers a concrete capture proposal, not another
synthetic fixture exercise.

## Capture decision after Phase A

The Phase A report must select the missing discriminator: actual live allocation
families/capacities, snapshot equality/retention, or CPU work. It should reuse
existing candidates and observations before asking for a new experiment.
A later root-owned capture card must predeclare **one exact procedure**: platform,
N, binary/features, account/cache/server identity, warmup/observe/teardown,
capture point/window, helpers, headroom/output bounds, cleanup and one-attempt
failure rule. It must state what answer would select a specific candidate versus
leave the result unresolved. This plan does not authorize an optional N1/N16
pair, arbitrary shortened observations, or repeated attempts.

Current tool facts (read-only18:03–18:13 observations): nativeConcord has
perf6.8.12 and matching kernel tools139; perf_event_paranoid=4, ptrace_scope=1.
Heaptrack/Valgrind are available through Ubuntu packages but not yet installed
at that observation. User offered to install useful tools; root recommended
Heaptrack and provided sudo commands. Verify installation before use. About
958MiB memory was available with no swap; that is a snapshot, not guaranteed
capture headroom. Existing 15GiB free disk likewise needs a fresh check.

Heaptrack can provide allocation call stacks with matching symbols; custom
pool/mmap coverage remains limited, and its run is a perturbed allocation
observation, not an RSS/CPU acceptance cell. A tiny owned-process smoke must
prove collection, symbol interpretation and bounded cleanup before profiling a
client. Starting a new owned child is preferable to attaching to another process.
Do not silently loosen system perf/ptrace permissions. No privileged measurement
configuration is released here. Source: https://github.com/KDE/heaptrack.

If a phase-split capture is chosen, actual after-script-Stop and after-slot-join
observations need distinct explicit barriers on a still-running diagnostic host.
The original profile-off run lacks that post-join sample. Joining clients changes
the live population; retain the normal product lifecycle, isolate any diagnostic
barrier, and review instrumentation/fixture semantics before execution.

## Candidate and verification boundary

| Evidence outcome | Next bounded action |
|---|---|
| Covered current snapshots retain material duplicated payload across the actual two epochs | Assess or measure the existing dedup candidate with identical behavior/configuration controls; do not infer equality from identical synthetic fixtures |
| Covered client/world fields dominate logical allocations | Propose one representation change with current occupancy, consumers and lifetime proof; measured RSS/CPU still required |
| Page retention remains suspected but live-owner coverage is missing | Keep allocator attribution unresolved; do not park live-set reductions on this suspicion |
| CPU work remains unlocated | Design bounded CPU attribution with valid per-thread/process accounting or matched controls; elapsed span sums are not CPU |
| No candidate has supported material potential | Report the concrete missing evidence or tradeoff; do not invent a rewrite or claim campaign completion |

Any implementation needs a reviewed ownership/behavior contract, affected crate
checks and separate client integration tests where applicable. Preserve packet
and action ordering, publication epochs, all snapshot families, script APIs,
rendering/cadence and existing errors. Candidate keep/park and matched measurement
rules remain performance-finish-plan §§5–6, including regressions and final
absolute targets. No additional live cell, default feature change, benchmark
acceptance or whole-campaign approval follows from this design.
