# Frozen direct-owner preparation tooling

This directory is intentionally fail-closed. It prepares only the original host/client Git objects plus the three reviewed patch receipts; it never uses the moving checkout as source input.

Prepare into a fresh path:

    ./prepare_frozen_source.py prepare --output-dir /absolute/fresh/output

Verify an already prepared archive:

    ./prepare_frozen_source.py verify --output-dir /absolute/prepared/output

Run the bounded native Linux qualification from the committed tooling directory:

    ./qualify_linux.py \
      --archive /absolute/prepared/output/frozen-direct-owner-source.tar.gz \
      --manifest /absolute/prepared/output/frozen-source-manifest.json \
      --output-dir /absolute/fresh/qualification-output \
      --mode linux

A non-Linux source check is available only for local compilation evidence:

    ./qualify_linux.py \
      --archive /absolute/prepared/output/frozen-direct-owner-source.tar.gz \
      --manifest /absolute/prepared/output/frozen-source-manifest.json \
      --output-dir /absolute/fresh/source-check-output \
      --mode source-check

`source-check` never emits a Linux qualification. Neither mode launches a frontend, live test, account, cache, or server. Runtime owner capture is forced off except where the isolated generated test explicitly enables its in-process seam.

Prepared artifacts are under `artifact/frozen/`. The original blocked audit remains at `artifact/`; it is not rewritten as success. The successful audit admits exactly two original-plus-overlay host files as test-only derivations with bound full hashes and byte-identical production prefixes; all other touched host files and all touched client files match the reviewed commits exactly. The complete derived difference is `artifact/frozen/derived-vs-reviewed-test-only.patch`.

Compact macOS source-check receipts are under `artifact/local-source-check/`; extracted source and Cargo target output remain outside the repository. That check is not Linux or live qualification. See `docs/memory/direct-owner-native-preparation-report.md` for identities, commands, results, and the remaining root prerequisites.
