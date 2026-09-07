# Local client cache fingerprint

`cache_provenance.py` fingerprints explicitly named local cache paths outside
measurement windows. Its input selection follows the current client source:
`bot_target.rs` chooses the cache/unpack roots; `unpack::version_hash` uses the
first eight SHA256 bytes (16 hex characters) of `versionlist`; `load_snapshot`
reads that version's `models.bin` and `anims.bin`; the OnDemand cache reader can
use backing stores in the cache directory and its parent.

The fingerprint records eight jag files, the two active snapshot files, and
presence/absence plus hashes of `main_file_cache.dat` and indices 1–4 in both
possible store directories. These are content/configuration inputs, not claims
that every file is read or resident. File and base-directory symlink bindings
are explicit. Rechecks reject changed bytes, paths or newly present inputs that
could change fallback resolution. A deterministic content digest excludes
physical paths; consumers must separately compare configuration paths.

Six tests passed: actual files and absences, missing required snapshot,
changed backing-store bytes, newly present preferred store, base-symlink change
and content-digest sensitivity. A read-only local capture is at
`diagnostics/cache-provenance-20260907T051034Z.json`: 15 present files,
15,147,404 bytes, active version `2faf336eeb0462ed`, content SHA256
`2a660d20eaccd1491da88e4b4d2e1ba0e0bdea7c73dbda954f8df4f97bb9d9a3`.
This diagnostic capture is not attached retroactively to any old live run.

No cache content, client source, launcher or historical artifact was changed.
The utility does not capture network-delivered asset contents, perform workload
qualification, measure resident memory, or establish performance acceptance.
Future managed runs must record their own pre/post fingerprints and compare
the paths against native qualification settings. This utility does not infer
the actual running client's selected path from a launcher default.
