# One bounded Linux failure discriminator

Declared by root on 2026-09-07 before launch. The original candidate N16
failure remains failed. Diagnosis t_0107305c was approved by actual Grok4.5
run392/session20260907_172326_c51e20; its proposed next step needs the precise
launcher qualifications below.

Run exactly one candidate N16 diagnostic with the same frozen188a520 binary,
client3456, fixture/cache, local server identity and existing30/120/60 timing.
Retain `--no-diagnostics` and `--failure-capture`; add only `--debug`. The launcher
clears inherited BOT_DEBUG and sets it when this flag is supplied. These flags
are independent, so removing no-diagnostics is not equivalent to adding debug.
Use a separate output tree, not another cell in the aborted matched batch.

The frozen memory binary and managed runner still emit their normal resource
samples. They remain raw diagnostic evidence and cannot support RSS/CPU/latency
acceptance or a matched saving. No resource claim is authorized from this run.
No Rust rebuild, script/fixture edit, relaxed deadline or retry is authorized.

Discriminator: preserve drained logs for all slots, correlate any tick error
with preceding `interrupted slow tick` for that same slot/tick. The literal
Unknown error is emitted by rustyscript0.12.3 inner_runtime.rs when V8 TryCatch
has caught something but has no message; host load.rs can interrupt slow ticks.
That code path makes interruption a hypothesis, not proof of the original cause.
No exposed field-mask or missing exception-value capability is assumed.
A clean or still-generic result is inconclusive about the original failure.

Runner `diagnostics/borrowed-fingerprint-failure-debug-20260907/run_debug_failure_cell.py`
SHA256 da98e6073a7d31c08a0c9d6f613e995a81cfb5df7eb88eb8b9f71dc1f5196904 is a
minimal adaptation of the corrected batch runner. It restricts index1 to
candidateN16, changes purpose/kind/output, adds debug, and verifies the previous
immutable fixture expectations rather than creating fresh expectations. Same
binary/build/cache/source preflights,128MiB memory guard and sampler cleanup.
Local syntax and exact diff were checked; run --prepare-only before live use.
