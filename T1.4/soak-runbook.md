# T1.4 local WSS cutover soak

This is a **local candidate gate**, not a claim that the two-hour run has already passed. The operative accepted design is `TRACK-T1/T1-GATE/r7e/T1-GATE.md` together with binding `r7e/MAIN-ACCEPTANCE.md`; Main's selected values are recorded in `TRACK-T1/T1.4/soak-values.md`. The committed [`soak-config.json`](soak-config.json) is the only qualifying source of values; its `_sources` maps each value to the decision. Do not tune a failed threshold or remove a fault to make a pass.

## Build identity and comparison

The route diagnostic comparator is a disposable local branch rooted at `41f14629b92060dd5954e1b80ca963e76e237a76`. Apply only the byte-identical shared `crates/host-play/tests/t14_soak_live.rs` and its Cargo test declaration. Do not merge or push this comparator. Record both commits and shared-source SHA-256 values before interpreting a pair. Both arms use the same release profile, features `live-harness,test-support`, committed workload, proxy, seed, asset cache, navigation bytes, and process sampler. The candidate also enables its separate `soak-observers` module.

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

Smoke runs use two accounts for 180 seconds and only the **first** approved latency phase; they never qualify the gate. Main runs the full candidate and baseline two-hour qualification with the same two commands **without** `--smoke`, using fresh absent directories. All accounts complete their stock Gatherer setup through the same proxies **without injected faults**. Each scenario's `StepKind::Relog` binds the inventory tab: its connection exit is allowed only before t0, after the runner logged its logout packet at that exact step for that slot, and must be followed by a connected return; any other setup exit fails. A cohort barrier holds the script Starts until every slot completes this setup. The fault clock t0 is the **first successful script Start after the barrier**, recorded by the Play runner in an atomic `proxy/soak-start.json` marker and confirmed by the proxy's `fault_phase_zero` receipt; the first WebSocket 101 during setup is not t0. The exact 7,200-second qualifying window includes warmup and all four 1,800-second proxy phases. The cohorts log in through Play's normal queue, each reaches scene 2, and each exercises a real Gatherer bank loop across at least two named content sites. Warmup ends at the later of 900 seconds after t0 and the last slot's scene-2 arrival plus one real bank cycle per slot. No session exit during the window is requested, including warmup. RSS slopes exclude warmup, and the summary reports each slope-window duration. No seeded test frame, synthetic bank counter or idle twenty-slot login is evidence of that workload.

**Woodcutting fixture correction, after the nonqualifying candidate smoke:** `smoke-candidate-289144-m/events.jsonl:33-39` records the Draynor oak slot at `(3115,3242,0)`, followed by one death, respawn at `(3222,3218,0)`, and an unexpected script stop at t0+101 s. Engine `content/maps/m48_50.jm2:6688-6694` places jail guard NPC 917 at `(3120,3238,0)` (map origin `(3072,3200)`); `content/pack/npc.pack:918` identifies it, and `content/scripts/areas/area_draynor/configs/draynor.npc:152-182` sets `huntrange=5` and `huntmode=aggressive_melee`. The observed tile was within its range. No transport payload divergence or unexpected connection exit was observed. Main approved replacing this unsafe workload site, not changing a fault value or ignoring the death.

The earlier Catherby fixtures also failed real resource routes.
`smoke-baseline-289144-r/summary.json:95-105` records an unreachable western oak approach.
`qual-1027/baseline-soak-qualification-attempt-12/events.jsonl:457` records the later normal-tree route outside the audited area.
These failures do not justify a transport threshold change.

## Workload route check

The proposed fixture is not approved for qualification.
The baseline diagnostic stopped at `(2678,3471,0)` under the old rectangle oracle.
The candidate diagnostic stopped on native `resource-unavailable` after approximately 376 phase-zero seconds.
Neither diagnostic satisfies the thirty-minute requirement.
Qualification requires the corrected scoped audit and the remaining authorized diagnostic.

Ten accounts gather normal logs from the configured real content site.
Each account retains one radius-five Start anchor from `NORMAL_STARTS`.
The checker requires at least two eligible normal trees for each account.
The source selection ranks patches by their minimum access-tile clearance.
Selection includes the radius-one NPC spawn offset, so the initial flee margin is twenty tiles.
Actual Gatherers choose their own trees, so shared selection and contention remain possible.
The fixture does not change the product's bounded-wait policy.
The other ten accounts gather `SECOND_TIER_KEY` within radius twelve of `SECOND_TIER_START`.
Both groups use Start selection and the configured bank.
The cohorts may share a real content site when it supports both resource keys.
Each account receives Woodcutting 99, one rune axe, and 64 banked spare rune axes.

`NORMAL_SAFE_WALK` defines both resource-census rectangles.
It is not a movement stop condition.
The guard rejects every authored danger envelope and movement outside `OUTER_WALK_BOUNDS`.
The selected region has its own outer bound in the shared harness.
The product-classified Maze and Mime interiors retain their separate travel rules.
The guard records a `worksite-exit` event when a bot leaves these rectangles.
That event does not fail the arm.
Only danger-envelope entry or movement beyond the outer bound fails the physical-route guard.

The checker uses each hunter's `wanderrange + huntrange + size`.
It checks all selected-content hunter placements, independently of combat-level immunity and collision shielding.
It does not exempt a route because a random event caused it.
Main selected this oracle from source evidence before diagnostic launch 3.
Real Guardian/client queries rejected Seers maple access and Seers bank.
A Swarm flee from `(2724,3493)` enters a highwayman envelope at `(2738,3507)`.
Hemenster also failed ordinary replans from accepted Guardian endpoints.
Main requires a global comparison of bank regions before the final diagnostic.
The comparison includes actual Guardian routes and their ordinary bank/resource replans.
It may replace maples with willow or oak logs when the safer region lacks usable maples.
The report records the selected region and the rejected alternatives.
Miscellania's 140-tile maple placement is not usable for this workload.
Its content grants kingdom approval instead of logs.
See `woodcut.rs2:69-76,140-148` and `lumberjack_leif.rs2:29-40`.

The conservative bound is not met.
The checked initial-flee and walk routes do not prove the worst-case scene-box flee.
The native client searches a 104-by-104 scene, whose Chebyshev diameter is 103 tiles.
Continued Guardian radius growth also remains outside this finite audit.
Main accepts this residual risk before launch 3, with the same rule for both arms.

An adjacent ordinary gate or door may pass the physical-route check.
The edge must have typed Door identity, no requirements, one-tile adjacency, and no level change.
Every approach, crossing endpoint, and exit walk must stay clear.
Each accepted crossing requires the corresponding content placement and dispatch evidence.
The report identifies those locs when the chosen region uses gates or doors.
The region does not need a gate crossing to qualify.
Stairs, ladders, teleports, agility shortcuts, and nonadjacent transports still fail closed.

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
It rejects a normal patch with fewer than two eligible trees.
It includes every second-tier resource within the retained Start radius.
It checks every footprint, Reach alternative, return observation, Start, and bank access goal.
It calls the production hazard-retreat selector for every resource access stand.
It calls the actual revision-289 client interaction BFS with cached loc geometry and raw collision flags.
It covers every resource and bank access origin at every qualifying scene base.
It also calls the actual client BFS for every short Traveller aim in the closed origin set.
These aims include both directions, click-ahead, scene clipping, and stalled-walk recovery.
It queries every alternative goal from all starts, access stands, and retreat destinations.
It repeats those queries from every intermediate route tile until the replan-origin set is closed.
This includes blocked-origin escape after a resource changes.
The real Guardian and Client also check all selected-content initial-flee range groups.
Each worksite and bank tile receives all adjacent NPC spawn directions and the same-tile fallback.
The Guardian chooses its real ordered exact goals, nearest fallback, and nearer-ring retries.
The probe stops after the first accepted flee.
It checks exact return walks from every intermediate accepted flee tile.
It also checks every goal in the Guardian's four-tile hazard ring through the real client BFS.
Every accepted Guardian tile enters the ordinary replan closure.
The result sets `complete_movement_coverage` and `guardian_continuation_covered` to false.
A scoped pass does not establish a complete movement bound.
The checker examines complete walk legs and permits only the ordinary crossings described above.
It rejects routes inside any hunter envelope or outside the wide outer bound.
The result retains source, content-map, catalogue, v17 pack, raw-flag and client-config hashes.
Native route rows use lossless gzip compression in `*-routes.jsonl.gz`.

An unroutable client alternative returns before movement encoding.
The checker counts this outcome as nonmovement for loc approaches, ordinary walk aims, and Guardian walks.
Every selected resource and bank loc must still have an actual accepted client approach witness.
Other client errors fail the check.

After a consumer-only correction, add `--replay "$SOAK_EVIDENCE_DIR/route-audit"` to the same checker command.
Use a fresh `--output` directory.
Replay requires identical selected input hashes and identical generated request batches.
It reads every retained native output row again without executing the unchanged native producer.
It does not convert an earlier verdict into a pass without checking the rows.

The old Catherby fixture did not constrain route geometry.
The Gatherer bounds resource origins, not every approach or bank-return tile.
Baseline attempt 12 reached `(2813,3477,0)`, inside the wolf's retained envelope.
Candidate attempt 1 reached `(2811,3473,0)`, outside its then-approved rectangle.
Actual client/v17 Swarm flee paths can reach both old observed tiles.
This proves a possible movement source. It does not identify either historical NPC.
Guardian walks omit their cause and NPC from logs. Main will file that separate product finding.
Do not change product logging or exempt danger-envelope entry.
Evidence and the route analysis are in `TRACK-T1/T1.4/soak-route-1028/report.md`.

## Nonqualifying diagnostics

Engine L diagnostics retain all twenty accounts and all committed fault phases.
Two diagnostic launches already failed, so only launch 3 remains authorized.
Run the corrected candidate arm once after the scoped audit and two-way tests pass.
Do not repeat either earlier arm or treat this one arm as paired qualification evidence.
Use an owned three-character account prefix.
The diagnostic duration starts at the first script Start after the setup barrier:

```sh
python3 -B tools/t14_soak_run.py --mode candidate --repo "$CANDIDATE_REPO" \
  --seed 1027 --diagnostic-seconds 1800 --name-prefix srt \
  --engine-dir "$ENGINE_L_DIR" --upstream-port 3080 \
  --output "$SOAK_EVIDENCE_DIR/diagnostic-candidate-corrected"
```

These runs cannot qualify the gate.
The committed qualification remains twenty accounts for 7200 seconds.
Its workload, account seed, fault phases, warmup, progress budgets, and limits remain unchanged.
Run the next Engine A arms without diagnostic or engine-override arguments.

Each run preserves `effective-config.json`, `run-manifest.json`, `nav-identity.json`, `cohort.log`, `cohort-exit.json`, `pid.json`, external `process-samples.jsonl` (10-second `/proc` RSS, monotonic clock, task count and every task's `comm`), the Play test's per-slot events/summary, `proxy/ready.json`, `proxy/soak-start.json`, `proxy/connections.jsonl`, and `proxy.log`. Candidate additionally records `candidate-five-compartment-ledger.jsonl` at every ten-second sample and `candidate-wss-payloads.jsonl` SHA snapshots each minute and at shutdown. The ledger's **250,760 bytes are only five named logical reservations**: 16 KiB received plaintext, 128 KiB pending TLS output, 64 KiB handshake deframer, 5000-byte outbound ring and 32 KiB inbound ring. Known actual ring occupancy and accessible TLS bytes are separate fields; unavailable private rustls/tungstenite occupancy is null, never fabricated as zero. This is **not** an actual whole-connection allocation measurement. Compare whole-process RSS slope/final against baseline, and perform the separate G5-E deterministic scoped counting-allocator test for requested-layout bytes, high-water and unexplained deltas. Allocator requested bytes are not allocator usable bytes.

The dedicated Linux G5-E measurement is `TRACK-T1/T1.4/g5e-linux-wss-one.json` (one local rustls/Tungstenite WSS client, explicit fixture-setup, handshake/upgrade, idle, fragmented read/write, and shutdown phases). The connected phase recorded 416 scoped allocation events, a driver-thread requested-byte net of +17,649, a fixture-thread net of -26,093, and a **process** live-requested-byte delta of -8,444 from the no-WSS fixture baseline; the process peak during connection was +16,634. The fragmented read/write phase peaked +65,292 from that baseline. A concurrent fixture worker can release its setup allocations during connection, so a positive process-net assertion would be an invalid per-WSS oracle. The static 1,572,864-byte event recorder and unknown native/OS allocation are not part of these requested-layout numbers. This receipt measures rustls/Tungstenite only, **not** the entire client-adapter endpoint, native usable capacity, or the required 20-connection RSS allowance. Keep those categories explicitly unqualified until the full platform-specific G5-E ledger and candidate/baseline soak are evaluated.

The authenticated-control fixture assigns a unique connection ID to each WSS connection. Its socket registration, readiness filters and backpressure assertions use that ID as the mio token, as the product endpoint does. A fixed token would discard readiness for the second connection and invalidate the sibling-progress and shared-memory measurements.

The raw TLS peer retains the initial client traffic secret only in memory. After it authenticates a TLS 1.3 KeyUpdate, it derives the next receive key and resets the record sequence. The G5-E recovery check permits authenticated queued application records after the reply, but rejects another handshake reply or a decryption error. A fixture regression checks the updated-key decoder's sequence, tag and inner-padding handling. The direct `ring` dependency is test-only; release products do not gain a dependency edge.

The proxy keeps raw terminal socket/TLS errors in each connection receipt; the comparator does **not** discard them. A post-response HTTPS TLS BrokenPipe is an accounted close only when headers/body and both ciphertext/plaintext transfer byte counts agree. A WSS `SSLEOFError` or proxy-A BrokenPipe is an accounted close only when both zero-payload WebSocket Close frames were forwarded, both game-binary cumulative hashes agree, client-to-engine bytes all match, and any unmatched TLS1.3 server-to-client tail is at most the two-byte WS Close or 24-byte encrypted final record. Every other error, larger deficit, missing Close or game hash mismatch fails. A Play actor's in-window exit fails separately; this teardown accounting is never permission to reconnect during the soak.

Reject missing/corrupt/short receipts. The qualifying comparator must enforce zero unexpected exits, zero proxy B read/write binary-payload hash divergences, zero candidate/client-vs-proxy payload hash divergences, every active WSS connection's five-compartment reservation ≤250,760 bytes at each sample and no known occupancy overrun, candidate RSS slope ≤ baseline slope +1 MiB/hour after warmup, and candidate final RSS ≤ baseline final RSS + 20×250,760 bytes +5% of baseline final. The census uses §21 §1 plus F0 ADR 0007 amendment 7: total Linux tasks are `T = N + J + V + F + H + A + B + G + E + E_v + L_T + R`, with `N = 20`, named singletons `F ≤ 14` and the newly explicit request threads `R ≤ 64` (`nav-find-`, `nav-inspect-`, `bank-pick-`, `card-prepare`); each other category and the test-harness main task are classified by `comm`. Raw task count is **not** capped at 14. Any unclassified task fails. Check all four latency phases, both traffic directions, the required segment and frame-fault counts, all twenty real per-slot bank cycles and uninterrupted two-hour residency. A failed, unobserved or ambiguous oracle blocks the gate; the smoke may only check infrastructure and must be labeled nonqualifying.
