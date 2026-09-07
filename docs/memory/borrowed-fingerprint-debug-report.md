# Borrowed-fingerprint Linux failure discriminator

Date: 2026-09-07

## Result

Exactly one live diagnostic was launched for the frozen candidate N16 cell. The
original failure was not reproduced as the same `Unknown` failure: the managed
cell completed its launcher path with exit code 0, but qualification failed
because active progress for `live257e0_9` was missing. This is a different
failure at the qualification layer, and the result is inconclusive about the
original candidate failure. No resource, latency, CPU, or RSS claim is made.
No further live run is authorized by the stop rule.

## Frozen inputs and exact invocation

- Host branch/build commit: `codex/memory-diagnostics`, build commit
  `188a52095f3a8a6db9b2567f5278993b80876852`; the recorded build provenance is
  in `diagnostics/borrowed-fingerprint-failure-debug-20260907/01-candidate-n16-debug/build-manifest.json`.
- Candidate binary: `tui-play-candidate`, SHA256
  `10ddc915c7b0f5ef9f3fbb9443c95bc74249303d9a048fcdc4fb4e4fc69af008`; the
  receipt records this at `diagnostics/.../cells/01-candidate-n16-debug/receipt.json:57`.
- Client commit: `3456edc8dabf7b25ada78110ffa56327af9f67a4`; host/client source
  provenance is retained in `diagnostics/.../client-source-proof.json` and
  `diagnostics/.../run_dirs/20260907T213227Z_tui_n16_active/metadata.json:33-38`.
- Runner: `run_debug_failure_cell.py`, SHA256
  `da98e6073a7d31c08a0c9d6f613e995a81cfb5df7eb88eb8b9f71dc1f5196904`,
  retained at `diagnostics/borrowed-fingerprint-failure-debug-20260907/run_debug_failure_cell.py`.
- Remote invocation: `ssh concord python3 /home/acfrazier/274bot-campaign/borrowed-fingerprint-failure-debug-20260907/run_debug_failure_cell.py --index 1`.
- The receipt's effective diagnostic CLI retained `--no-diagnostics` and
  `--failure-capture`, and added only `--debug`, with `--warmup 30` and
  `--observe 120`; see `diagnostics/.../cells/01-candidate-n16-debug/receipt.json:6-25`.
- The run used the frozen server PID 152004 and start identity
  `linux_proc_start_ticks:241214967`; see `diagnostics/.../cells/01-candidate-n16-debug/receipt.json:80-103`.

## Observed evidence

The managed runner returned one completed attempt and one unavailable
qualification result (`diagnostics/.../run_debug_failure_cell.py` stdout retained
in the task run capture). The cell report records `attempts: 1`, `launched: true`,
`performance_acceptance: false`, launcher exit code 0, and final status
`completed` at `diagnostics/.../cells/01-candidate-n16-debug/cell_report.json:1-7,75-83,134-138`.

The independent binding artifact identifies the qualification failure as
`missing active progress: live257e0_9` at
`diagnostics/.../01-candidate-n16-debug/independent-binding.json:196-208`.
The qualification snapshots show the observed slot state as `Running` with
`error: null` at both observe-start and observe-end in
`diagnostics/.../run_dirs/20260907T213227Z_tui_n16_active/samples.qualification.jsonl:1-2`.
The observe-end snapshot also records `ingame: true` and `scene_state: 2` for
slot 0, so this was not a blanket frontend startup failure.

The drained frontend log is retained at
`diagnostics/.../run_dirs/20260907T213227Z_tui_n16_active/run.log` and contains
normal slot thread/handshake messages and host hitch diagnostics. A search of
the retained batch and run directory found no literal `Unknown`, `tick error`,
or `interrupted slow tick` record to correlate for the failed slot/tick. That
absence does not establish that interruption was not causal; it only means this
run provides no positive same-slot/same-tick correlation evidence. The protocol
therefore requires the result to remain inconclusive rather than treating log
absence as proof.

## Artifact capture and integrity

The full new batch, including its preflight, receipt/cell directories, and all
referenced run-directory output, was copied under
`diagnostics/borrowed-fingerprint-failure-debug-20260907/` without including
other batches. Transfer archives and local SHA256 records are retained:

- `remote-capture.tar.gz` SHA256
  `9d3c3591383cd68ef50b44a982a2533a410ca125117bf26eb0d75e2246dd1fbf`
- `run_dir-20260907T213227Z_tui_n16_active.tar.gz` SHA256
  `e42511f087d774fd941a4dbee68b5b9b26f46f020f32a9cfa386f77db296ae38`
- Local runner SHA256 matches the declared
  `da98e6073a7d31c08a0c9d6f613e995a81cfb5df7eb88eb8b9f71dc1f5196904`.

## Next action

Park this discriminator as an inconclusive, non-acceptance diagnostic. Under
the approved stop rule, do not run another live cell; root may use the preserved
same-slot evidence for offline diagnosis or revise the plan through a separately
approved task.
