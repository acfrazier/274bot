# Lossless diagnostic compression for publication

GitHub rejected the first stage-2 push because two N32 raw JSONL blobs were
larger than 100 MiB. The remote main remained at 76b2016b7. Only unpublished
commits were rewritten, retaining individual messages, authors, committers,
parent topology and every product source blob. No squash or force push.

The receipt maps historical commit IDs to their publishable equivalents.
Historical review/build receipts retain their original IDs; source bytes and
the aef3952d client gitlink are identical. This is an artifact packaging change,
not a new functional or performance result. All failed runs remain retained.

The two diagnostics now use `.jsonl.gz`. Decompressed SHA-256 values match the
original process receipts exactly. To run the historical harvest helper, first
restore its expected raw filename with `gzip -dk samples.diagnostics.jsonl.gz`
inside each N32 evidence directory. Keep the original receipt unchanged.
The original local history is preserved at codex/stage2-before-compression.
