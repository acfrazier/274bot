# T1.4 local WSS cutover soak

This is a **local candidate gate**, not a claim that the two-hour run has already passed. The operative accepted design is `TRACK-T1/T1-GATE/r7e/T1-GATE.md` together with binding `r7e/MAIN-ACCEPTANCE.md`; Main's selected values are recorded in `TRACK-T1/T1.4/soak-values.md`. The committed [`soak-config.json`](soak-config.json) is the only qualifying source of values; its `_sources` maps each value to the decision. Do not tune a failed threshold or remove a fault to make a pass.

## Build identity and comparison

The route diagnostic comparator is a disposable local branch rooted at `41f14629b92060dd5954e1b80ca963e76e237a76`.
Apply the byte-identical shared `crates/host-play/tests/t14_soak_live.rs` and its Cargo test declaration.
Any necessary native event-observation seam must also be byte-identical and must not change product behaviour.
Record that shared fixture diff in the report.
Do not merge or push this comparator.
Record both commits and shared-source SHA-256 values before interpreting a pair.
Both arms use the same release profile, features `live-harness,test-support`, workload, proxy, seed, asset cache, navigation bytes, and process sampler.
The candidate also enables its separate `soak-observers` module.

Each pair must record its own source digests. Earlier smoke and qualification attempts used older fixtures; their digests do not identify the current route-safe workload. The baseline trusts the private CA through `SSL_CERT_FILE` and `SSL_CERT_DIR`; the candidate installs that CA in its test TLS configuration before startup. Neither arm changes the OS trust store or Engine binaries.

The runner creates a new private output directory per run. It generates a private CA and leaf for `127.0.0.1`, copies Engine A's immutable client pack into a per-run HOME, and writes the WSS server roster there with the read-only Engine `world.json` node ID. The common path is **client → A (ciphertext segmentation/latency/stall) → TLS terminator → B (plaintext WebSocket re-framing, fragmentation and ping) → Engine A**. Proxy B consumes the unique matching Pong for each Ping it injects, forwarding other Pongs unchanged. HTTPS assets share the A/TLS path but bypass WebSocket re-framing; only `/client/client.js` is served as an explicit local test fixture using the **public** RSA modulus extracted from Engine A's read-only `private.pem`, because its bundled JS lacks that modulus. The effective config and proxy receipt commit the source and served-body hashes. No game payload or password bytes are logged: only counts, transport fingerprints, per-message/cumulative SHA-256, compartment samples and native scene/bank status. A proxy peer port correlates its connection with the candidate observer binding; source-port collisions are a failure, not a reason to guess identity.
The runner binds the **production runtime WSS profile** rather than a static metadata-free remote profile: compiled Gatherer needs audited selected game data. Both builds set `BOT_NAV_BUILD=require` and use the same complete offline `BOT_NAV_SNAPSHOT_ROOT` (default: the operator's `~/.274bot/unpack-289`, resolved **before** HOME isolation); the build checks the Engine A cache/content provenance and publishes a trusted navigation identity row. The manifest records the selected snapshot root and build mode. The WSS runtime profile uses that build's attested staged 289 pack and flags; `nav-identity.json` requires their SHA-256 bytes to equal the fixed external pack/flags in `soak-config.json` and the other cohort's selected bytes. This changes only source provenance, not navigation content. The actual bot process still uses a fresh per-run HOME and the copied immutable client cache. A missing source, mismatched cache identity, or withheld game data fails before the workload; no synthetic game facts are substituted.

The separate `rss_ladder` memory fixture runs as a `harness = false` executable. It initializes protected V8 on process main before Play creates workers, as the product does. Stable libtest's worker entry cannot satisfy this contract in release builds. Both comparator and candidate must use this entry, the same generated navigation inputs, and the same workload. Without `LIVE=1`, the executable checks its argument parser but does not run a memory measurement.

The paired measurement fixtures require `soak-observers` for candidate runs. That feature enables explicit `PlayComposition` startup before cache preparation and installs isolated TLS trust. The native-TLS comparator has no explicit composition API: its existing cache and client initialization create their network resources before the observation window. Keep the shared fixture source byte-identical between both arms. Do not interpret a candidate run without `soak-observers` as qualification evidence.

Live tests must start their process network before profile binding or template preparation. Shared selection helpers own this startup when more than one test calls them. `support/network_composition.rs` retains the process composition in a `LazyLock`. The survivable-admission, toll-gate and web-cut selectors use that helper. The quest-door fixture starts it before profile binding, not before the later client initialization.

Direct-client live fixtures retain one `host_play::DirectClientLogin` per client across login and relog. This context uses the production queue, preparation and attempt lifecycle, and returns `LoginError` without reducing it to a boolean. The interaction API no longer exposes login. The survivable-admission fixture retries only the engine's already-logged-in response (code 5) after its setup logout: its budget is 26 engine cycles of 600 ms, covering the engine's 15-second logout-flush retry plus one cycle. Other typed errors fail immediately. This fixture retry does not change production login policy.

The live map covers the root interaction module with `host.api-gather-wire-shadow` and the bank test-support harness with `host.bank-fetch-session`; both keep their existing three-run requirement.


Before running, start the fixed Engine A service listening on `127.0.0.1:1080`, with the 289 navigation pack referenced in the config available. Set a nonnegative common `SOAK_SEED` once and an evidence root outside either repository. Run the infrastructure smoke **for both builds**:

```sh
python3 tools/t14_soak_run.py --mode baseline \
  --repo /srv/dev/scratch/274bot-worktrees/t1.4-soak-clean \
  --seed "$SOAK_SEED" --smoke --output "$SOAK_EVIDENCE_DIR/smoke-dev"
python3 tools/t14_soak_run.py --mode candidate \
  --repo /srv/dev/scratch/274bot-worktrees/t1.4-transport \
  --seed "$SOAK_SEED" --smoke --output "$SOAK_EVIDENCE_DIR/smoke-candidate"
```

A **diagnostic-only** baseline run may append `--diagnostic-no-ws-faults` with `--smoke` to retain TCP segmentation/latency while disabling only proxy B fragmentation/Ping. This mode cannot run qualification or satisfy the baseline/candidate fault comparison; use fresh absent evidence directories and do not alter `soak-config.json`.

Smoke runs use two accounts for 180 seconds and only the **first** approved latency phase; they never qualify the gate. Main runs the full candidate and baseline two-hour qualification with the same two commands **without** `--smoke`, using fresh absent directories. All accounts complete their stock Gatherer setup through the same proxies **without injected faults**. Each scenario's `StepKind::Relog` binds the inventory tab: its connection exit is allowed only before t0, after the runner logged its logout packet at that exact step for that slot, and must be followed by a connected return; any other setup exit fails. A cohort barrier holds the script Starts until every slot completes this setup. The fault clock t0 is the **first successful script Start after the barrier**, recorded by the Play runner in an atomic `proxy/soak-start.json` marker and confirmed by the proxy's `fault_phase_zero` receipt; the first WebSocket 101 during setup is not t0. The exact 7,200-second qualifying window includes warmup and all four 1,800-second proxy phases. The cohorts log in through Play's normal queue, each reaches scene 2, and each exercises a real Gatherer bank loop for its configured tree tier. Warmup ends at the later of 900 seconds after t0 and the last slot's scene-2 arrival plus one real bank cycle per slot. No session exit during the window is requested, including warmup. RSS slopes exclude warmup, and the summary reports each slope-window duration. No seeded test frame, synthetic bank counter or idle twenty-slot login is evidence of that workload.

**Woodcutting fixture correction, after the nonqualifying candidate smoke:** `smoke-candidate-289144-m/events.jsonl:33-39` records the Draynor oak slot at `(3115,3242,0)`, followed by one death, respawn at `(3222,3218,0)`, and an unexpected script stop at t0+101 s. Engine `content/maps/m48_50.jm2:6688-6694` places jail guard NPC 917 at `(3120,3238,0)` (map origin `(3072,3200)`); `content/pack/npc.pack:918` identifies it, and `content/scripts/areas/area_draynor/configs/draynor.npc:152-182` sets `huntrange=5` and `huntmode=aggressive_melee`. The observed tile was within its range. No transport payload divergence or unexpected connection exit was observed. Main approved replacing this unsafe workload site, not changing a fault value or ignoring the death.

The earlier Catherby fixtures also failed real resource routes.
`smoke-baseline-289144-r/summary.json:95-105` records an unreachable western oak approach.
`qual-1027/baseline-soak-qualification-attempt-12/events.jsonl:457` records the later normal-tree route outside the audited area.
These failures do not justify a transport threshold change.

## Workload route check

The fixture uses normal logs from `woodcutting.falador_west.n` and oak logs
from the distinct `woodcutting.dwarven_mine` site. Both use Falador West bank.
The five normal Starts are `(2946,3399)`, `(2955,3399)`, `(2952,3417)`,
`(2955,3407)`, and `(2958,3415)`, all inside the authored `.n` region
`x=2946..2958, z=3399..3417`. Their deterministic revision-289 selection
and native bank-route witnesses are in
`qualify-1102/start-selection-v2/selection.json`. The oak Start is
`(3007,3411,0)` with radius 12 and three reachable oaks in the Dwarven Mine
site (`content/maps/m46_53.jm2:5371`, `m47_53.jm2:4278,4458`).
The outer bound remains `x=2840..3065, z=3262..3527`.
The selected workload audit must pass before the new diagnostics.
The two earlier diagnostics did not complete thirty minutes and do not satisfy this requirement.

The route-safety proof covers workload movement only:

- worksite to bank and bank to worksite
- Gatherer retreat and re-approach
- ordinary replan and Traveller recovery during the fault phases

Random-event movement is outside this proof.
This exclusion includes Guardian flee routes and their later ordinary replans, with or without stairs.
The earlier Guardian-derived staircase counterexample is not a workload-route failure.
The alternate oak site is selected from the same revision-289 content.

Five accounts gather normal logs at the authored `.n` site. Each radius-five
Start has at least two reachable in-site trees; the five Starts admit a
distinct-tree capacity matching. The selection ranks least resource overlap,
nearest prior Start, widest spacing, then coordinates. Actual Gatherers choose
their trees, so shared selection and contention remain possible.

Fifteen accounts gather oaks at Dwarven Mine within radius twelve of
`(3007,3411,0)`; the selected patch has three accessible oaks. Falador West
bank routes and oak interaction faces are evidenced in
`qualify-1102/site-probe-dwarven_mine/result.json` and
`site-access-dwarven_mine/result.json`. The oak square has a static
danger-envelope margin of 27 tiles. The nearer Taverley SE and Druids Circle 2
sites had bank-reachable Starts but no routes to their owned oaks; the
Falador West 2 oak patch failed its bank route. These failed candidates do
not change the route oracle. Each account gets one Lobster in inventory and
one in its own bank. Gatherer keeps one food ready and eats below half base
HP. The fixture records HP, hitmarks, observed damage targets, death chat,
and death events before checking route boundaries. It does not infer a
killer from a target relation or a respawn tile.

`NORMAL_SAFE_WALK` defines both resource-census rectangles.
It is not a movement stop condition.
The guard records a `worksite-exit` event when a bot leaves these rectangles.
That event does not fail the arm.
For workload movement, danger-envelope entry or movement beyond `OUTER_WALK_BOUNDS` fails the route guard.

The checker uses each hunter's `wanderrange + huntrange + size`.
It checks all selected-content hunter placements, independently of combat-level immunity and collision shielding.
A random event does not make a workload route pass this static check.
Instead, the generator excludes random-event routes from the workload proof.
The proof does not claim a complete bound on all possible product movement.

During a soak, the boundary oracle distinguishes workload departures from excursions started by a real owned random event.
It records the random-event start before attributing a same-frame departure.
The event-started excursion and its events are observations, not stop conditions.
Its attribution continues through the later return or replan until safe re-entry.
An unrelated workload departure after re-entry still fails.
Readiness holds, foreign events, or missing event evidence must not grant an exemption.
The two-way fixture test must show both outcomes.
Death, stall, missed-work, and transport thresholds remain unchanged.
Do not change product random-event policy or grant immunity.

The custom WSS roster has no membership declaration.
Its Remote profile resolves Unknown membership as false.
Guarded local `world.json` membership does not apply to that WSS profile.
Both the native client's membership field and the native router's member mode must match this false value.

An adjacent ordinary gate or door may pass the workload route check.
The edge must have typed Door identity, no requirements, one-tile adjacency, and no level change.
Every approach, crossing endpoint, and exit walk must stay clear.
Each accepted crossing requires the corresponding content placement and dispatch evidence.
The region does not need a gate crossing to qualify.
Stairs, ladders, teleports, agility shortcuts, and nonadjacent transports fail closed for workload routes.

Run the following commands in a worker driver through `aubaine.sh`.
Do not build or query routes on merlot.

```sh
BOT_NAV_BUILD=skip cargo build --release --locked -p host-play --example soak_routes
python3 -B -m unittest discover -s tools -p test_t14_soak_routes.py -v
"$HOME/experiments/274bot/.superpowers/bin/isohome" python3 -B tools/t14_soak_routes.py \
  --router target/release/examples/soak_routes \
  --pack "$NAV_PACK" --flags "$NAV_FLAGS" \
  --cache "$ENGINE_DIR/data/pack/client" --content "$ENGINE_DIR/../content" \
  --output "$SOAK_EVIDENCE_DIR/route-audit"
```

The checker enumerates all authored resources in each normal patch.
It rejects either tier's patch when it has fewer than two eligible trees.
It includes every second-tier resource within the retained Start radius.
It checks every footprint, Reach alternative, return observation, Start, and bank access goal.
It calls the production hazard-retreat selector for every resource access stand.
It calls the actual revision-289 client interaction BFS with cached loc geometry and raw collision flags.
It covers every resource and bank access origin at every qualifying scene base.
It also calls the actual client BFS for every short Traveller aim in the closed workload-origin set.
These aims include both directions, click-ahead, scene clipping, and stalled-walk recovery.
It queries every alternative goal from all starts, access stands, and retreat destinations.
It repeats those queries from every intermediate workload-route tile until the replan-origin set is closed.
This includes blocked-origin escape after a resource changes.
Random-event endpoints do not seed that closure.
The checker examines complete workload walk legs and permits only the ordinary crossings described above.
It rejects workload routes inside any hunter envelope or outside the outer bound.
The result retains source, content-map, catalogue, v17 pack, raw-flag, and client-config hashes.
Native route rows use lossless gzip compression in `*-routes.jsonl.gz`.

An unroutable client alternative returns before movement encoding.
The checker counts this outcome as nonmovement for loc approaches and ordinary walk aims.
Every selected resource and bank loc must still have an actual accepted client approach witness.
Other client errors fail the check.

After a consumer-only correction, add `--replay "$SOAK_EVIDENCE_DIR/route-audit"` to the same checker command.
Use a fresh `--output` directory.
Replay requires identical selected input hashes and identical generated request batches.
It reads every retained native output row again without executing the unchanged native producer.
It does not convert an earlier verdict into a pass without checking the rows.

The old Catherby fixture bounded resource origins, not every approach or bank-return tile.
Baseline attempt 12 reached `(2813,3477,0)`, inside the wolf's retained envelope.
Candidate attempt 1 reached `(2811,3473,0)`, outside its then-approved rectangle.
Actual client/v17 Swarm flee paths can reach both old observed tiles.
This establishes a possible movement source, not either historical event's identity.
The old logs do not identify the exact cause or NPC.
Keep that attribution limit in the report.
Evidence and route analysis are in `TRACK-T1/T1.4/soak-route-1028/report.md`.

## Nonqualifying diagnostics

Main authorized two new thirty-minute Engine L diagnostics after the chance-outcome ruling.
Run one baseline and one candidate arm after the workload audit and two-way oracle tests pass.
Retain all twenty accounts, the same fault configuration, and an owned three-character account prefix.
Use fresh absent evidence directories for both arms.
The diagnostic duration starts at the first script Start after the setup barrier:

```sh
python3 -B tools/t14_soak_run.py --mode baseline --repo "$BASELINE_REPO" \
  --seed 1027 --diagnostic-seconds 1800 --name-prefix srt \
  --engine-dir "$ENGINE_L_DIR" --upstream-port 3080 \
  --output "$SOAK_EVIDENCE_DIR/diagnostic-baseline-chance"
python3 -B tools/t14_soak_run.py --mode candidate --repo "$CANDIDATE_REPO" \
  --seed 1027 --diagnostic-seconds 1800 --name-prefix srt \
  --engine-dir "$ENGINE_L_DIR" --upstream-port 3080 \
  --output "$SOAK_EVIDENCE_DIR/diagnostic-candidate-chance"
```

These runs cannot qualify the gate.
The committed qualification remains twenty accounts for 7200 seconds.
Its workload, account seed, fault phases, warmup, progress budgets, and limits remain unchanged.
The report must provide exact Engine A commands without diagnostic or engine-override arguments.
Only Main can authorize those qualifying runs.

Each run preserves `effective-config.json`, `run-manifest.json`, `nav-identity.json`, `cohort.log`, `cohort-exit.json`, `pid.json`, external `process-samples.jsonl` (10-second `/proc` RSS, monotonic clock, task count and every task's `comm`), the Play test's per-slot events/summary, `proxy/ready.json`, `proxy/soak-start.json`, `proxy/connections.jsonl`, and `proxy.log`. Candidate additionally records `candidate-five-compartment-ledger.jsonl` at every ten-second sample and `candidate-wss-payloads.jsonl` SHA snapshots each minute and at shutdown. The ledger's **250,760 bytes are only five named logical reservations**: 16 KiB received plaintext, 128 KiB pending TLS output, 64 KiB handshake deframer, 5000-byte outbound ring and 32 KiB inbound ring. Known actual ring occupancy and accessible TLS bytes are separate fields; unavailable private rustls/tungstenite occupancy is null, never fabricated as zero. This is **not** an actual whole-connection allocation measurement. Compare whole-process RSS slope/final against baseline, and perform the separate G5-E deterministic scoped counting-allocator test for requested-layout bytes, high-water and unexplained deltas. Allocator requested bytes are not allocator usable bytes.

The dedicated Linux G5-E measurement is `TRACK-T1/T1.4/g5e-linux-wss-one.json` (one local rustls/Tungstenite WSS client, explicit fixture-setup, handshake/upgrade, idle, fragmented read/write, and shutdown phases). The connected phase recorded 416 scoped allocation events, a driver-thread requested-byte net of +17,649, a fixture-thread net of -26,093, and a **process** live-requested-byte delta of -8,444 from the no-WSS fixture baseline; the process peak during connection was +16,634. The fragmented read/write phase peaked +65,292 from that baseline. A concurrent fixture worker can release its setup allocations during connection, so a positive process-net assertion would be an invalid per-WSS oracle. The static 1,572,864-byte event recorder and unknown native/OS allocation are not part of these requested-layout numbers. This receipt measures rustls/Tungstenite only, **not** the entire client-adapter endpoint, native usable capacity, or the required 20-connection RSS allowance. Keep those categories explicitly unqualified until the full platform-specific G5-E ledger and candidate/baseline soak are evaluated.

The authenticated-control fixture assigns a unique connection ID to each WSS connection. Its socket registration, readiness filters and backpressure assertions use that ID as the mio token, as the product endpoint does. A fixed token would discard readiness for the second connection and invalidate the sibling-progress and shared-memory measurements.

The raw TLS peer retains the initial client traffic secret only in memory. After it authenticates a TLS 1.3 KeyUpdate, it derives the next receive key and resets the record sequence. The G5-E recovery check permits authenticated queued application records after the reply, but rejects another handshake reply or a decryption error. A fixture regression checks the updated-key decoder's sequence, tag and inner-padding handling. The direct `ring` dependency is test-only; release products do not gain a dependency edge.

The proxy keeps raw terminal socket/TLS errors in each connection receipt; the comparator does **not** discard them. A post-response HTTPS TLS BrokenPipe is an accounted close only when headers/body and both ciphertext/plaintext transfer byte counts agree. A WSS `SSLEOFError` or proxy-A BrokenPipe is an accounted close only when both zero-payload WebSocket Close frames were forwarded, both game-binary cumulative hashes agree, client-to-engine bytes all match, and any unmatched TLS1.3 server-to-client tail is at most the two-byte WS Close or 24-byte encrypted final record. Every other error, larger deficit, missing Close or game hash mismatch fails. A Play actor's in-window exit fails separately; this teardown accounting is never permission to reconnect during the soak.

After a completed workload, the shared fixture requests ordinary logout and
waits for the final receive and bidirectional empty WebSocket Close receipts.
Only then does it finalize candidate payload observations. An incomplete
workload or missing Close receipt remains a failure; this does not relax the
unchanged transport evaluator.

Reject missing/corrupt/short receipts. The qualifying comparator must enforce zero unexpected exits, zero proxy B read/write binary-payload hash divergences, zero candidate/client-vs-proxy payload hash divergences, every active WSS connection's five-compartment reservation ≤250,760 bytes at each sample and no known occupancy overrun, candidate RSS slope ≤ baseline slope +1 MiB/hour after warmup, and candidate final RSS ≤ baseline final RSS + 20×250,760 bytes +5% of baseline final. The census uses §21 §1 plus F0 ADR 0007 amendment 7: total Linux tasks are `T = N + J + V + F + H + A + B + G + E + E_v + L_T + R`, with `N = 20`, named singletons `F ≤ 14` and the newly explicit request threads `R ≤ 64` (`nav-find-`, `nav-inspect-`, `bank-pick-`, `card-prepare`); each other category and the test-harness main task are classified by `comm`. Raw task count is **not** capped at 14. Any unclassified task fails. Check all four latency phases, both traffic directions, the required segment and frame-fault counts, all twenty real per-slot bank cycles and uninterrupted two-hour residency. A failed, unobserved or ambiguous oracle blocks the gate; the smoke may only check infrastructure and must be labeled nonqualifying.
