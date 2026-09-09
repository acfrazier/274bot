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

Current status: preparation is blocked before archive creation because the read-only host patches materialize two members that do not match their reviewed receipt hashes. See `artifact/provenance-audit.json` and `docs/memory/direct-owner-native-preparation-report.md`.
