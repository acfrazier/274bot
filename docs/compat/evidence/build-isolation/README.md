# Diagnostic build isolation correction

At 2026-09-11 02:09 UTC, root's private navigation probe failed to compile
against frozen host d2372fe0. The source had the old InteractReq::UseOn and
ItemRowInput shapes, while Cargo reused a script rlib containing the new
uncommitted source_item_id/source_item_slot/target_item_id/target_item_slot
and slot fields from another build. The compiler-artifact record marked that
script dependency fresh in the shared campaign target directory.

The unchanged frozen source was hash-verified before this build. Source-file
hash verification therefore did not establish that a shared-target binary
contained precisely that source. The retained compiler messages are in
navigation-load/build-external-nav_origin_probe.jsonl and .log. No product
source change was made to address those mismatched artifacts.

Historical runtime receipts still identify an actual binary hash, commands,
observations, failures and process outcome. Their host_commit/source_root fields
identify the intended source snapshot; they must not be treated as proof of the
exact compiled-source combination when the build reused a shared target.
This qualification applies to earlier source-specific claims from the affected
diagnostic builder. Static/source reviews with independently isolated builds
remain separate evidence; this issue does not change the observed gameplay.

Root changed the private Mac diagnostic builder to use a new empty Cargo target
per source/build label, and the private navigation variants each get a separate
target. Within a target the same immutable source is used for headless and
native binaries. Receipts name the target and carry isolated_build=true.
Launch helpers now refuse to start a new LIVE cell without that receipt flag.
Existing cells finish and preserve their observations; batch chains then stop
before launching a next cell. No raw receipt, source snapshot or failed log is
overwritten. Reviewed source is rebuilt, and qualifying catalog/native cases
are rerun on the isolated candidate before exact-source acceptance.

## Subsequent isolated checks

The a4157243 immutable export rebuilt both catalog binaries in an initially empty
source-specific target. Root independently reran generated wearpos (1), script
loadout store (10), loadout isolate composition (4), gold stubs (12 plus 1 ignored),
TUI loadout (6) and panel loadout (5) tests in that same unchanged export/target;
all passed. Receipts and before/after source hashes are retained here.
Actual Grok 4.5 useOn review 1194 also used an independent empty target.
The subsequent 50f2be8a native layout check and navigation variants use their
own separately isolated exports and targets. Shared-target historical results
are retained separately from these newly qualified checks.
