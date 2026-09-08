# Native cohort input-path diagnostic result

The 2026-09-08 diagnostic localized input loss to an absent capture sender.
All 20 requested Left/Right pulses completed, with 40 down/up edges observed
at Windows, ImGui and `stream_capture_for`. Every stream entry had `channel=0`;
no host drain or metric admission was observed. All published input counters
remained zero. This is a functional diagnosis, not latency or performance acceptance.

## Provenance and completion

The [predeclared protocol](native-cohort-input-trace-protocol.md) used candidate
host `984c63993cebcf4dc3ff6cc965031e896dc685a4`, client
`fd956c91bf09e059359c8e182a33583e2c626cd3`, Windows binary SHA256
`8247b70397a0e1580e222ff5d530d9173e1f3d6c3401a794425620e1ac5b9f91`.
Both native roles have verified builds and scoped regressions recorded in
[the build receipt](windows-cohort-native-builds.json).

Panel PID13436, start `2026-09-08T12:25:08.7358434Z`, BotTest console session2,
N=1 active, focused-one, 120s warmup / 180s observe, responsiveness+fine,
`BOT_DEBUG=1`. Normal exit0 at `12:31:29.8139802Z`, no timeout. No concurrent
Windows build or frontend was present at launch. The builder VM was off.

A first capture correctly refused the old hardcoded binary binding. That failure
is archived. A separately named capture helper changed only the expected binary
path/hash to this frozen executable. Root read fresh initial, configuration and
after-input captures: scene2, selected sole slot `live347c0_0`, capture checked,
and Game Image visible. Current rectangle was `[114,114,2212,1040]`; the input
helper checked it and used point `[514,414]`. The prior rectangle was not reused.

Exact reviewed helper `f64b81d`, SHA256
`04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1`,
completed20/20 pulses at `12:28:17.5811539Z`..`12:28:36.7596366Z`, no missed
or deferred slots. Captures verify UI state; SendInput completion alone does not
establish host receipt.

## Observed path

| Stage | Emitted records | Observation |
| --- | ---: | --- |
| Gate | 14 | Sender initially present at mono175109100ns, absent by209030200ns |
| Windows arrow | 40 | Left/Right down/up delivered |
| ImGui arrow | 40 | Corresponding edge counts observed |
| Stream entry | 40 | All channel0, slot present, ID `0x783650e976e3ae41` |
| Host drain | 0 | No traced key drain |
| Metric start | 0 | No traced admission |
| Generation bind | 4 | Reserved zero-work heartbeats only |
| Texture present | 4 | Reserved zero-work heartbeats only |

These are independent stage counts, not a per-event correlation identifier.
There were no trace saturation records. `win_focused` in the gate concerns the
ImGui window; OS foreground was separately checked by the helper.
`stream_capture_for` returns immediately when its sender is absent, before
metric admission. The source investigation is therefore about capture attachment,
not an inference that a telemetry row alone caused the zero counts.

The likely lifecycle cause is a memory-policy assignment of `game_pane_open=true`
after a real pane-close callback disconnected capture, bypassing the ordinary
reopen transition. Task `t_f2d0af32` must reproduce and review that cause before
any correction is accepted. No second native pulse sequence is queued without
that named source correction.

## Raw cohort and reader boundary

378 samples retained:14 seed,119 warmup,181 observe,4 drain,60 teardown.
The sidecar has300 unique completed Decode events, all starts inside the fixed
window and all completions within the finite tail; no Panel events. Terminal
reports records300/losses0/pending0, available=true and producers_joined=true.
This confirms raw decode accounting shape; it does not make the input population
complete or establish a matched latency claim.

Reader `69fe3e5` passed Grok4.5 review and57 root cohort tests, but root withheld
integration after testing this native envelope. Actual observe-end qualification
`elapsed_s=314.0232895` differs from its retained cohort stamp `314.0232196`;
the terminal repeats the retained stamp exactly. Source captures these at separate
instants. The reader incorrectly compares them for near-exact equality and
returns `qualification_observe_end_elapsed_mismatch`. Task `t_2a8d467c` corrects
that ownership check with a native fixture and same-card review.

This simple diagnostic supervisor did not create canonical `metadata.json`.
`analyze_run` correctly reports missing_metadata. The direct structural reader
probe maps its metadata argument from recorded qualification settings, explicitly
without matched provenance or acceptance. A separate counterfactual copy changing
only the qualification elapsed value was used to isolate this parser error; it
is not native evidence. The original archive remains unchanged.

## Artifacts

Archive SHA256 `8a9e5a1a9fc446a980ab579faf04516dd2ad45e94737c05cff96e1e148c7dbd5`;
manifest SHA256 `f65fd49f66148f16758c1aaf6e8cbe45ddba8d0519fa9747d1bd860637a353bc`.
Root verified all35 manifest files. Archive: `diagnostics/windows-input-trace-cohort-a/archive.tar.gz`.
Extracted run: `diagnostics/windows-input-trace-cohort-a-archive/latency-diagnostic-input-trace-20260908-a/`.
Independent summary, captures and reader probes: `diagnostics/windows-input-trace-cohort-a/`.
The archive includes failed capture receipt, final native state, controller scripts,
all raw sidecars and the input helper receipt. Final native state records no frontends.
