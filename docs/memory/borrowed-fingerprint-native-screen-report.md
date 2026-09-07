# Borrowed fingerprint native Linux screen — stopped preflight

Captured: 2026-09-07 UTC
Status: stopped before the first live cell; no performance result.
Performance acceptance: false

## Protocol and fixture

The predeclared order was R/C/C/R at N16, then R/C/C/R at N1, using the reviewed immutable TUI pair, real PTY 120x40, 30 s warmup, 120 s observation, 60 s teardown, `--no-diagnostics --failure-capture --sustain`, 0.5 s managed sampling, and a 128 MiB MemAvailable stop guard. The immutable binaries were not rebuilt or modified:

- reference source: `e3188e2522c4681a47e0127c8202caaa0c3b81a8`
- candidate source: `188a52095f3a8a6db9b2567f5278993b80876852`
- client: `3456edc8dabf7b25ada78110ffa56327af9f67a4`
- reference binary SHA-256: `019c35779271f150bb83fb7cf6fcb755a4abee8abd1baa2378dcf9e76c932b76`
- candidate binary SHA-256: `10ddc915c7b0f5ef9f3fbb9443c95bc74249303d9a048fcdc4fb4e4fc69af008`

## Preflight and exact failure

Concord preflight succeeded for the environmental checks immediately before the attempted cell: Ubuntu 24.04 x86_64, listener `127.0.0.1:43594` present, server PID `152004` present, and MemAvailable `951332 kB` (above the 128 MiB guard). The server was not signaled, restarted, or changed.

The first declared cell was `reference_n16_01`. Its managed controller executed exactly once and exited before launch:

- controller status: `preflight_failed`
- attempts: `1`
- launched: `false`
- exact error: `invalid client sources`
- started: `2026-09-07T20:09:09.037215Z`
- ended: `2026-09-07T20:09:09.040672Z`

The failure was in the transferred adapter build-manifest binding: the initial manifest encoded the client source value as the 40-character client commit rather than a 64-character source digest. This was caught fail-closed by `build_provenance.verify_build` before the launcher started. The sequence was stopped as required; the failed cell was not silently retried and no later cell was attempted.

The failed raw cell receipt and exact spec are preserved at:

`diagnostics/borrowed-fingerprint-native-screen-20260907/cells/reference_n16_01/`

The dedicated remote directory and immutable pair remain at:

`/home/acfrazier/274bot-campaign/borrowed-fingerprint-native-screen-20260907/`

The original freeze evidence remains unchanged. The corrected manifest was not used to run another cell.

## Measurements and claims

No frontend launched, so there are no RSS medians or startup peaks, CPU seconds, wall seconds, ready/active/progress counts, iterations, server series, helper accounting, adjacent comparisons, replicate variation, or N1–N16 finite differences. No RSS, CPU, allocation-removal, latency, renderer, or final-budget claim is supported. No CI or campaign acceptance claim is made.

## Follow-up boundary

A future attempt requires a fresh, explicitly approved screen after reviewing and correcting the manifest binding. It must not overwrite or reuse `reference_n16_01`; this stopped preflight artifact remains evidence of the failed attempt.
