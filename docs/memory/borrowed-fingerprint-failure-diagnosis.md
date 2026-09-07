# Borrowed-fingerprint candidate failure diagnosis

Captured: 2026-09-07 UTC
Status: diagnosis only; the failed candidate cell remains failed and is not a performance result.

## Question and bounded conclusion

The corrected Linux screen's candidate cell `02-candidate-n16` launched once and
failed at 17.386014077 s. The preserved failure is:

`live256c5_12: tick 19: Unknown error`

The evidence establishes a candidate-process functional failure during early
startup/qualification, but does not establish that the borrowed-fingerprint
change caused it. It also does not establish a fixture or server failure. The
narrowest supported classification is: an isolate/script error was surfaced as
a generic tick error while the 16-slot workload was still converging; the
available artifacts omit the underlying exception and the encode operation that
preceded it. Causal attribution is therefore unresolved.

No RSS, CPU, cadence, latency, or candidate-vs-reference comparison is valid.

## Evidence checked

The approved screen reports are present under `docs/memory/diagnostics/` rather
than at the named root paths (for example,
`docs/memory/diagnostics/borrowed-fingerprint-corrected-native-screen-report.md`).
That placement is preserved intentionally: these approved reports must not be
moved or edited as part of this diagnosis.

### Cell receipt and qualification boundary

- `02-candidate-n16/cells/02-candidate-n16/cell_report.json` records
  `launched: true`, `attempts: 1`, launcher exit 1, and
  `status: failed_or_unavailable`. It also records only one qualification line
  (`qualification_line_count: 1`), not the 16-slot failure boundary. The
  candidate binary SHA-256 is recorded in the sibling `receipt.json` and in
  `verified-build-preflight.json` under `files.binary.sha256`:
  `10ddc915c7b0f5ef9f3fbb9443c95bc74249303d9a048fcdc4fb4e4fc69af008`.
- `02-candidate-n16/cells/02-candidate-n16/receipt.json` records only
  `runner:unexpected qualification phase` and `runner:missing observation
  boundary` as runner errors, plus `frontend_failed_or_incomplete` and
  `launcher_failed`. It records no exception text, stack, snapshot field, or
  operation name.
- The receipt/cell report `run_dir` sibling
  `docs/memory/diagnostics/borrowed-fingerprint-native-screen-20260907b/20260907T203813Z_tui_n16_active/samples.qualification.jsonl`
  holds the one `failure-boundary` record with 16 slots in mixed
  `Idle`/`Running` states. The corresponding `samples.jsonl` in that run
  directory has 17 seed-phase samples and zero observe samples. The cell-dir
  path `02-candidate-n16/cells/02-candidate-n16/samples.qualification.jsonl`
  does not exist. Thus there is no observe-start or observe-end boundary and
  no qualified candidate sample.
- The failing slot record has `state: Running`, `error: "tick 19: Unknown error"`,
  `ingame: true`, `scene_state: 2`, and position `(2661, 3306, 0)`. Its runtime
  has only 2 dispatched ticks and `last_completed_tick: 20`; its paint is still
  `ThievingBot — starting`, with one-item Lobster rows. Other slots had not
  reached a common steady state (some were Idle, some Running).

### Binding, fixture, host, and timing controls

- `server-identity.json` and the receipt bind the same existing server PID 152004
  and Linux start identity `linux_proc_start_ticks:241214967`; the server was not
  started, stopped, or changed by this cell.
- `host-conditions.json` identifies Ubuntu 24.04.4 x86_64, 2 CPUs, 2,014,852 kB
  RAM, no swap, and 829,632 kB MemAvailable. This exceeds the 128 MiB guard.
  CPU and memory pressure fields show no full pressure at setup. This does not
  prove the host was otherwise noise-free, but supplies no direct resource-guard
  explanation.
- `fixture-cache-preflight.json` verifies the same packed client inputs and
  unpack snapshot identity used by the declared cell; the post-completion cache
  verification succeeded. The receipt's sampler also closed cleanly with exit 0.
- The runner used the declared candidate binary, `--warmup 30`, `--observe 120`,
  `--no-diagnostics`, and `--failure-capture`; there was no retry or changed
  timeout. The controller stopped the collector after the launcher failed, so
  the controlled sampler exit 0 is cleanup status, not frontend success.

### Reference comparison

The preceding reference N=16 cell qualified with exit 0, 117 native observation
samples, all 16 slots ready/active at both boundaries, `ingame=true`, and
`scene_state=2`. This shows the server/fixture/protocol can support this declared
cell, but it does not eliminate timing sensitivity or prove that candidate code
is responsible for the different outcome.

The reference and candidate use the same client commit and client source proof.
The frozen host inputs differ exactly as declared: reference host snapshot
`763700b6c3f7f92e617caa0cf052eaccfab05b56b2c840b068f519de27dd7098`; candidate
snapshot/`host_sources_sha256` `f7143dc74ae74b598e486ce7e6b758d7a6fe3b5d2cbe101da986c5261301613c`.
The candidate commit is `188a520`; the exact Rust code ownership difference is
`crates/script/src/isolate_fb.rs` and `crates/script/src/slot.rs`, changing the
live snapshot delta path from constructing/replacing a full owned fingerprint
to borrowed comparison plus field-selective retention. There is no client diff.
Both matched builds and their script/host-play/TUI feature tests exited 0 in the
freeze receipt.

## Candidate-code plausibility, without causal overclaim

The changed live path is `SlotScript::encode_snapshot_delta` in
`crates/script/src/slot.rs:248-259`, calling
`IsolateBuf::encode_snapshot_delta_updating`. In
`crates/script/src/isolate_fb.rs:2296-2321`, the candidate compares the borrowed
input, encodes the masked snapshot, then retains only changed fingerprint fields.
The candidate's local oracle, unchanged-post pointer, force-banks, and restart
coverage all pass. These tests prove intended equality/retention behavior for
constructed inputs; they do not reproduce this multi-isolate startup failure.

The failure text is consistent with the existing error-propagation boundary:
`SlotScript::drain_logs` retains the latest isolate log beginning with `tick `
(or a script-stop line) as `last_error` (`crates/script/src/slot.rs:390-406`).
The preserved output contains only `Unknown error`, not the JS exception value or
host operation. There is no evidence of a Rust panic, allocation failure, cache
mismatch, server identity mismatch, or qualification-reader false pass. The
candidate change is temporally associated with the failed binary, and its
snapshot encode path is a plausible suspect, but the artifacts do not connect
that path to slot 12's error.

## Verification performed

- Executed the existing read-only review scripts
  `docs/memory/_review_corrected_native_screen_r1.py` and
  `docs/memory/_review_corrected_native_screen_r1d.py`; they confirmed the
  failure boundary, mixed slot states, slot-12 error, matching hashes, and
  absence of a candidate observation window.
- Executed `cargo test -p script --features load`: 42 library tests, 1 catalog
  test, 3 declared-ABI tests (1 ignored), 11 gold-stub tests, 2 host-JS tests
  (1 ignored), 11 JS-cache tests, 144 load-isolate tests, 1 picker test, 11
  load-shape tests, 7 sibling tests, 2 loadout tests, 4 registry tests, 19
  rs2b0t-registry tests, 5 settings-bag tests, 8 slot tests, 14 unloadable
  tests, 6 walk-to tests, and 0 doc tests passed.
- Ran `git diff --check e3188e2..188a520`; it reported no whitespace errors.
- No live frontend, rebuild, network/VPS action, retry, or production-code edit
  was performed.

## One actionable next step

Root should run exactly one bounded, failure-only diagnostic with the same frozen
candidate binary, fixture, 16-slot declaration, and timeout, changing only the
existing diagnostic switch from `--no-diagnostics` to `--debug` (equivalently,
set `BOT_DEBUG=1`). In this path `host-play`'s existing
`emit_script_debug_logs` drains isolate log lines, including the
`interrupted slow tick ...` line emitted by `crates/script/src/load.rs:1119`
when the slow-tick budget interrupts a tick. The discriminator is therefore
whether the preserved slot-12 `tick 19` failure has an interrupted-slow-tick
line or only the generic error/other drained lines.

This does not assume that failure capture can preserve a JS exception
value/message, a snapshot field mask, or an encode event. The frozen binary has
no encode-event instrumentation, so absence of such an event is not evidence
against the candidate and must not be used to park it. If the run does not
reproduce or still reports only `Unknown error`, the result is inconclusive:
make no causal attribution and neither accept nor park the candidate on that
basis. Do not collect RSS/CPU data, alter the fixture or script, retry until
pass, or relabel the existing failed cell.
