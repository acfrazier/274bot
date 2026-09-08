# Current N1 allocation capture: failed bound, usable global-peak output

The sole capture `current-owner-heaptrack-n1-2003` failed at the reviewed 2 GiB
raw-output guard. It does not qualify the full observation/Stop procedure and
establishes no performance acceptance or memory savings. Interpretation and
peak printing succeeded. Their output identifies a startup navigation-loading
peak, not a retained per-bot allocation population. No candidate is selected
from this global peak, and no replacement capture is released.

## Identity, release and execution

The design is `current-owner-allocation-capture-plan.md` at e8c3c80. Final
instrumentation 1fe21610b6446779133b5ff0dc229ba93599ecc0 passed actual Grok4.5 /
xai-oauth review, session `20260908_155322_0bb06f`, task t_314f3b20, run859.
The prior rejected iterations and their reproductions remain preserved.

The saved System/no-allocation-counting TUI binary remains host c0709ab,
client3456edc8, SHA a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9.
The 85 Python files were staged separately in Concord's
`/home/acfrazier/274bot-campaign/profile-tooling-stage-1915`, using archive SHA
807158864234f71813133eda756d2ac4dbca26715c0e20776211ccbd0a86a684.
All 1,113 original frozen source files were reverified unchanged. No Rust rebuild
or runtime feature change occurred. Native affected Python tests passed: 101
run, two platform skips. The owned-process smoke `heaptrack-seam-smoke-2001`
passed direct-child ownership, child-only limits/environment, raw permissions,
interpretation and known allocation-family checks. Its common-time peak was
5,730,024 bytes, including the known 4,196,352-byte bytearray family.

An earlier `2002` preflight failed before launch with attempts0 because root's
launch script used an incorrect server-sidecar path. The corrected script uses
`profile-tooling-stage-1915-server-identity.json`; the failed preflight is retained.
This was not a second game capture.

The one frontend was PID2096/start603986, under launcher2083 and controller2066;
server726/start595 remained on the previously verified Concord boot. The child
used the real ELF and PTY with Heaptrack1.5.0 direct preload. Profile toggles and
allocation counting remained off. Native metadata brackets frontend execution
by 1788897753.9746997 and 1788898344.6389525 Unix seconds (590.664 seconds).
The printer reports 589.61 seconds on its own trace clock; these are different
clock boundaries, not interchangeable timestamps.

## Failure and qualification

The raw-size guard recorded `2149010072 >= 2147483648` bytes. Frontend exit was
-15 (SIGTERM); launcher and root controller returned1. The guard's sampled peak
frontend RSS was195,039,232 bytes, below512MiB. This is a monitored RSS value
including profiling overhead, not a separately measured overhead amount.
The controller's MemAvailable guard did not trigger.

Only `observe-start` appears in `samples.qualification.jsonl`. At that boundary,
the slot was Running, ingame, scene_state2, and its Thiever paint reported26
steals. The wall bracket ends at1788897889.114621; approximately455.524 seconds
remain until the frontend wait returned. That does not complete the requested
600-second observation. There is no observe-end, script-Stop, after-Stop or join
boundary. The existing qualifier returned1 / qualified=false with:

- missing boundary qualification;
- observation incomplete;
- process failed or incomplete.

The managed receipt also reports missing observation boundary,
frontend_failed_or_incomplete, and launcher_failed. All owned controller,
frontend, interpreter and printer processes were absent after completion;
collector cleanup returned0 and the receipt records no frontend orphan risk.

## Successful bounded analysis of the incomplete trace

The interpreter exited0 in58.556s, sampled RSS114,855,936 bytes. The printer
exited0 in13.515s, sampled RSS58,503,168 bytes. Both stayed within the reviewed
180-second/RSS512MiB/AS768MiB bounds. These are analysis processes, outside the
frontend allocation population. The raw file is2,149,010,072 bytes and the
interpreted file810,563,095 bytes; both remain on Concord, with hashes recorded
in the companion result and evidence manifest.

The exact `--merge-backtraces=0 --flamegraph-cost-type=peak` output has14,903
rows,194 positive rows, no negative or malformed costs, and a positive-cost sum
of160,842,811 bytes. This agrees with the printer's rounded160.84M global peak.
This is one common-time peak, not a sum of independent stack peaks.

A disjoint source-subtree partition of those rows is:

| Captured allocation subtree | Requested bytes at the global peak |
| --- | ---: |
| NavWorld::load_pack | 147,272,041 |
| IfType::unpack | 6,491,160 |
| Cache::unpack | 4,783,967 |
| host_play::load_templates | 1,933,184 |
| Other | 362,459 |
| Total | 160,842,811 |

This coarse partition is reproduced by exact frozen symbol markers in
`diagnostics/owner-capture-evidence-2015/recompute_global_peak.py`. It is not
advertised as an allocation-line or private-owner classification. Raw mangled
stacks and row numbers remain in the companion JSON and peak text.

The navigation subtree accounts for91.563% of this population. Its largest
row is73,438,581 bytes from std::fs::read under NavWorld::load_pack, equal to the
verified navpack's file size. Frozen `crates/nav/src/world.rs:49-55` reads that
file into a temporary byte vector and decodes by borrow. The other two large
rows are65,142,784 and8,142,848 bytes under nav::pack::decode. Reading the frozen
pack's first25bytes gives width1792/height9088: four planes require65,142,784
u8 walk cells and8,142,848 bytes of u64 blocked words. Those sizes match frozen
`crates/nav/src/pack.rs:338-350`; the blocked allocation's exact source line is
inferred from size and calling context, not proven inline debug information.

The original file buffer coexists with decoded arrays at this peak and drops
when load_pack returns. Frozen `crates/host-play/src/lib.rs:3181` loads the world
once into an Arc in Play::new. Thus this peak exposes startup overlap and shared
navigation storage. It cannot rank retained Client/World/snapshot/script owners
per bot, nor justify replacing that missing answer with a startup optimization.

## Evidence and next decision

Local evidence: `diagnostics/owner-capture-evidence-2015/`. Its export archive
SHA is1a73be129f22b2e78a2dc220b8305709aff83e4fe4c046050bd924e95412a9a1;
46 included files were hash-verified. The two large trace files are explicitly
marked remote-only in `owner-capture-evidence-2015-manifest.json`. Root recompute,
qualifier stdout/exit, native smoke, tool manifest/test logs, launch metadata,
process-accounting receipts, stop reason and both analysis outputs are retained.
The companion `current-owner-allocation-capture-result.json` records exact
values, original stack rows and trace hashes. Independent evidence review is
required before accepting these conclusions.

Task t_227f43d3 is a separate source/format capability audit for an offline
replay of the already saved data. It must prove any timestamp/live-allocation
semantics on primary Heaptrack1.5.0 source and saved controlled fixtures before
proposing production replay. No such parser, phase slice, private-owner ranking,
end-of-observation state, snapshot epoch, mmap/V8/GPU accounting, RSS causality,
overhead result or saving is established here. No new capture or optimization
is released by this report. The broader campaign's matched comparisons,
lifecycle/scaling/rendering/latency gates and final Grok4.6 review remain open.
