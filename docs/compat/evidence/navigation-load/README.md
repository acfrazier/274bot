# Navigation loading origin qualification

Reviewed product host `d2372fe018f3fdaf69b90fe25ba25247ef74ff53`, client
`56d80272bcbda3eb1e22db096c1c5e21d3497de4`. Same-card Grok 4.5 review
run 1191/session `20260910_220016_dea506` approved the source. Root built each
private variant in its own new empty Cargo target; all original source hashes
were checked before and after. Private-source manifests identify the added
probe/interactive entry and the bundled variant's manifest-derived table.

All four revision/origin probes passed. Each selected pack is read and decoded
once across bind, shared template load and validation for Play. External packs
have one content hash and one custom-navigation hash-stage start; bundled packs
have zero. Both paths keep the same world Arc, retain it after the owned disk
copy is damaged, and refuse a new binding to the damaged copy. The helper
restores each owned pack; original runtime packs were never modified. Cache
identity checks remain in place. `qualified-results.json` contains exact facts;
raw logs, commands, binary hashes and receipts are alongside it.

Root launched both isolated native variants for revision 289 and read the
resulting screenshots. Each reached the ready empty-vault panel without a
remaining progress bar and exited normally. No bot login was requested. The
captures occurred after startup completed: they verify the ready UI, while
the matched probe counters establish skipped/reused hashing. The external hash
bar was not captured during these two short runs. Earlier bar observations
retain their historical build-provenance qualification.

This is a private unsigned local bundle assembled from the actual manifests,
not a shipped release or a startup-performance comparison. Raw elapsed times
include concurrent diagnostic work. Packaging/signing and full catalog/front-end
acceptance remain separate. The initial shared-target compiler failure is
preserved as `build-external-nav_origin_probe.*`; it is not a gameplay failure.
