# Current per-bot ownership attribution plan

**Task:** `t_1bb2dfd4`  
**Checkout:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Branch:** `codex/memory-diagnostics`  
**Kind:** bounded design / ownership investigation only. No tool, runtime, service,
cache, account, VPS, Windows, or VM work in this card. No representation change.
No new acceptance claim. No live cell, build, or profiling executed by this plan.

Parent evidence package (approved `t_2a69012e`, review `de0a165`, report `8b786b9`):
single-pair TUI active N1/N16 on frozen host `c0709ab` / client `3456edc8` /
binary `a0c6eb0b…`, Concord native Linux, profile-off, System allocator,
`memory-profile-no-alloc`, **`snapshot-dedup` feature-off**.

| Quantity | N1 | N16 | Working TUI target |
|---|---:|---:|---:|
| Steady median RSS | 176.322 MiB | 520.779 MiB | ≤256 / ≤512 |
| Mean process CPU | 0.048993 cores | 0.561746 cores | — / ≤0.5 |
| Descriptive Δ/bot (15 steps) | — | **22.964 MiB / 0.034184 cores** | ≤16 MiB active |

Sources for numbers: `current-tui-n1-n16-attribution-report.md`,
`current-tui-n1-n16-attribution-evidence.json`, independent review
`current-tui-n1-n16-attribution-review.md`. Plan authority:
`performance-finish-plan.md` §§1, 3, 4A/4B; workflow `docs/execution.md`.

---

## 1. Decision framing

**Goal of the next work:** rank concrete per-bot **live owners** and **recurring
CPU consumers** against the descriptive 22.964 MiB and 0.034184-core slopes so
root can choose **one** bounded implementation candidate or **park**.

**Not goals:** performance acceptance; pure client object size claims; linearity
or idle/fixed proof from two points; subtracting helpers/server from frontend;
equating V8 logical heap, malloc stacks, virtual maps, or GPU descriptors to RSS;
re-proposing already-landed empty-table appearance boxing or already-shared nav
as “new” savings; enabling `snapshot-dedup` as if it were the measured default.

**Release rule after this plan’s review:** design approval does **not** authorize
implementation or a live cell. Root freezes binary/fixture and opens a separate
measurement card only if §6 still requires it after static reuse.

---

## 2. Source-present candidates vs frozen runtime defaults

Orch clarification (this card): **code existing ≠ calibration default.**

| Mechanism | Present in current HEAD source? | Frozen calibration default (`c0709ab` / plan) | Keep/park / status |
|---|---|---|---|
| `snapshot-dedup` Arc Widgets/SideTabs/Loc + Play-local `SlotDedupDirectory` | Yes (`crates/api/src/snapshot_dedup.rs`, feature plumbing on api/host/host-play/panel/tui) | **OFF** — `crates/tui/Cargo.toml` opt-in; calibration plan forbids the feature | Mechanism landed + milestone-reviewed as instrument path; **no RSS/CPU acceptance**; feature-off path is production default and is what N1/N16 measured |
| Gen-only family share without content equality | Proposed then **rejected** (`snapshot-family-sharing-report.md`) | N/A | Do not revive gen-only share |
| Appearance `Vec<Option<Box<Packet>>>` | Yes (`vendor/.../client/client.rs` ~358, 941, 6909) | **ON** in client `3456edc8` | Provisionally retained for allocation removal; RSS/CPU claims parked (`appearance-confirmation-report.md`) — **do not reimplement** |
| Borrowed fingerprint live encode | Yes (`script/isolate_fb.rs`, `slot.rs`) | In tree on campaign branch; performance candidate **parked** (no qualified 16-slot performance keep) | Retain source; do not treat as measured RSS owner reduction |
| Animation-base sharing / shared nav pack / `Client::from_shared` cache+ifaces | Yes | ON for production-like host | Already shared fixed owners — **zero new credit** against incremental slope |
| Terminal runner snapshot release after Done | Yes | Harness-only | Not on TUI production multi-shell path |
| TUI renderer | `RasterMode::Off` forced (`crates/tui/src/bin.rs` ~534–546) | OFF | No per-bot GPU/renderer head in this pair |

**Implication:** any ranking that assumes immutable snapshot family sharing is
active **overstates** current savings. The measured pair is **private `Vec`
widgets/loc/side_tabs per `GameSnapshot` shell**, two shells per active TUI slot.

Historical Mac lite stacks (`incremental-owner-attribution-report.md`,
`targeted-allocation-owners.md`, `next-snapshot-ownership-proposal.md`) remain
**family signatures and topology evidence**, not current-binary resident ranks.
Their client (`451759f2` era) pre-dates appearance boxing; panel N32 four-owner
widget/loc totals include panel + scenario paths absent from this TUI pair.

---

## 3. Concrete live owners and lifetimes (current TUI active path)

Topology for the measured workload: one process `tui-play`, N slot threads, no
panel `Session::nav_states`, no headed renderer, shared process assets once.

### 3.1 Shared / fixed (must not be charged per bot)

| Owner | Anchor | Lifetime | Notes |
|---|---|---|---|
| `Play.cache: Arc<Cache>` | `host-play` `Play` ~3051–3061 | Process / Play | Unpacked once; `prepare_client` injects |
| Shared iface decode + mut template Arcs | `host::prepare_client` ~128–141; `Client::from_shared` | Process | COW mut overlays per client, not full re-decode |
| `Play.world: Option<Arc<NavWorld>>` | `host-play` ~3089–3092, load at Play construct | Process | One pack; slots clone Arc |
| Animation / sound shared unpack | client shared load paths | Process | Historical N1=N16 family totals |
| TUI frontend / harness scaffolding | `tui` + `memory-profile-no-alloc` publisher | Process | Harness present; overhead **unmeasured** |

### 3.2 Per-slot live owners (charge candidates)

| Owner | Anchor | Rebuild / work cadence | Retained after script Stop? | After `stop_slot` join? |
|---|---|---|---|---|
| `Client` (simulation, entity tables, boxed appearances, private `World`) | `client/client.rs` `struct Client`; construct via `from_shared` | Packet/`mainloop` every ~20 ms host frame | Yes while slot thread alive | Dropped with thread join |
| `Client.world` scene squares / ground stamps | `core/world.rs` (via client) | Scene rebuilds / ground sets | Yes with client | Dropped with client |
| Host `SlotLoop::snapshot: GameSnapshot` | `host` ~897–899, `after_drain` ~989+, `rebuild_dirty` ~1027 | Dirty families after drain each frame | Yes with slot | Dropped with slot |
| Host-play `nav_snapshot: GameSnapshot` | `host-play` ~3865–3873, `observe_rebuild_snapshot` ~2930–2935 | **Full `rebuild` on tick_edge only**; off-tick keeps last blob | Yes with slot | Dropped with slot |
| `SlotScript` Load isolate + V8 heap | `script/slot.rs` ~27–46; gauges in `memory_profile.rs` | Script ticks; V8 sample ≤1 Hz | Cleared on Stop (pair shows isolates 0) | Gone |
| `last_snapshot: Option<SnapshotFingerprint>` + `IsolateBuf` | `slot.rs`; encode path `isolate_fb.rs` | Each observe encode; borrowed path avoids rebuild when unchanged | Cleared on Stop | Gone |
| In-flight snapshot channel bytes | `script` inflight leases | Transient encode→decode | 0 at pair post_stop | Gone |
| `NavBot` / wires / status row | `host-play` maps on Play | Tick follow / UI | Mostly with slot | Cleared on stop_slot |
| Optional `SlotLoop.renderer` | `host` SlotLoop | Paint path | **None on TUI** (`RasterMode::Off`) | N/A |

### 3.3 `GameSnapshot` heavy families (feature-off storage)

`crates/api/src/snapshot.rs`:

- Feature-off: `widgets: Vec<WidgetView>`, `side_tabs: Vec<SideTabView>`,
  `loc: Vec<LocView>` (private owned bodies).
- Feature-on only: same fields as `Arc<Vec<_>>` + intern via `DedupHandle`
  (`attach_dedup`). **Not active in calibration binary.**
- Gates: Widgets/SideTabs via `InvIfaceGate` (iface+inv gens); Loc via scene gen
  + `loc_model_stamp` (distance-sensitive sweep). Provenance audit already showed
  non-gen inputs; feature-on path therefore uses **content equality intern**, not
  gen-only share (`snapshot-dedup-milestone-review.md`).
- Public getters still `&[T]`. No `Clone` on `GameSnapshot`. Distinct observe-before-drain
  vs after-drain epochs are intentional (`snapshot-consumer-audit.md`).

Payload byte helpers already exist for diagnostics (not RSS):
`widgets_payload_bytes(_vec)`, `side_tabs_payload_bytes(_vec)`,
`locs_payload_bytes` in `snapshot_dedup.rs` — usable offline/feature-on census;
must not be summed into process RSS.

### 3.4 Recurring CPU consumers (per added active bot)

| Consumer | Where | Cadence | Notes |
|---|---|---|---|
| `Client::mainloop` + packet drain | client + host frame | ~50 Hz scheduling target | Pair sustains ~49.5 client iters/slot/s |
| `rebuild_dirty` on host snapshot | host after_drain | Every frame with dirty flags | Widget/loc walks when gates move |
| `nav_snapshot.rebuild` | host-play observe | Server tick edge only | Full rebuild, not dirty-only |
| Script encode + V8 tick | script | Tick edge | Borrowed fingerprint reduces clone churn; CPU keep unproven |
| Guardian / auto-run | host SlotLoop | Frame | Reads host snapshot |
| Nav `Traveller::follow` | host-play | Tick when armed | Uses nav_snapshot + shared NavWorld |
| TUI draw/flush | tui | UI loop | Process-level; not per-bot 3D |

No clean per-phase CPU breakdown exists on the N1/N16 pair (profiles off by design).

---

## 4. What current evidence already shows (reuse first)

From `current-tui-n1-n16-attribution-evidence.json` (logical gauges, **not RSS**):

| Counter | N1 median / post_stop | N16 median / post_stop | Δ/bot (15) |
|---|---:|---:|---:|
| Steady RSS | 176.322 MiB | 520.779 MiB | **22.964 MiB** |
| V8 used | 5.815 MiB | 90.527 MiB | **~5.65 MiB** logical |
| V8 total | 9.000 MiB | 112.747 MiB | **~6.92 MiB** logical |
| snapshot_inflight | (steady present in samples) / **0** post_stop | same / **0** | — |
| Post-stop RSS | 163.078 MiB | 413.703 MiB | **~16.71 MiB** |
| Steady−post_stop RSS | ~13.24 MiB | ~107.08 MiB | **~6.26 MiB** released/bot at Stop |

**Interpretation (descriptive, not causal field proof):**

1. **V8 logical heap explains only a fraction of the 22.964 MiB slope** (~5.7–6.9 MiB
   of *logical* bytes). It must not be treated as RSS. It roughly tracks the
   RSS **released at Stop** per bot (~6.3 MiB), consistent with isolates and
   script-path backing being the main Stop-reclaimed band in this pair.
2. **~16.7 MiB/bot of RSS remains after logical owners report zero**
   (active/V8/inflight = 0). After `stop_slot` join, clients should drop; the
   residual is therefore dominated by **allocator-retained pages and still-mapped
   process fixed cost**, not by “V8 still live.” Equating post-stop RSS to a live
   `GameSnapshot` is false.
3. **N1 already beats the one-bot TUI RSS budget**; the campaign miss is
   **N16 total RSS (~8.8 MiB)**, **active incremental (~7.0 MiB/bot)**, and
   **N16 CPU (~0.062 cores)**. Fixed-cost rewrite is not the first lever the
   pair suggests (§4A still needs owner rank; §4B candidates must be judged
   against this TUI topology).
4. Server (~632–655 MiB median) and helpers are **separately accounted**; do not
   fold into client incremental.
5. Raw `missing_resource_provenance` and **unmeasured overhead** remain open on
   any later pure-client claim; they do not block writing this design.

Historical exclusive allocation families (Mac lite, older binary) useful only as
**ordered leads** under current topology:

| Historical family (allocated MiB, N16−N1) | TUI today | Rank role |
|---|---|---|
| Client construction excl. other | Large | Still per-client struct/tables; appearance empty-table already boxed |
| Client world/build | Large | Best non-snapshot per-client lead (`architecture-c`) — **field census missing on current binary** |
| Snapshot WidgetView + LocView | Large | **Two private shells** feature-off (host + nav), not four; upper bound ≪ old N32 four-owner 179 MiB |
| Other API snapshot paths | Medium | Secondary families on both shells |
| V8 non-mmap stacks | Medium | Aligns with current V8 gauges order-of-magnitude |
| Nav / animation / sound / iface templates | Δ≈0 | Shared — not incremental |

Do **not** sum those historical MiB into 22.964 or into each other.

---

## 5. Exact evidence still missing to rank owners vs 22.964 MiB and CPU

Missing items that would change implement-vs-park:

1. **Current-layout static upper bounds** for one live client + two feature-off
   `GameSnapshot` shells (and fingerprint/`IsolateBuf` steady sizes) on
   **client `3456` + host HEAD**, as requested capacities / `size_of` / payload
   walks — not RSS.
2. **Steady-state unique vs duplicate snapshot payload** under the real TUI
   two-owner attach graph **if** feature-on dedup is ever measured; today we
   only know feature-off duplicates full private vectors when both shells hold
   equal content (source topology), not measured unique body bytes on this pair.
3. **Phase-split process accounting** on one frozen binary:
   observe steady → post script Stop (logical zero) → post all `stop_slot` joins
   → process exit: RSS **and** Linux `RssAnon` / `Private_Dirty` (or equivalent)
   so retained free pages are not labeled as live `World` bytes.
4. **Per-phase CPU** attribution coarse enough to rank mainloop vs snapshot
   rebuild vs script/V8 vs TUI flush without full sample profiling noise
   (bounded timers already exist under opt-in profiles; calibration left them off).
5. **Harness/collector overhead** still `unmeasured` — any candidate comparison
   must keep the same harness or report overhead separately.
6. **Repeatability** — single pair; no CI on slope. One attribution experiment
   must not pretend to close that.

**Not missing for ranking start:** proof that appearance empty-table boxing
exists; proof nav is shared; proof TUI has no renderer; proof dual snapshot
shells exist; V8 vs Stop-release order-of-magnitude; N16 miss direction.

---

## 6. ONE bounded next experiment

### Name

**O1 — Current-source static owner ledger + archive counter decomposition
(with optional single Linux phase-split RSS/smaps cell only if static bounds
cannot rank).**

### Why this shape (not “another easy diagnostic”)

- The pair already supplies RSS/CPU/V8/inflight/post_stop. Re-running N1/N16
  without new owner resolution wastes the frozen cell.
- Historical malloc stacks are wrong binary/topology for causal rank on
  `a0c6eb0b`.
- Full `MallocStackLogging` / lite stacks **perturb CPU into double-digit cores**
  historically and must not be the first tool against a 0.06-core miss.
- Feature-on `snapshot-dedup` census is valuable **only after** static dual-shell
  feature-off upper bounds show snapshot duplication could cover a material
  share of the **7 MiB/bot incremental miss** (not the entire 23 MiB slope).

### Phase A — mandatory, no live cell (this is the primary experiment body)

**Platform:** any dev host; **read-only source + unit/check builds in a later
authorized card**, not this design card.

**Inputs (frozen identities to cite, not rebuild here):**

- Host runtime freeze: `c0709ab` / measurement binary `a0c6eb0b…`
- Client: `3456edc8`
- Current branch HEAD for source anchors (may be ahead on docs/controllers only;
  if source drifts from freeze, say so and do not mix)

**Procedure:**

1. **Owner ledger table** (extend §3): every retained field/path with purpose,
   consumers, sharing boundary, lifetime, and whether it is live at observe /
   post-Stop / post-join.
2. **Static bounds (compile-time or tiny unit tests, no game server):**
   - `size_of` / layout notes for `Client` hot tables (players/NPC/appearance
     slots already boxed), key `World`/`Square` pieces if accessible without
     full client link gymnastics.
   - For representative fixture snapshots (existing api snapshot tests or a
     captured serialized steady sample **if already on disk in archives**):
     run existing `widgets_payload_bytes_vec` / `locs_payload_bytes` style
     accounting **twice** to model feature-off dual-shell retention
     (host + nav), and **once** to model ideal same-epoch Arc share ceiling.
   - Fingerprint/`IsolateBuf` steady upper bound from types + existing script
     tests (allocation proofs already exist for unchanged encode).
3. **Archive decomposition (no new run):** from N1/N16 evidence JSON + raw
   observe samples already archived:
   - Reconfirm V8 used/total medians, post_stop RSS, steady−stop delta.
   - Compute “Stop-reclaimed per bot” vs “post-join residual per bot” as
     **separate columns**.
   - Explicit non-claims: no RSS↔payload identity.
4. **Rank band assignment** against the **7.0 MiB/bot incremental miss** and
   **0.062 core N16 CPU miss** (not against full 23 MiB unless bounds justify):

| Band | Meaning |
|---|---|
| A | Static dual-shell snapshot payload ceiling ≥ ~3 MiB/bot **and** CPU rebuild plausible |
| B | Client world/construction static bound dominates residual after subtracting A and V8-logical band |
| C | Residual primarily allocator page retention / fixed process cost → implementation of representation may not move RSS without allocator/return or live-set shrink proof |
| D | CPU miss dominated by mainloop/script path independent of snapshot bytes |

5. **Stop Phase A** when each band is either bounded with source proof or marked
   **unknowable without Phase B**. Produce implement/park recommendation (§8).

**Phase A success criteria:** a ranked owner list with numeric **upper bounds**
and explicit RSS-domain separation; no performance acceptance language.

### Phase B — optional, single attempt, only if Phase A leaves A/B unordered

**Authorized only by a later root card after this design is review-approved and
Phase A report shows necessity.**

| Item | Choice |
|---|---|
| Platform | **Native Linux Concord** (same class as calibration). Mac allowed only for static/layout; Mac RSS is not Linux rank. No Windows/VPS commands in the design card itself |
| Binary | Prefer **exact** `a0c6eb0b…` replay tooling if owner probes can be external; else one rebuild from freeze with **only** added publish-on counters (no behavior change). Feature list: `memory-profile-no-alloc` **or** narrow counter build; **`snapshot-dedup` still OFF** unless the card is explicitly a dedup discriminator pair |
| N / workload | Prefer **N=1 and N=16** active Thiever, same warmup/observe **or** shortened observe **only** if documented as attribution-not-acceptance; same accounts/cache/server identity discipline as calibration |
| Capture | Process RSS + CPU as today; Linux `/proc/self/smaps_rollup` (or `RssAnon`) at: mid-observe, post script Stop, post all joins, final exit; existing V8/inflight gauges; **no** MallocStackLogging lite/full on the primary attempt |
| Owner coverage | Live client count, snapshot shell counts (always 2×N feature-off), V8 isolates, optional feature-off payload probes if a reviewed tiny counter is pre-approved |
| Accounting rules | Frontend only for slope; server/helpers separate; never sum V8+payload+smaps into one “explained RSS”; double-count ban: pages freed but retained ≠ live `World` |
| CPU | Optional **one** bounded scheduling/client-tick timer window already in tree, not stack profiling |
| Attempts | **One**. On tool failure, preserve artifacts and stop; no favorable retry |
| Decision boundary | If Phase B still cannot separate band B vs C, **park representation work** and record allocator-retention as the blocker rather than inventing a snapshot rewrite |

**Explicitly out of experiment O1:** Windows matched runner; panel GPU modes;
enabling `snapshot-dedup` as default; behavior/cadence/fidelity changes;
merged publication epochs; script API narrowing because Thiever does not read
widgets.

---

## 7. Required tests / proofs (for later implementation cards, not this design)

When a **specific** owner candidate is chosen after O1:

| Candidate class | Proofs before keep |
|---|---|
| Feature-on snapshot dedup (already coded) | Feature-off behavior unchanged; api snapshot tests; snapshot_dedup CX tests; **matched** feature off/on clean N1/N16 RSS+CPU with same freeze; census unique/duplicate bytes; no content mismatch oracles; CPU non-regression margin per plan §5 |
| Further snapshot representation / fewer shells | Forbidden to merge observe/after-drain epochs without behavior contract; consumer audit regressions; guardian ordering; script FB parity |
| Client world/scene packing | Client integration tests separately; layout occupancy discriminator; no cross-client world share assumption (`architecture-c`) |
| Script/V8 path | Existing isolate tests; Stop clears gauges; borrowed fingerprint remains parked until qualified progress+resource pair |
| Allocator purge / return | Must prove live-set shrink or measured RSS drop; purge alone is not a gameplay feature |

Design-card proofs (this commit): source anchors cited; arithmetic from evidence
JSON; no STATE edit; no code edit.

---

## 8. What findings choose implementation vs park

| Finding from O1 | Action |
|---|---|
| Dual-shell feature-off snapshot payload upper bound covers a **material** share of the **7 MiB/bot miss**, and feature-on census (if run) shows high equality hit rate on TUI host+nav | Prefer **measure** existing `snapshot-dedup` off/on as the **first** implementation-adjacent experiment (code already present). Keep is still evidence-gated; default stays off until clean win |
| Snapshot ceiling ≪ miss; client world/construction bounds dominate | Open **one** client-side representation design (world/tile packing), not snapshot rewrite; requires separate design+tests |
| Stop-reclaimed ≈ V8 band and residual ≈ allocator pages (band C) with small live logical payloads | **Park** large Rust representation bets for RSS; document that incremental RSS is mostly non-returned pages / fixed cost; CPU work goes to plan §4C profiling of mainloop/script without allocation logging |
| CPU miss with tiny memory ceilings | Do **not** ship memory-only dedup expecting CPU fix; schedule bounded CPU attribution (§4C) instead |
| Any candidate needs epoch merge, family drop, or gen-only share | **Reject** (consumer audit + family-sharing provenance) |
| Appearance / shared nav / animation re-proposed as new incremental win | **Reject as double-count** |

**Hard parks already in force:** performance acceptance from the current pair;
idle incremental; intercept-as-measured-fixed-cost; helper CPU as overhead;
borrowed-fingerprint performance keep without new qualified evidence.

---

## 9. Deliverable and non-deliverables

**This card delivers:** `docs/memory/current-per-bot-owner-attribution-plan.md`
(this file) on `codex/memory-diagnostics`.

**Does not deliver:** code/feature changes; STATE updates; live cells; binary
builds; service/account/cache edits; merged epochs; acceptance; implementation
authorization.

**Next after same-card `reviewer` approval:** root may authorize Phase A
execution (static ledger card) and only then Phase B if the Phase A report’s
necessity clause fires.

---

## 10. Source anchor index (quick)

| Topic | Path |
|---|---|
| Slot snapshot + rebuild_dirty | `crates/host/src/lib.rs` (`SlotLoop`, `after_drain`, `rebuild_dirty`) |
| nav_snapshot + tick_edge rebuild | `crates/host-play/src/lib.rs` (`observe_rebuild_snapshot`, slot thread ~3865+) |
| Play shared cache/nav | `crates/host-play/src/lib.rs` (`Play`, `prepare_client` in host) |
| GameSnapshot families | `crates/api/src/snapshot.rs` |
| Dedup feature (off by default) | `crates/api/src/snapshot_dedup.rs`; crate features `snapshot-dedup` |
| SlotScript / fingerprint / IsolateBuf | `crates/script/src/slot.rs`, `isolate_fb.rs` |
| V8/inflight gauges | `crates/script/src/memory_profile.rs`; samples via `host-play/src/memory.rs` |
| RSS sampling | `crates/host-play/src/rss.rs` |
| TUI raster off | `crates/tui/src/bin.rs` |
| Appearance boxing | `vendor/fr-client-rust/crates/client/src/client/client.rs` |
| Calibration feature policy | `docs/memory/current-native-tui-calibration-plan.md` |
| Pair evidence | `docs/memory/current-tui-n1-n16-attribution-*.md/json` |

---

## 11. Bottom line

The measured miss is **incremental and N16**, not one-bot fixed cost. Current
defaults keep **two private snapshot shells per bot** and **no snapshot-dedup**.
Existing counters already split a **~6 MiB/bot Stop-reclaimed (V8-scale) band**
from a **~17 MiB/bot post-logical-zero RSS residual** likely dominated by
**allocator retention + process fixed cost**, not live V8. The next step is
**O1 Phase A static/archive owner ranking** against the **7 MiB/bot incremental
miss** and **CPU miss**, reusing payload helpers and pair math; only if that
cannot order snapshot vs client-world vs allocator bands should root authorize
**one** Linux phase-split smaps cell. No Rust representation optimization until
that ranked evidence and a reviewed candidate design exist.
