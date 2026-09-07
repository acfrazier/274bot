# Standalone sleep excess diagnostic

The approved cadence audit found mean requested sleep 19.719ms and actual sleep 27.401ms in the N1 client loop. To check whether the game path is necessary to produce this delay, root compiled the standalone std-only `sleep_probe.rs` with `rustc -O`. It allocates its bounded sample vector before timing, uses 20 warmup calls, records Instant elapsed around `std::thread::sleep`, and prints only after the timed phase. It does not load client, host, cache, or game assets.

Evidence directory: `diagnostics/bare-sleep-20260907T065159Z/`. Preflight records source/binary SHA and rustc version. Source and executable copies are retained alongside the raw samples and summary. Each requested duration has 300 measured calls.

| Requested | Mean actual | Mean excess | p99 actual | Calls/wall second |
| --- | ---: | ---: | ---: | ---: |
| 19ms | 25.9741ms | 6.9741ms | 28.5538ms | 38.4997 |
| 20ms | 27.9066ms | 7.9066ms | 30.0368ms | 35.8336 |

Excess histogram upper bounds 1,2,5,10,20ms,+overflow: 19ms requests [10,14,47,229,0,0]; 20ms requests [9,8,33,130,120,0]. These are direct finite-sample durations, not inferred client intervals or a matched candidate result.

This is a mechanism diagnostic performed while the overhead implementer and unrelated OS/user activity were present. `quiet_campaign=false` is recorded. It is not a quiet performance cell and cannot quantify a production improvement. It shows that similar sleep excess can occur without executing the game path on this Mac under current conditions; it does not identify kernel timer policy, prove identical thread QoS, isolate background load effects, or predict Linux results. The unchanged game server continued running separately.

The next bounded experiment may compare the same 20ms nominal schedule with an advancing phase deadline in this standalone probe. No production scheduler policy has changed. A shipping change would require its own behavior and CPU/latency evidence; this report does not approve one.
