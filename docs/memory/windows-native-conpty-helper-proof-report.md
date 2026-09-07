# Native Windows ConPTY helper and input proof

## Verdict

The preserved N1C artifact is a successful, workload-qualified native Windows ConPTY managed run with identity-bound helper accounting and clean controlled teardown. It proves the managed path observed a real `conhost.exe` helper and that the TUI accepted the declared input-probe workload. It does not prove paired performance acceptance, helper overhead, physical presentation cadence, or completion of the memory campaign.

## Artifact and provenance

- Run: `native-conpty-n1-c`, completed with exit code 0.
- Native host: Windows 11 `10.0.26100`, user `BotTest`; builds were stopped before the run.
- Tooling checkout: `e3188e2522c4681a47e0127c8202caaa0c3b81a8`.
- Frozen TUI binary: source build `b4b686fd8765cc9d1aa880346f440bd6b781246e`, SHA-256 `9f7d5a7ac9a9bcd579be0697ee96cc244b5b667c18171770f0ad53c33e120ebe`.
- The original `b4` N1 binding failure remains preserved. The later `2539cab` reader replay independently bound this run with `qualified=true`, `match_keys_missing=null`, and native/managed resource status available; it did not rewrite the original receipt.
- The root archive was copied before the Windows reboot boundary and verified with SHA-256 `a1df925bbebf7b91909cadc0fd6224be2d86dd9cafc0872f29585dec59791045`.

## Helper ownership and role accounting

The launcher metadata captured the actual ConPTY helper:

| Role | PID | Start identity / evidence |
|---|---:|---|
| launcher | 15936 | `windows_creation_filetime:134332779430904533` |
| `conpty_helper_0` | 18892 | `windows_creation_filetime:134332779459620460`, parent PID 15936, image `conhost.exe` |
| frontend | 22052 | `windows_creation_filetime:134332779459672211` |
| controller | 14160 | `windows_creation_filetime:134332779421553616` |
| collector | 10652 | `windows_creation_filetime:134332779460455294` |
| bootstrap | 25552 | `windows_creation_filetime:134332779419930767` |
| game server | 24224 | `windows_creation_filetime:134332700460815336` |

The sampler was launched with all six explicit roles, including `conpty_helper_0`, at a 0.5-second interval. The `process_accounting.jsonl` receipt contains 330 samples, all 330 reported `ok`, stable role identities, `binding_errors=[]`, `runner_errors=[]`, sampler `exit_code=0`, `completion=controlled_stop`, and `status=closed`. Separately, the observation-window managed-resource records report 240 `resident_sample_count` samples per role, including `conpty_helper_0`; that per-role count is not the full sampler receipt count. Teardown created the collector stop file at 18:08:30.931Z; collector and launcher both exited 0, and the frontend had no signal or orphan risk. Cleanup does not record a `conpty_helper_0` exit code: the helper remained identity-stable with `status=ok` through `controlled_stop` (`observed_span_s` approximately 164.49 seconds), so this evidence does not establish the helper's true process exit. The top-level run ended at 18:09:30.021Z/18:09:30.405Z in the preserved metadata and completion receipts.

This is direct role accounting for the declared roots, not arbitrary descendant-tree coverage. The sampler explicitly documents that child current RSS, Windows host pressure, and descendant processes are not measured. The run also has no declared helper-overhead comparison: `overhead.status=unavailable`, `has_helper_snapshots=false`, and `continuous_helper_series=false` in the derived analysis. The continuous role series proves identity-bound sampling, not the incremental cost of the helper.

## Input coverage and latency

The run requested `--tui-input-probes` and recorded 119 `o` writes during the 120-second observation window, one approximately every second. All writes were bracketed as `observe-start`; the raw probe endpoint is explicitly `terminal write only; not visible acknowledgment`. The final restore happened after observation, and the probe file completed with `sent=119`, `stopped=false`, and no write error.

The offline derived input gate reports:

- `input_coverage_complete=true`;
- 117 in-window input samples in the histogram (`buckets_delta[0]=117`), with two writes excluded by the observation boundary;
- `visible_ack_available=true` and `sample_age_ms=63` for the reported endpoint;
- diagnostic p99 bucket `0–5 ms`, target `100 ms`, verdict `meet`.

The last two points must not be conflated. The raw probe timestamps measure writes into the terminal transport, not application acknowledgment. The derived visible-ack/input histogram is useful diagnostic evidence for this run, but it is not a direct key-to-application latency measurement and has no scanout or hardware-frame component. No input-latency acceptance claim is made here.

## Workload and resource observations

The native qualification was `qualified=true`, `errors=[]`, `n=1`, active TUI, terminal size 120x40, 119.2236046 seconds of observation, and exit code 0. The run recorded 4.046875 process CPU seconds (`0.0339435719` cores), median resident bytes 181,002,240, peak resident bytes 187,375,616, and 48.7068 client ticks per slot-second with four steal gains. Those are diagnostic observations for this single candidate run; the resource gate remains unavailable because resource provenance/overhead comparison was not declared as an accepted pair.

The process sampler separately reports the server resource series as available, but the helper path itself was not used to establish an OFF/ON overhead delta. GPU completion was not requested (`gpu_completion_profile=false`), and the derived analysis warns that callback/completion timing is not physical presentation or 40-fps proof.

## Boundaries and missing metrics

- No paired control/candidate or instrumentation-overhead cells; no helper overhead acceptance.
- No physical scanout or hardware GPU-completion proof; callback cadence is not frame rate.
- No final memory-budget, CPU-regression, p99 responsiveness, or matched-savings acceptance.
- No broad descendant-tree inventory beyond the declared roles; Windows pressure is unavailable rather than zero.
- Raw terminal writes do not independently prove application receipt latency; the derived 0–5 ms input bucket remains diagnostic.
- This run does not establish lifecycle/scaling, N=16/N=128 capacity, or full-campaign success.

## Sources

- `docs/memory/diagnostics/windows-native-conpty-e3188e2-c/20260907T180543Z_tui_n1_active/metadata.json`
- `docs/memory/diagnostics/windows-native-conpty-e3188e2-c/20260907T180543Z_tui_n1_active/input-probes.jsonl`
- `docs/memory/diagnostics/windows-native-conpty-e3188e2-c/managed-conpty-e3188e2-c/cells/native-conpty-n1-c/receipt.json`
- `docs/memory/diagnostics/windows-native-conpty-e3188e2-c/managed-conpty-e3188e2-c/cells/native-conpty-n1-c/cell_report.json`
- `docs/memory/diagnostics/windows-native-conpty-e3188e2-c/managed-conpty-e3188e2-c/cells/native-conpty-n1-c/process_accounting.jsonl`
- `docs/memory/diagnostics/windows-native-conpty-e3188e2-c/managed-conpty-e3188e2-c/independent-binding-reader-2539cab.json`
- `docs/memory/conpty-helper-accounting-report.md`
- `docs/memory/windows-native-reader-replay-review.md`
