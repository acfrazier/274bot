# Next retained snapshot ownership reduction

Design only under `performance-finish-plan.md` §§4A/4B. No implementation,
cargo builds, benchmarks, live runs, or campaign stop. Root chooses and
implements only after same-card review; whole-branch Grok 4.6 remains required
later. Appearance-table boxing and animation-base sharing are already
implemented/provisionally retained and are **out of scope** here.

## Budget context (not a new measurement)

Linux N16 remains about **610.68 MiB / 0.7296 cores** versus targets
**512 MiB / 0.5 cores**. This card only proposes a measured-owner mechanism to
close remaining gaps. Perturbed native allocation evidence is used for owner
ranking, not RSS acceptance.

Sources: task brief; `docs/memory/incremental-owner-attribution-report.md`;
`docs/memory/targeted-allocation-owners.md`; `docs/memory/snapshot-consumer-audit.md`;
`docs/memory/performance-finish-plan.md` §§4A/4B.

## Largest provably redundant retained family

### Measured heap lead

| Family (malloc stacks, not RSS) | N1 | N16 | N16−N1 |
|---|---:|---:|---:|
| Snapshot **WidgetView** | 3.235 MiB | **53.878 MiB** | **50.643 MiB** |
| Snapshot LocView | 1.875 MiB | 28.594 MiB | 26.719 MiB |
| Other explicit API snapshot paths | 0.629 MiB | 10.487 MiB | 9.858 MiB |

(`incremental-owner-attribution-report.md`; lite stack logging; perturbed.)

At N32, **WidgetView + LocView vector bodies alone** sum to about **178.75 MiB**
across four call paths with nearly equal weight
(`targeted-allocation-owners.md`):

| Path | Idle/active bytes | Owner |
|---|---:|---|
| Panel Session callback | 48,381,952 | `Session::nav_states` `GameSnapshot` per slot |
| Host `client_frame` / after_drain | 45,334,528 | `SlotLoop::snapshot` |
| Host-play observation | 45,334,528 | per-slot `nav_snapshot` in observe closure |
| `ScenarioRunner::tick_with_hold` | 48,381,952 | runner snapshot (terminal release already implemented) |

Similar content **does not** authorize one mutable `GameSnapshot` or merged
publication epochs (`snapshot-consumer-audit.md`). It **does** establish
duplicate **immutable family payloads** retained by independent shells for the
same client.

**Primary redundant family:** `GameSnapshot` private `widgets: Vec<WidgetView>`
(and the nested `side_tabs[].widgets` trees rebuilt beside it). **Secondary
same-mechanism family:** `loc: Vec<LocView>` (same multi-owner pattern, smaller).

Do not drop Widgets/SideTabs because a common Thiever path does not read them:
panel navigation uses widgets, side tabs, dialogue, modals; script observation
encodes the full public surface (`snapshot-consumer-audit.md`;
`nav/traveller.rs` `spell_button` / `find_component`).

## Current ownership, invalidation, serialization, public API

### Storage and gates (`crates/api/src/snapshot.rs`)

- `GameSnapshot` is a generation-stamped **owned** read model. Private fields
  include `widgets: Vec<WidgetView>`, `side_tabs: Vec<SideTabView>`
  (`SideTabView.widgets: Vec<WidgetView>`), `loc: Vec<LocView>`, plus many
  smaller families.
- `Family::Widgets` / `Family::SideTabs` rebuild via `InvIfaceGate` on
  **`client.gens.iface` and `client.gens.inv`** (component tree + TYPE_INV
  slot data). Rebuild walks open roots (`widget_roots`) or all 14 side icons
  into owned `WidgetView` values (strings, item rows, script vectors cloned
  from iface tables).
- `Family::Loc` tracks scene gen + `loc_model_stamp` (locs can change without
  a scene gen bump).
- Public accessors return borrows: `widgets() -> &[WidgetView]`,
  `side_tabs() -> &[SideTabView]`, `locs() -> &[LocView]`.
- `ReadContext::component` scans `widgets()` then `side_tabs[].widgets`.
- `GameSnapshot` derives `Serialize` for terminal/shot sidecars; gates and
  some gens are `serde(skip)`. Public view structs keep **owned** fields
  (`WidgetView` / `SideTabView` field layout is the compatibility surface).
- There is **no** `Clone` on `GameSnapshot`; owners hold distinct structs and
  rebuild in place.

### Consumers and publication boundaries

| Owner | Where | When rebuilt | Readers / contract |
|---|---|---|---|
| Host `SlotLoop::snapshot` | `crates/host/src/lib.rs` | Every frame after `mainloop` via `rebuild_dirty` from `Pump` dirty flags | Guardian `tick(client, &slot.snapshot, …)` immediately after drain; auto-run energy path |
| Host-play `nav_snapshot` | `crates/host-play/src/lib.rs` observe closure | **`tick_edge` only** (`observe_rebuild_snapshot` → full `rebuild`) | Script FB encode / dispatch, wire `Interactions`, `step_nav_bot`, optional nav capture; off-tick keeps last blob |
| Panel `Session::nav_states` | `crates/panel/src/session.rs` | Observe callback: `GameSnapshot::rebuild` then `WorldState::from_snapshot` when any family moves | UI WalkTo / BankBudget / follow; traveller spell/glider/component search |
| `ScenarioRunner::snapshot` | `crates/scenario/src/runner.rs` | Each non-Done tick; **replaced with `GameSnapshot::new()` after whole terminal tick returns** | Seeding, arms, nav, shot sink, evidence (evidence copies facts; does not borrow working snap) |

Frame order (`Host::client_tick`): **observe first** (prev-frame guardian
status), then `client_frame` (`mainloop` → paint → `after_drain` → guardian).
Observe snapshots therefore publish the client state **before** this frame’s
packets; host `SlotLoop::snapshot` publishes **after** this frame’s drain.
Those are **distinct epochs by design**. Collapsing them into one mutable
shell would move guardian/script/nav relative to packet processing.

Harness note: terminal runner release already removed the fourth vector owner
after Done (`snapshot-consumer-audit.md`); production panel/TUI paths still
carry two or three live shells per slot.

### Forbidden shapes (explicit)

- One shared **mutable** `GameSnapshot` for all consumers.
- Global intern across clients / slots.
- Provenance = generation counters alone if any non-gen client input still
  mutates views without bumping those counters (must be proven or folded into
  the gate before share).
- Dropping a family because one script does not read it.
- Relabeling appearance/animation layout wins as this card’s savings.

## Hypothesis assessment

> Private Arc-backed immutable family sharing among **same-client** snapshot
> owners, with explicit per-client bounded lifetime and equality/provenance
> gates, could remove duplicate retained WidgetView storage while keeping
> distinct publication epochs.

**Verdict: accept as the next bounded ownership candidate**, scoped to heavy
vector families only, with a mandatory provenance audit gate before code.

### Why it can remove material storage

At steady state after a widgets/locs rebuild, panel (if present), host-play
`nav_snapshot`, and host `SlotLoop::snapshot` retain **separate `Vec` bodies**
with equal content for the same slot. Native path sums are nearly equal per
owner. Replacing those bodies with `Arc<[…]>` / `Arc<Vec<…>>` clones under a
**per-slot publication registry** removes (owners−1) full payloads while each
shell remains a distinct `GameSnapshot` value with its own gens/gates/scalars.

### Why it is not free / not automatic RSS

- Allocation evidence ≠ RSS; allocator retention and fixed working set remain.
- TUI active path typically has **two** production shells (host + host-play),
  not four; upper bound is ~½ of process WidgetView/LocView stacks when both
  live and match, not ¾.
- Panel active path can have **three** shells → up to ~⅔ of those vector
  bodies when all match.
- Rebuild cadence differs (host every dirty frame; host-play tick_edge;
  panel on observe rebuild). Sharing must key off **published provenance**,
  not “same wall clock.”
- First rebuild still walks and allocates one full body; savings are
  **retained duplication**, not first-touch construction.

### Provenance contract (must hold before implement)

Share a published widgets blob only when all of the following match the
blob’s stamped provenance for that slot:

1. Same slot/client identity (never cross-client).
2. `gens.iface` and `gens.inv` equal to the blob’s gate stamp (today’s
   `InvIfaceGate` inputs).
3. **Audit pass:** every input to `widget_view` / `walk_widget_tree` /
   `widget_roots` / side-tab availability (`side_icon`, `active_icon`,
   `side_modal_id`, modal roots, inv slot materialization) either (a) cannot
   change without bumping iface and/or inv gen, or (b) is added to the stamp.
4. Optional content equality check in debug/tests only (`ptr_eq` after share;
   full `PartialEq` on `WidgetView` slices when forcing dual rebuild).

Loc blobs: stamp `loc_gen` + `loc_model_stamp` (existing loc rebuild gates).

If the audit finds a non-gen input that changes visible widgets/locs without
a stamp bump, **do not ship gen-only sharing**; either extend the stamp or
**reject this mechanism** and take the buffer-pool candidate below.

## Concrete mechanism

Name: **per-slot immutable family publication (Arc bodies).**

### Lifecycle / ownership

```
Slot thread (host run_client observe + frame):
  FamilyPub {
    widgets: Option<PubBlob<Arc<Vec<WidgetView>>>>,      // stamp: iface, inv, …
    side_tabs: Option<PubBlob<Arc<Vec<SideTabView>>>>,  // same stamp family
    loc: Option<PubBlob<Arc<Vec<LocView>>>>,            // stamp: loc_gen, model_stamp
  }
```

- Lives only on the slot thread / slot-private state; dropped on slot stop
  with `SlotLoop` / observe closure teardown. **No process-global intern.**
- `PubBlob { provenance, body: Arc<…> }`.
- On rebuild of family F for owner O:
  1. Compute prospective provenance from `Client`.
  2. If `FamilyPub` has F with equal provenance → `Arc::clone` the body into
     O’s private field **and still advance O’s private gates/stamps exactly
     as a successful walk would today** (see gate sync below); do not walk.
  3. Else walk/build a **fresh** `Vec` (never mutate a shared Arc body), wrap
     `Arc::new`, store in `FamilyPub`, install in O, and advance the same
     gates/stamps.
- **Gate/stamp sync on every install (share or walk):** Today
  `InvIfaceGate::moved` always writes `iface`/`inv` from the client when
  consulted, and `rebuild_loc` always updates `loc_gen` (via `track`) and
  `loc_model_stamp` when it decides to rebuild. A share path that only
  clones the Arc and skips the walk **must still** set on receiving owner O:
  - Widgets: `widgets_gate.iface` / `widgets_gate.inv` ← `client.gens.iface` /
    `client.gens.inv` (same side effect as `widgets_gate.moved`).
  - Side tabs: `side_tabs_gate.iface` / `side_tabs_gate.inv` likewise.
  - Loc: `loc_gen` ← `client.gens.scene` and `loc_model_stamp` ←
    `loc_model_stamp(client)` (same as a dirty loc rebuild).
  Without this, the next quiet-frame gate consult can re-fire a false
  “moved,” or skip a real move, desyncing stale/dirty behavior versus
  today’s rebuild path. Returning `true`/`false` from the family rebuild
  helper must match today’s meaning (content installed vs gate said stay).
- **Replace-only Arc bodies (non-mutation):** Today `rebuild_widgets` /
  `rebuild_side_tabs` / `rebuild_loc` `clear()` + push **in place** on a
  uniquely owned `Vec`. Under sharing that is forbidden on a multi-ref Arc:
  never mutate through a shared `Arc` (`clear`, push, index mut, interior
  field writes). On rebuild always allocate a fresh `Vec`, then
  `Arc::new` / swap the field pointer. In-place `clear`+reuse only when
  `Arc::get_mut` proves unique ownership is an **optional** micro-opt, not
  required. Serde still emits the **inner sequence** shape (no Arc wrapper
  in JSON).
- Owners may still hold an **older** Arc after a new publication (previous
  epoch). That is intentional: distinct epochs stay readable until that owner
  rebuilds. Dropping the last Arc frees the body (bounded by owner count ×
  distinct epochs still referenced — typically 1–2 short-lived gens, not
  unbounded history). **Do not** retain a generation ring buffer.
- Host `SlotLoop::snapshot`, host-play `nav_snapshot`, and panel `nav_states`
  remain separate structs; only heavy vector fields become Arc-backed
  privately.
- Live share rate also depends on **observe-before-drain vs after-drain**
  epoch skew (frame order above): owners that rebuild on different packet
  epochs will not match provenance until their stamps catch up; that is
  expected, not a merge of epochs.

### Public API compatibility

- Keep `WidgetView` / `SideTabView` / `LocView` public owned field layouts.
- Keep `widgets() -> &[WidgetView]` etc. via `Arc` deref/slice.
- `ReadContext`, `Interactions`, traveller, guardian continue to take
  `&GameSnapshot` and read slices.
- Serde: serialize Arc payloads as the inner sequences (same JSON shape for
  shots/evidence).
- Do not expose `Arc` in the public api surface unless required for a thin
  type alias; private fields may use `Arc`.

### Exact affected files (implementation later)

| File | Change |
|---|---|
| `crates/api/src/snapshot.rs` | Private `widgets` / `side_tabs` / `loc` storage → Arc; install/rebuild helpers that accept optional `FamilyPub`; gates stamp provenance; accessors unchanged |
| `crates/host/src/lib.rs` | `SlotLoop` holds `FamilyPub`; `rebuild_dirty` goes through publish/clone |
| `crates/host-play/src/lib.rs` | Observe closure shares the same `FamilyPub` handle for `nav_snapshot` rebuild |
| `crates/panel/src/session.rs` | `nav_states` rebuild uses same per-slot pub when available (thread-safe handle or rebuild path wired through host-play/panel callback already on slot thread) |
| `crates/scenario/src/runner.rs` | Optional: use local `FamilyPub` only if multi-shell appears; terminal release remains; **no** merge with production pubs |
| `crates/api/tests/snapshot.rs` (+ focused unit tests) | Provenance, ptr_eq share, non-share on stamp move, serialize shape |
| `crates/host` / `host-play` / `panel` / `nav` tests as needed | Guardian order, tick_edge stale blob, walk/component resolution |

No client submodule representation change. No appearance/animation revisits.

### Source-derived retained-allocation limits (not RSS)

Let \(W\) = live WidgetView (+ nested side-tab widget) heap for one full rebuild
at scale, \(L\) = LocView heap, \(k\) = number of co-living same-slot shells
that hold that family at matching provenance.

| Bound | Expression | Notes |
|---|---|---|
| **Upper removed allocation** | \((k-1)(W+L)\) while stamps match | N16 TUI: \(k\approx2\) → ~½ of process WV+LV stacks if both owners always match; panel active \(k\approx3\) → ~⅔. Using incremental N16 totals as crude process totals: WV 53.878 + Loc 28.594 ≈ 82.5 MiB → **upper ~41 MiB (TUI)** or **~55 MiB (panel k=3)** allocation, not RSS. N32 path table ~45–48 MiB/owner × (k−1) for WV+LV vectors only. |
| **Lower removed allocation** | **0** | If provenance never matches across owners; if only one shell lives; if audit forces no share; if rebuild always unique content every owner every time |
| **Unchanged** | First-touch walk cost; string/item heap inside one body; non-Arc families; IsolateBuf/FB fingerprints; client construction; shared nav/anim | |

These are **source + owner-count** limits on duplicate vector bodies. Clean
RSS/CPU deltas require the matched experiment below.

### CPU / copy tradeoff

| | Today | Proposed |
|---|---|---|
| Walks per stamp change | Up to \(k\) full widget/loc walks | **One** walk + up to \(k\) Arc clones on **install** (owners that rebuild this stamp) |
| Quiet frames (no install) | Gates skip rebuild; \(k\) retained duplicate bodies | Gates skip; owners **retain existing Arc refs** to one body — **no** per-quiet-frame Arc clones |
| Refcount | None | Atomic inc/dec only on install / owner drop / epoch publish replace; should be cheap vs walk |
| Risk | — | Extra indirection; must not add fleet-wide locks (slot-local only) |

Net: should **reduce** CPU on multi-owner rebuild edges; quiet frames stay
gate-skip with stable refs (refcount churn is not a quiet-frame cost). Quiet
RSS depends on whether duplicate bodies were resident. Reject if clean CPU
regresses beyond plan margins without clear allocation removal.

## Old / new test oracle

**Old oracle (preserve):**

- Family rebuild only when gates say moved; stale views otherwise.
- `widgets()` / `side_tabs()` / `component` content and order from walks.
- Guardian after_drain ordering; hold freezes follow/script tick as today.
- Host-play off-tick keeps last nav snapshot; tick_edge rebuilds.
- Panel nav publish + WorldState gating; traveller spell/side-tab search.
- Script complete snapshot surface (FB fields), delivery/order/skip/timeout
  unchanged by this card.
- Scenario terminal shot sees full snapshot; post-Done working snap empty;
  same-tick follow-failure continuation still sees snapshot through outer
  return.
- Serde shot shape for widget/loc arrays.

**New oracle (add):**

- When two owners rebuild with equal provenance, `Arc::ptr_eq` on internal
  bodies (test-only access or cfg).
- Shared install advances the receiving owner’s private gates/stamps to the
  same values a walk would (`widgets_gate` / `side_tabs_gate` iface+inv;
  `loc_gen` + `loc_model_stamp`). After share, a second rebuild call with
  **unchanged** client gens returns false (gate stay) without allocating or
  walking; after a real iface/inv/scene/model move, rebuild runs and is not
  stuck on a stale gate.
- Bodies are replace-only: after share, mutating one owner’s view storage
  must be impossible without unique ownership; a rebuild always installs a
  new Arc pointer when content is rebuilt (no in-place clear of a shared
  body).
- When iface/inv (or loc stamp) moves, no ptr_eq with previous blob; content
  matches a fresh independent rebuild golden.
- Owner A holding epoch E0 while owner B publishes E1: A still reads E0
  content until A rebuilds; no cross-talk.
- Stop/slot teardown: no retained FamilyPub / Arc cycles (strong count drops).
- Debug assert path: optional deep `PartialEq` after share vs forced private
  rebuild.

## Clean matched experiment + native attribution plan

1. **Freeze** pre/post binaries; same client digest, nav pack, cache, allocator,
   frontend geometry; no lite stack logging in clean cells.
2. **Modes:** real TUI N1 and N16 active (primary duplication is host +
   host-play). Optional panel N1/N16 if three-owner savings are claimed.
3. **Screening:** short 30/120/60 paired R/C or C/R; then longer confirmation
   only if allocation removal and non-regression look real.
4. **Claims:** RSS median/peak, CPU cores, p99 latency margins per plan (5%
   CPU / 2 ms p99). Do not subtract malloc from RSS.
5. **Native attribution (separate perturbed cell after clean):** lite
   `malloc_history` owner filters for WidgetView/LocView stacks; expect
   **one** dominant body scale per slot for shared families at steady state,
   not \(k\) near-equal path groups. Record path labels for panel vs host vs
   host-play.
6. **Keep / park / reject** on evidence only. Intermediate above-budget
   improvement may be provisionally retained; final 512 / 0.5 acceptance is
   separate.

## Fallback if Arc family share cannot safely remove material storage

Reject Arc sharing when any of: provenance audit fails; clean cells show no
allocation removal and no RSS movement beyond noise; CPU/latency regression
without benefit; implementation would require global intern or single mutable
snapshot.

Then implement the **already-approved consumed-buffer pool**
(`performance-finish-plan.md` §4B) as the independent next candidate:

### Consumed-buffer pool — ownership / lifetime

| Piece | Today | Proposed |
|---|---|---|
| Encode scratch | `IsolateBuf` FlatBufferBuilder **already reset/reused** per slot | Unchanged |
| Posted bytes | `copy_finished()` → **new `Vec<u8>`** every post (`isolate_fb.rs`) | Fill from pool or fresh; hand off ownership to isolate |
| In flight | `LoadIsolate::post_snapshot(Vec<u8>)` → channel `SnapshotMessage { bytes }` | Same delivery/order; message carries pooled buffer id/generation if needed |
| After decode/materialize | `bytes` dropped with message | **Return** buffer to per-isolate pool **only after** JS snapshot object materialization finished (or tick skip path that drops unread snapshot) |
| Pool size | — | **At most two** buffers per isolate (in-flight + spare), matching plan |
| On Start/Stop | `SlotScript` rebuilds `IsolateBuf`; isolate join | Drain pool; drop buffers; no cross-isolate reuse |
| Behavior preserve | skip/timeout/interrupt, keyframe/delta, force banks | Bit-identical FB content; only backing allocation identity changes |
| Verify | — | Reuse counters / pointer stability tests; native IsolateBuf path sizes; no earlier packet lifetime extension past materialization |

Affected files (fallback): `crates/script/src/isolate_fb.rs` (`copy_finished` /
encode returns), `crates/script/src/load.rs` (post + isolate-thread return
path), `crates/script/src/slot.rs` (wire pool through encode/post), tests in
script load/isolate_fb. Does **not** replace WidgetView multi-owner retention;
targets **active IsolateBuf ~30.6 MiB** class churn
(`targeted-allocation-owners.md`), complementary to Arc family share.

## Recommendation summary

1. **Next ownership optimization:** per-slot **Arc-backed immutable
   Widgets/SideTabs (+ Loc) family publication** among same-client snapshot
   shells, preserving distinct epochs and public read APIs.
2. **Largest redundant family:** retained **WidgetView** vectors (with
   side-tab nested trees); LocView same pattern secondary.
3. **Hard gates:** provenance audit for non-gen inputs; share install must
   sync owner family gates/stamps; replace-only Arc bodies (no shared
   in-place mutate); no mutable shared `GameSnapshot`; no cross-client intern;
   no family dropping for script subsets; no double-counting
   appearance/animation wins.
4. **If rejected:** consumed-buffer pool (≤2 per isolate, return after
   materialize) as the independent §4B candidate with the lifetime table
   above.
5. **Out of scope here:** implementation, live runs, final budget acceptance,
   Windows/VPS/289 work.

Deliverable path: `docs/memory/next-snapshot-ownership-proposal.md` only.
