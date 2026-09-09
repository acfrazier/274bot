# Native production-prefix replay result — diagnostic only

Date: 2026-09-09 UTC
Status: completed validated prefix diagnostic; not production acceptance, owner attribution, or a memory-saving result.
Source: `a55972aa5cce2902c2b09aa098ee825d84a5957a`
Evidence root: `diagnostics/root-native-production-a55972a/`
Structured companion: `heaptrack-owner-native-production-prefix-result.json`

## Decision and provenance

After the approved fixture qualification (`4ab8c7e`) and root smoke recovery (`edcf3bb`), root released exactly one native replay of the identical existing failed capture prefix. It used manifest `7fcd08aed20e44c81dd495f83ae663b167f4392b84c39a79ce927b45165e1595`, native executable SHA-256 `98b5a2a20b0daf66057024f4618972fb34219b34568a09fa3be89a674d265b7f`, unchanged CPU 180 s / wall 300 s per-phase guards, RSS 512 MiB, address space 768 MiB, table 256 MiB, output 32 MiB, scratch 64 MiB, and a 930 s outer limit. There was no new capture, reinterpretation, limit change, or automatic retry.

The fresh admission found 1,011,859,456 available memory bytes, 11,810,811,904 free disk bytes, no listed conflicts, and 41 staged files rehashed. The remote wrapper PID 3963 and runner PID 3964 completed; child 3965 was reaped. Root's export readback found all three PIDs absent. The runner returned exit 0 after 167.132470633 s. Exit 0 means the bounded replay completed; it does not turn the failed capture into a complete capture or acceptance.

The local archive is `production-replay-1-export.tar.gz`, SHA-256 `62dc105b3095f274176a3ee9b84e31b53606cac504f6149f59930c420ef70cbd`. I independently rehashed every entry listed in `production-replay-1-export.json` (20 entries, including the replay, control and result artifacts); all recorded byte sizes and hashes match.

## Accounting and cutoffs

The replay reports tracked requested allocations, not RSS, retained allocator pages, V8/pool/mmap/GPU inventory, or per-bot ownership:

| cutoff | requested bytes | live objects | interpretation |
|---|---:|---:|---|
| peak, 940 ms | 160,842,811 | 89,122 | first common-time global peak |
| last mark, 589,611 ms | 132,027,090 | 209,263 | last actual timestamp |
| EOF | 132,028,404 | 209,283 | captured-prefix end; event-tail time unbounded |

Raw totals are 57,857,548 allocations, 57,648,265 frees, and 2 unknown frees. The raw and interpreted identity fields agree for the recorded prefix. The interpreted stream has 14903 canonical rows, with 194 positive peak rows and 160,842,811 peak bytes.

I independently recomputed, for all three cutoffs, the requested-byte and live-object sums from stacks, families and descriptors. Each of the three table families agrees with the receipt totals: peak 160,842,811 / 89,122; last mark 132,027,090 / 209,263; EOF 132,028,404 / 209,283. For all descriptor rows, `size * live_count == requested_bytes` also holds. The receipt's canonical full positive multiset equality is recorded as runner attestation; this report does not claim that I independently reconstructed all 14903 canonical rows.

## Attribution boundary

Unknown ownership is 100% of requested bytes at every cutoff: 160,842,811 at peak, 132,027,090 at last mark, and 132,028,404 at EOF. Nearly all symbol coverage is unresolved. The family outputs contain mangled Rust v0 symbols with empty source-file and line fields, and are conservatively classified as `allocator_or_native_boundary` / unknown. A generic allocator template (`alloc`, `RawVec`, `Vec`, or similar) and a crate substring (`nav`, `client`, `api`, etc.) are not source-owner proof. No demangling guess, source-owner ranking, lifetime ranking, per-instance attribution, or publication-epoch attribution is accepted here. The bounded source-symbol discriminator remains a separate root decision.

The result therefore cannot answer which bot, object, field, epoch, cache, renderer, script runtime, or host path owns these bytes. Descriptor IDs are not pointers, instances, ages, snapshots, or publication epochs.

## Resources and throughput

Native phase measurements are:

| phase | CPU seconds | wall seconds | guard result |
|---|---:|---:|---|
| 1 | 72.296205772 | 72.309123209 | below unchanged 180 / 300 |
| 2 | 45.877838647 | 45.885733043 | below unchanged 180 / 300 |
| 3 | 48.919478674 | 48.933855589 | below unchanged 180 / 300 |

Sparse `/proc` peak RSS was 23,871,488 bytes. The child's cumulative peak RSS was 44,568,576 bytes. These are different measurements with different scopes and must not be compared as a saving or treated as comparable peaks. The highest recorded charged-capacity table value was 31,736,666 bytes; it is not RSS. The receipt's `resources.completed_passes` predates the final phase and lists only phases 1 and 2, while `runner.phases` records all three. This is documented evidence placement, not an invented correction.

## Capture and acceptance limits

The source capture remains failed at the raw-size guard. Replay cannot repair missing observe-end, Stop, or post-join evidence. `capture_complete=false` and `acceptance=false` remain explicit. The EOF cutoff is an input-prefix boundary, not process shutdown. Unknown frees and unrecorded/realloc-to-zero events remain outside the coverage. The unsuppressed replay is not an exact canonical leak oracle.

No final memory budget, incremental per-bot cost, absolute RSS budget, N/N+x scaling result, lifecycle/post-Stop plateau, responsiveness, rendering cadence, target-hardware result, or campaign acceptance gate is established by this diagnostic. The campaign's approved finish line remains unresolved: TUI/panel RSS and incremental budgets, CPU/simulation/latency, rendering and last-FBO behavior, lifecycle/resource pressure, 1/16/32/128 validation, and required final Grok 4.6 whole-branch review all remain separate work.

The historical failed report `heaptrack-owner-replay-native-result.md` and its JSON companion are preserved unchanged. This report is evidence only; it makes no code, classifier, parser, demangling, capture, replay, build, test, remote, server, cache, account, merge, or push changes.

## Local extraction note

The first local extraction attempt failed before extraction because the old Python tar filter did not support the requested filter argument. Bounded member validation then succeeded; no engine retry was performed. The validated local material is the evidence root and archive named above.
