# Mac project cache cleanup — 2026-09-09

Operator requested project cleanup after near disk exhaustion. Root measured
282078736 KiB (about269 GiB) at the beginning of this pass, after the operator's
own cleanup. About198 GiB belonged to the active campaign checkout.

Root removed92 explicitly inventoried inactive Rust debug intermediate
directories: deps, incremental, build and .fingerprint under older worktree
caches and completed coverage/fingerprint diagnostic build caches. Before
removal, root verified directory device/inode, resolved paths, absence of
tracked files, no matching process arguments, and no open files reported by
lsof. No source/worktree/branch, raw log/measurement, frozen archive or release
output was selected. The active campaign's general target and current worker
outputs were excluded. Directory-by-directory receipts preserve the exact list.

Observed filesystem free-space increase: 137809018880 bytes (128.34 GiB).
This is observed free-space change during cleanup, not a sum of apparent file
sizes; hard links and concurrent activity can make those differ. The candidate
inventory's allocated-size sum was136.19 GiB. Filesystem reported about452 GiB
free afterward. Current workers continued and transcript a3abbdd entered review.

Evidence: diagnostics/mac-project-cleanup-20260909/
inactive-debug-candidates.json and inactive-debug-cleanup.json. Workflow now
requires explicit cache locations, reuse when functional test isolation permits,
and root-owned cleanup after preserving evidence. Matched performance isolation
and all acceptance requirements remain unchanged.

Final project allocation:147192456 KiB (140.37 GiB), down from269.01 GiB at
the start of this pass. About128.6 GiB less allocated in the project tree.
