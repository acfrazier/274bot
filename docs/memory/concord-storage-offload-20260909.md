# Concord completed-capture offload

Operator authorized downloading worthwhile artifacts missing locally and removing
verified remote copies as work proceeds. This is storage maintenance, not a
performance result. It applies to Concord VPS, distinct from the Hyper-V builder.

On 2026-09-09 Concord initially had 11,807,068,160 available bytes on its
52,002,074,624-byte filesystem. Completed capture
`/home/acfrazier/274bot-campaign/current-owner-heaptrack-n1-2003` held roughly3GB.
No active heaptrack, frontend, compiler or native profiling process was found.

All23 source files were hashed. Existing21 local summaries/receipts matched;
the two missing allocation traces were downloaded to the same local evidence
folder. Each download was length/hash checked, synced and atomically renamed.
Then the complete local23-file capture was independently rehashed. Before remote
removal the two remote traces were rehashed, checked for ownership/regular-file
identity and open file descriptors. Only those two exact files were unlinked.

| File | Bytes | SHA256 |
| --- | ---: | --- |
| allocation/alloc.raw | 2,149,010,072 | `16f51266e56bd065f891ad822a00659a3bd0803aef8f748c82cde5e492b5ca29` |
| allocation/alloc.interpreted | 810,563,095 | `cb005f5e7bae1869fbd5c5552fb4e5499fed6b25c43b74c9964c1d11f0ed076e` |

Total logical bytes offloaded: 2,959,573,167. Available space increased by
2,959,585,280 to14,766,632,960 bytes. Remote small summaries remain, with
`allocation/archived-locally-20260909.json` identifying the local destination and
hashes. No live source, cache, server, executable, unrelated application or
Hyper-V builder artifact was removed.

Complete local evidence:
`diagnostics/owner-capture-evidence-2015/current-owner-heaptrack-n1-2003/`.
Source manifest, download receipts, local full-copy verification and remote
removal receipt: `diagnostics/concord-storage-20260909/`.
Future consumers must use the local traces or explicitly stage a verified copy;
the historical source capture paths in earlier reports now identify archived
inputs and should not trigger an unbounded recapture/retry.
