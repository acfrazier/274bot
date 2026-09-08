# Current latency companion plan

Status: bounded source-only plan; no new measurement, result, or acceptance claim.
Branch: `codex/memory-diagnostics`.
Owned artifact: this file only.

## 1. Finish-line gates and endpoint provenance

The approved budgets in `performance-finish-plan.md` §1 and §5 remain unchanged:

- decode update → script dispatch, p99 `<=100 ms`, with complete closed accounting;
- focused physical UI input → visible UI acknowledgement, p99 `<=100 ms`, with complete closed accounting.

These are separate populations. The panel endpoint is a newer mailbox generation presented to the focused Game Image / wall-tile texture. It is a host texture bind/upload and `GameView::present` boundary, not display scanout, compositor presentation, swapchain completion, or a GPU hardware completion timestamp. The TUI endpoint is a successful `terminal.draw` flush, not terminal-emulator paint or scanout. Never merge TUI flush and panel texture presentation into one latency result or call them equivalent visibility.

This plan changes neither target nor reader acceptance rules. It cannot turn incomplete coverage, an overflow bucket, an unfocused slot, or a provenance mismatch into a pass.

## 2. Current evidence and the actual gap

`latency-boundary-confirmation-audit.md` recomputes `reference_metrics.py` from the archived `metadata.json` and `samples.jsonl` and preserves the current classifications:

| Role | Gate | Available | Boundary-pending | Overflow |
|---|---:|---:|---:|---:|
| Candidate | input | 13/16 | 2 | 1 |
| Candidate | decode | 14/16 | 2 | 0 |
| Reference | input | 13/16 | 3 | 0 |
| Reference | decode | 12/16 | 4 | 0 |

The reader chooses one deterministic maximal publisher-contained span using the fixed observe mono bounds and requires zero pending at both selected edges. It does not search for a quieter interior window. It also preserves `ended`, cancel/lost/drop, counter identity, clock containment, and overflow failures.

The unavailable boundaries are real aggregate gauges, not evidence that the reader may slide the window:

- candidate decode has `pending=1` at the selected start and clears on the next sample;
- reference decode has `pending=1` at the selected end;
- input has long-lived open starts at observe end in both roles;
- the prior TUI data retains a producer focus/flush correlation risk around 30-second focus rotation. The corrected origin helper was separately smoke-tested, but that smoke is not full-fleet latency evidence.

The missing capability is fixed-window cohort closure. A post-end global drain that merely appends samples cannot repair the original fixed-window histogram: it does not identify which completions belong to starts inside the window, and a requirement that the original edge gauge be zero still rejects the edge-open rows. Waiting for global zero under continued traffic is not equivalent to complete accounting for `[start,end)`.

## 3. Current source capabilities and immutable rules

Existing source supports the following and should be reused:

- `crates/host/src/responsiveness_profile.rs`: bounded per-generation decode queues, process-wide bounded input pending storage, monotonic start/complete/cancel/lost/drop counters, per-slot generations, pending gauges, coarse/fine latency histograms, and process-local mono capture brackets;
- decode start at the host `PLAYER_INFO` generation edge after drain, and completion at host-play entry to the script observe/on-game-tick path;
- panel input start at a focused actionable mouse-down/key-down event, and completion only through `note_panel_present` for the focused mailbox generation;
- TUI input start at a focused key edge and completion only after the retained origin reaches a successful `terminal.draw` flush;
- `crates/host-play/src/memory.rs`: one-second samples, phase labels, per-slot responsiveness rows, `elapsed_mono_ns`, and existing `observe-start` / `observe-end` qualification records; `focus_index()` pins slot zero for `focused-one`;
- `docs/memory/reference_metrics.py`: slot/generation pairing, mono bracket validation, deterministic containment, counter identities, p99 bucket bounds, `overflow_bucket`, and `boundary_pending_incomplete`.

Do not relax or reinterpret any of these rules. In particular, preserve pending as a gauge, preserve every cancel/lost/drop/overflow, reject resets and malformed brackets, keep raw rows, and never trim a favorable quiet sub-window. Use only existing duration controls `BOT_MEMORY_WARMUP_S` and `BOT_MEMORY_OBSERVE_S`, and existing `--responsiveness-profile` / `--responsiveness-fine`; do not invent flags.

## 4. Frozen current Windows panel lineage and first cell

The first companion cell is the current Windows panel matched pair, `focused-one`, N=16 active workload. These immutable hashes identify the parent lineages for a new identically instrumented matched build, not binaries capable of emitting the proposed cohort schema. Use these native parent lineages, not the older Mac/TUI manifest `diagnostics/matched-instrumented-build-20260907T101400Z/build-manifest.json`:

| Side | Host | Client | Panel binary | Staged panel |
|---|---|---|---|---|
| Baseline/reference | `9268890217d968cfeb7c66ebb11dd5c3dd2c084f` | `abb811bd0afa1acd99319ccd5bc36bfb241080f9` | `e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5` | `C:\ProgramData\274bot-Test\renderer-owner-census-9268890` |
| Candidate | `fb3589ac28583242b999ac864ea69c4ef8fa5923` | `fd956c91bf09e059359c8e182a33583e2c626cd3` | `a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f` | `C:\ProgramData\274bot-Test\tile-boxed-fb3589a` |

The current candidate host source aggregate is recorded as `189dac149beb6f258f5a79bdf09de43818556a5f06ffa783a608f0113a64d8a4`; its client source aggregate is `fad2e79248c8d62cff2ceba5c757ef555705e85d3dc6797586c9841d9dbfb53e`. The baseline/native freeze receipt remains authoritative for the full baseline provenance. Bind both sides to the same cache/catalog/nav/settings/server/geometry and renderer policy, and record literal source/binary hashes, OS/CPU/RAM/GPU/backend, N, workload, focus policy, ready/active/rendering counts, and actual endpoint settings. Do not compare these Windows binaries with the older Mac/TUI binaries or with a different client/cache/server state.

For this `focused-one` panel cell, the declared input population is exactly `{slot 0}`: `focus_index()` pins slot zero and only that tile is in scope for focused physical UI stimulation. The other 15 active slots remain in the decode population and resource workload, but their `no_input_samples` are outside the panel input population—not fleet failures and not latency passes. A reusable panel latency stimulation helper has not been established. The reviewed `windows-visual-proof-tools/invoke-rebuild-capture.ps1` performs one guarded click for visual diagnostics; the Teles click is panel chrome and does not exercise `stream_capture_for` into the Game Image. Before a panel input companion, a separate bounded helper must deliver ordinary actionable input through the actual focused Game Image capture path, verify capture enabled and slot-zero focus, and record event cadence plus PID/start/window identity. Source route: `crates/panel/src/session.rs` `stream_capture_for`/`note_panel_input_start`; capture disabled is a no-op. Choose stimulus that preserves the active fixture and verify resulting start/bind/present accounting. Do not call the visual teleport controller an existing latency probe or claim input coverage before this prerequisite is reviewed and functionally proven. Decode remains a separate N=16 population and is reported independently for reference and candidate.

## 5. Predeclared complete accounting protocol

This protocol must be agreed before reading either result. It is a measurement design, not an acceptance waiver.

1. After the common cohort instrumentation and panel stimulation prerequisite pass review, rebuild and freeze both native parent roles with identical diagnostic source changes and new source/binary hashes. Run exactly one new reference/candidate pair on the Windows `focused-one`, N=16 active cell. Use the existing 120-second warmup and 600-second observe defaults unless the frozen receipt explicitly records another fixed duration. Keep responsiveness and fine profiles identical on both sides; retain all one-second observe rows, including first and last.
2. Define fixed monotonic boundaries from the existing qualification records: `T_start` is the observe-start boundary and `T_end` is the observe-end boundary. The performance cohort is defined by event start time, not by the sample in which a counter becomes visible:
   - include an event iff `T_start <= event.start_mono_ns < T_end`;
   - exclude every event started before `T_start`, even if it completes during observe;
   - exclude every event started at or after `T_end`, even if it completes during the tail;
   - include the completion/terminal outcome of every included event if it occurs after `T_end` during the finite tail.
3. Add bounded source-side cohort identity, rather than trying to infer identity from aggregate deltas. Each decode/input start receives a per-slot-generation event sequence plus `start_mono_ns`; its pending record retains that identity and endpoint surface. Completion retains the same identity and `complete_mono_ns`; cancel, lost, dropped, and overflow retain the identity or an explicit bounded-loss record. The emitted observe row/qualification metadata must expose the fixed boundary values, cohort counts, and a schema/version marker sufficient for the reader to distinguish observe from tail evidence.
4. At `T_end`, stop admitting new events to the performance cohort but do not stop the runtime or discard pending records. Continue the existing one-second sampling lifecycle in an explicit `drain`/`settling` phase for a predeclared finite timeout. The tail may close included events only; it must never add post-end starts. If an included event does not complete or receive a valid terminal outcome before timeout, preserve `boundary_pending_incomplete` (or the existing explicit timeout reason). Do not wait indefinitely for global zero and do not require the original aggregate edge gauge to be zero as a substitute for cohort closure.
5. Bound the ledger using existing pending capacities and explicit accounting for saturation. A full ledger must increment the existing dropped/overflow outcome and make the affected cohort unavailable; it must not evict an older included identity silently. Drain cancellation, loss, drop, reset, generation mismatch, malformed timestamps, and any missing identity also make the affected gate unavailable. Retain raw observe and drain rows and the final ledger summary so a reader can reproduce every exclusion and terminal outcome.
6. Reader math changes only to consume the explicit cohort records. For each slot/generation and gate, compute the latency histogram and p99 from completed members of the predeclared cohort, plus terminal outcome counts for that cohort. Validate monotonic identities, `T_start <= start < T_end`, completion-to-start ordering, bracket containment, and the unchanged counter conservation rules. Keep the existing p99 lower/upper bucket semantics; an overflow bucket has a finite lower bound and no invented upper bound. Keep `boundary_pending_incomplete` for unclosed cohort members, not for a mere aggregate gauge that includes pre-start or post-end traffic.
7. Keep the existing aggregate sample fields immutable for compatibility. Add only additive phase/cohort fields if needed; do not reinterpret `pending`, change bucket boundaries, remove `no_input_samples`, trim rows, or select an inner window. The reader must still fail closed if the new cohort schema is absent or malformed.

The core review question before implementation is whether the proposed identity ledger and tail prove membership in `[T_start,T_end)` without admitting post-end starts, while retaining bounded loss semantics. A drain-only change without this event-cohort identity is insufficient.

## 6. Bounded source work and tests

The next implementation is limited to the existing host-play sampling/window lifecycle and its reader fixtures:

- In `crates/host/src/responsiveness_profile.rs`, expose bounded event identity/start/completion or terminal-outcome records for decode and panel/TUI input using the existing per-generation queues and input pending bound. Preserve surface labels and origin correlation; do not change the endpoint definitions.
- In `crates/host-play/src/memory.rs`, record immutable `observe-start` / `observe-end` mono boundaries, stop cohort admission at observe end, publish explicit `drain` phase rows, enforce one finite drain timeout, and perform normal teardown only after the tail or timeout. Do not extend the fixed observe interval, omit its final row, or stop scripts before observe-end accounting is recorded.
- In `docs/memory/reference_metrics.py`, add additive parsing/validation for the cohort schema only if the source records require it. Leave existing legacy-reader rules intact and fail closed when complete cohort evidence is absent.
- Add focused fixtures for: decode start inside the cohort completing in drain; input start inside the cohort completing in drain; pre-start completion excluded; post-end start excluded; drain timeout unavailable; cancel/lost/drop/overflow during drain unavailable; generation mismatch unavailable; missing/malformed cohort metadata unavailable; and quiet-inner-window selection still rejected. Preserve the existing pre-activity input rule for leading all-zero rows.
- Keep the reviewed TUI origin routing unchanged unless a separate source proof finds a remaining correlation defect. TUI follow-up, if later chosen, is a distinct Linux/PTy population with `terminal.draw` flush provenance; it needs no new permission flow and cannot close the panel physical-UI gate.

## 7. Executable proof and stopping rule

After the bounded source change, run the affected host/host-play tests and Python reader/fixture tests. Apply the same reviewed instrumentation to both named native parent roles, build and freeze new binaries with new hashes, verify source/provenance symmetry and affected native tests, and obtain the required combined integration review before native comparison. Preserve all old source/binary receipts unchanged; the old executables cannot emit the new cohort schema. After the separately reviewed Game Image stimulation helper is functionally proven, execute the newly frozen Windows reference/candidate `focused-one` pair once, archive all raw JSONL, receipts, phase/cohort records, and provenance, and independently recompute each side. Verify that every excluded or unavailable member is reproducible from raw evidence and that no post-end start entered the cohort. Report reference and candidate independently, then the paired difference/non-regression result; no complete population means no acceptance claim.

If panel input is incomplete for the declared `{slot 0}` population, report that exact reason. Do not relabel the other 15 slots as failures or passes. A separately named TUI diagnostic may be run only as an endpoint-specific follow-up in the existing campaign scope; report its flush result separately and do not use it as panel evidence. Do not run another parked tile/appearance/resource stage, change unrelated renderer flags, or rerun without a named provenance or source confounder.

The only claim this plan authorizes is: “fixed-window responsiveness accounting is complete or unavailable for the declared endpoint and population, with raw evidence and reproduced reader reasons.” It does not authorize display-scanout, GPU-completion, target-hardware, full-fleet input from TUI flushes, p99 acceptance, or overall campaign acceptance. The approved p99 targets remain `<=100 ms` for both gates.
