# Cache receipt binding

The receipt writer optionally accepts `cache_provenance_path` before launch.
It recomputes the complete current fingerprint with `verify_snapshot`, binds
the artifact path/hash and content identity, and records verification time.
At completion it rechecks the artifact bytes and recomputes the full fingerprint
again. Any changed local input, new preferred store, missing file, or malformed
snapshot produces `cache_provenance_changed_or_invalid` in the durable receipt.

`verify_snapshot` compares a freshly captured complete scope with the supplied
snapshot, including typed JSON content. Merely omitting an inconvenient record
does not bypass verification. File hashing occurs outside the observation.
The optional argument preserves existing receipt callers; absence still means
no cache proof. The runner must capture/pass it explicitly before future cells.
This is local file evidence; network-loaded asset contents are still outside
the fingerprint. There is no performance or memory-residency assertion.

Validation: 16 cache/receipt tests passed, including complete-scope verification,
omitted record rejection, successful pre/post binding, and changed cache bytes
causing failed completion. No live frontend/server run or build.
