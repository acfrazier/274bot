# Managed launch/completion receipt writer

`managed_receipt.py` provides two functions for the next managed collector:
`create_launch` writes an exclusive pre-launch record with the exact launcher
argv, cell identity/order/kind, sampler configuration, binary SHA256 and named
manifest/server/host sidecar paths and hashes. `complete` writes an exclusive
completion receipt with frontend and launcher exit codes kept separate, raw
metadata/sample/qualification file hashes, launch-record hash and sampler output
hash. It checks the process time envelope and rechecks pre-launch bindings.

Missing raw files, absent run directory, unsuccessful exits, changed inputs or
inconsistent metadata remain recorded as `failed_or_unavailable` cells with
`binding_errors`; they are not dropped. Raw artifacts are never enriched or
rewritten. The API must be called at the actual launch and completion events;
it does not reconstruct historical launch records or execute any processes.

Seven tests passed with real temporary artifacts and a deterministic synthetic
clock: positive byte bindings, external mutation, separate failed launcher and
successful frontend, missing run/raw preservation, CLI/time mismatch, exclusive
creation and failed/missing sampler output. The positive fixture contains
deliberately minimal raw JSON: the writer is not a workload qualifier or
metrics parser. The evidence reader must independently validate all schemas,
hash bindings, qualification, matching, observation coverage and overhead.

This is a writer utility for integration into a future managed run, not an
executed measurement or a performance-acceptance gate. It has no automatic
retry, process discovery, subprocess, environment dump or signal behavior.
