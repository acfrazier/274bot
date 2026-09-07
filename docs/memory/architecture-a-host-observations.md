# Architecture A — normalized immutable observations, independent publication cursors

Independent design investigation for t_1effd619, on `codex/memory-diagnostics`.
Read-only source/evidence investigation; no implementation, builds, client edits,
native profiling, network access, live workloads, or resource acceptance. The
other new architecture/publication fan-out reports were not read. Source anchors
below refer to the inspected checkout, not the older attribution binaries.

## Recommendation and decision

Keep the distinct observation times. Stop making each observation time own a
complete second copy of the same model.

Recommend a staged host read-model redesign: owner-local publication cursors over
immutable, exactly compared family bodies; then normalize location definitions
and widget component values away from observation-specific context. Canonical
means canonical *equal content*, not a single latest snapshot that all consumers
must read. Do not make client generations an identity certificate. Do not move
input, draw, packet processing, guardian, script dispatch, or client updates.

The bounded first step is rebuild-edge equality dedup of the existing complete
Widgets/SideTabs/Loc values, with all existing owner gates unchanged. It is a
retained-memory experiment, not a CPU optimization. The larger architectural
payoff is compact immutable cores plus cheap owner-local projections, particularly
Loc distance and repeated widget component payloads. That second step requires
an explicit Rust read-surface decision, described below; it cannot honestly be
sold as swapping `Vec` for `Arc` with no consumer migration.

This lane is useful but cannot be the complete budget solution. Archived TUI N16
WidgetView+LocView allocations total 82.472 MiB; the optimistic two-owner whole-
body dedup ceiling is 41.236 MiB allocated, not RSS. Native Linux's diagnostic
N16 RSS excess is 98.683593 MiB and active incremental excess is 13.569791
MiB/additional slot. Even optimistic equal-body sharing is only 2.577250 MiB per
slot in that allocation population. Do not divert the campaign into repeated
snapshot-only profiling to explain the whole gap. Fixed assets, client/world,
renderer, and CPU ownership require the root's independent synthesis.

## Evidence ledger: what exists and what must not be counted twice

Required evidence read: behavior-contract.md, performance-finish-plan.md (budgets,
4A/4B/4C), native-platform-resource-screen-report.md,
windows-adapter-clamshell-report.md, targeted-allocation-owners.md,
incremental-owner-attribution-report.md, snapshot-family-sharing-report.md, and
snapshot-consumer-audit.md. STATE/execution were read for orientation only.

| Population | Evidence | Interpretation for this proposal |
|---|---|---|
| Native Linux N1/N16 | 175,255,552 / 640,348,160 B median RSS; N16 0.72958343 profiled core | Diagnostic scale evidence, not final acceptance; N1 below 256 MiB, N16 above 512 MiB, CPU above 0.5 core |
| Windows five N1 panel cells | 527,222,784–663,781,376 B diagnostic frontend median RSS | Above 384 MiB; provenance/overhead acceptance unavailable; no native N16 inference |
| Archived N1/N16 TUI WidgetView stacks | 3.235 / 53.878 MiB allocated | Includes flat and nested widget bodies; not an extra estimate to add to older panel owner totals |
| Archived N1/N16 TUI LocView stacks | 1.875 / 28.594 MiB allocated | Large per-owner records plus definition clones; resident fraction unknown |
| Other explicit API paths | 0.629 / 10.487 MiB allocated | Potential additional families, not credited to first step |
| Older N32 panel four-owner W+L | 178.750 MiB allocated | Pre-runner-release archive, not current three-owner baseline |
| Removed runner W+L path | 46.141 MiB allocated, verified absent in later capture | Already removed; zero new credit here |
| Shared nav, appearance, animation | Older reports include duplicate nav, inline appearance, larger animation storage | Current source already has campaign changes; zero new credit to this proposal |

The incremental attribution binary used client 451759f2. Later appearance and
animation changes make its construction/fixed-owner figures archival. Its
snapshot builders are a lead supported by current source, not a new capture of
current resident ownership. No fresh field layout or unique-body count was
measured here. Mac allocator/lite overhead and x86_64 RSS are different accounting
systems. No subtraction of Mac malloc bytes from Linux/Windows RSS is a forecast.
V8 mappings, live malloc, allocator free pages, GPU memory and RSS are not additive.

Windows closed-NVIDIA's scheduling miss (250–500 ms p99 bucket) and recovery on
reopen remain a separate presentation/scheduling finding. This design cannot
claim to repair it, or replace GPU callback proof with snapshot throughput.

## Current owners and exact seams

1. `crates/host/src/lib.rs:332–368`: `Host::client_tick` invokes observe before
   `client_frame`. `client_frame:524–548` performs `after_drain`, then guardian
   using `SlotLoop::snapshot`. `SlotLoop:869` owns that snapshot;
   `after_drain:959–979` and `rebuild_dirty:984–1038` select its rebuild calls.
   Dirty-family outer selection is part of the current behavior; invoking every
   family and trusting its inner gate would not be an equivalent replacement.
2. `crates/host-play/src/lib.rs:3795,3865–3989`: independent `nav_snapshot`, rebuilt
   through `observe_rebuild_snapshot:2927` only on tick edges; passed to captures,
   world-state derivation, script observe, queued wire dispatch, and navigation.
   The script observation is not just a thieving/navigation subset.
3. `crates/panel/src/session.rs:786,1816–1823`: `nav_states` retains another
   GameSnapshot plus WorldState. Callback scenario work precedes this rebuild
   (`1785–1799`); follow (`1840–1856`) and UI routing read the stored snapshot.
   Do not assume panel and script callback builds occur without intervening
   client mutations. A same-frame label is not sufficient sharing provenance.
4. `crates/api/src/snapshot.rs:534–665`: snapshot mixes gate/history metadata,
   scalar observations, complete owned families and derived context.
   `rebuild_family:765`, `rebuild:799`, Widgets `1479`, SideTabs `1494`, Loc `1776`
   are the builder seams. `widgets:1028`, `side_tabs:1033`, and `locs` expose
   slice-backed public views; compatibility with these signatures matters.
5. `crates/api/src/snapshot.rs:294–338`: WidgetView has owned text/scripts/actions/
   items and contextual root/parent/coordinates; SideTabView owns another vector
   of those records. `widget_view:2626–2677` clones component fields and derives
   bindings/items. Active side widgets can overlap the flat open-root view;
   exact extent is unmeasured and must not be assumed to equal all widgets.
6. `crates/api/src/snapshot.rs:448–470,2352–2441`: LocView repeats cache definition
   strings/actions/flags and placement, plus distance baked from the local tile
   *at loc rebuild*. `rebuild_loc:1776–1786` does not gate on that tile.
7. Downstream seams include `api/src/query.rs:1516,1554`, `api/src/interact.rs:632–
   737,1179–1217`, host-play's loc conversion at `2841`, script encoding in
   `crates/script/src/isolate_fb.rs:2270–2365` and materialization in
   `crates/script/src/load.rs:1636,1916`. Keep their supported result/order/error
   contracts, including fingerprint/delta and forced bank keyframes.

Narrowing the host to a few guardian scalars is not the recommended shortcut:
`host/src/random.rs:257,1019,1064` reads locs; `667–818` delegates actions through
Interactions. Indirect validation consumes scene/inventory and component state.
Panel navigation likewise uses complete widgets/side tabs for transports, and
bank/world gating. No family is declared dead merely because one workload does
not touch it. Completed scenario retention is already resolved.

## Proposed ownership model (new names, not existing APIs)

### A1. Preserve public types, canonicalize only at rebuild edges

Introduce a slot-instance-local `ObservationStore` owned by the slot lifetime,
shared by host, host-play and panel cursors for that instance. A cursor retains
its current GameSnapshot scalars, gates and history; replace only private
family storage with immutable Arc-owned bodies. Public slice getters and JSON
shape stay as today. Serialization may need explicit delegation rather than a
new implicit serde feature; check crate manifest when implementing.

For each existing family call:

- If its existing outer/inner gate says no rebuild, do nothing: no hash, compare,
  allocation, registry lookup, gate refresh, or replacement. Retain old content.
- If it rebuilds, execute the same walk against this call's Client state and
  capture all values. Gate transitions and return value remain those of the
  original builder even when output compares equal.
- Compare that completed value against the current family bodies held by the
  bounded set of cursors for this slot. Equality includes all ordered children,
  nested fields, absent/empty distinctions and derived values. Loc needs an
  explicit total field comparator; it currently lacks derived PartialEq.
- On exact equality, install the matching Arc and release the candidate's
  duplicate allocation. Otherwise publish a new immutable body. An optional
  hash can reject candidates, never establish equality; first experiment should
  use no hash at all. Do not dedup only serialized fields if Rust readers can
  observe more fields.

Registry entries should be weak references to current per-owner family bodies,
not a strong historical cache. Bound entries by registered cursors, overwrite
on replacement, remove on cursor drop, and clean dead weak entries. Compare
outside any panel-global lock. If two threads miss concurrent interning, permit
a duplicate allocation rather than a value/timing change; quantify lost hits.
Never mutate a published Arc with `make_mut` and inadvertently clone a large
body. Build in scratch, then replace. Scratch high-water capacity and temporary
old/new/candidate overlap are part of the accounting, not free space.

This mechanism defeats CX1–CX6 in snapshot-family-sharing-report.md by comparing
what the later walk actually produced. It does not require a complete mutation
stamp. Unequal tabs, scroll, drag state, draw text, tutorial roots, or distances
remain different bodies under identical generations. A quiet cursor deliberately
stays stale. Immutable sharing therefore preserves, rather than fixes, the old
staleness behavior. There is no per-read comparison and no game-loop reduction.

### A2. Normalize what prevents useful sharing

A1 cannot share two LocView arrays whose only difference is baked distance.
Its array records also repeat definition payloads, and complete WidgetViews bake
context around repeated component content. Proposed second representation:

- `LocDefinition`: immutable copied name/description/actions and definition
  fields. Intern by exact values, scoped to the actual cache identity/lifetime;
  start per-slot, not a process-global id-only table. Different cache revisions
  must not alias. Copy at the host fence; do not borrow mutable client memory.
- `LocPlacement`: ordered typecode/info/layer/tile/shape/angle/footprint and
  definition reference. Canonicalize the full ordered placement sequence by
  exact equality at a loc rebuild. No assumption that model-stamp equality is
  a complete scene identity.
- `LocObservation`: placement-body Arc plus the local tile captured on *that
  owner's last loc rebuild*, including None. Resolve distance from that frozen
  context, never from the current player family. Old snapshots keep old distance
  even when movement updates their player view. This removes the CX6 obstacle
  without changing a single rebuild edge.
- `WidgetValue`: immutable component payload (including mutable-at-client fields
  captured by value: text, scroll, inventory items, scripts, model state, etc.).
  Exact-value interning, not `(component_id, iface_gen)` identity. A field being
  usually static is not proof it is immutable. Shared subpayloads such as
  scripts/strings can subsequently be interned when equality and cost justify it.
- `WidgetOccurrence`: component-value handle plus parent/root/position context.
  Keep the original ordered occurrence lists separately for open roots and each
  side tab. Side-tab active/visible/available flags stay in the owner's frozen
  observation. Keep the current visited-array scope: Widgets uses one visited
  set across its roots, SideTabs another across all tabs. Do not silently turn
  this into per-root visitation or deduplicate occurrences by component id.

Each cursor's manifest references the immutable families at the times it last
rebuilt them; its metadata includes its existing gates and history. This is a
family-version manifest, not a claim that all its families describe one fresh
client epoch. Coherent means preserving the same historical composition the old
snapshot exposed. One new globally latest manifest would be a semantic change.

A2 eliminates owner-specific duplicate core bodies even across distance/context
skew and can reduce duplication *within* one body. It still must observe client
values at each old builder edge. It does not remove all walks or promise CPU
savings. More expensive lookup tables can lose both CPU and memory; prefer a
bounded value arena, compact indices and simple per-slot equality first.

### The nontrivial API boundary

A normalized LocObservation cannot return `&[LocView]` with computed per-record
distance without materializing those records. WidgetOccurrence likewise cannot
provide the current public-field WidgetView slice as zero-copy normalized data.
Keeping a permanently materialized legacy vector in every cursor defeats much
of A2. A transient adapter also needs a lifetime compatible with the caller;
returning a reference to a temporary is not a design.

Root must choose before A2 implementation: authorize an additive borrowed row/
iterator read surface and migrate production host/nav/script consumers to it,
while preserving supported script values, wire bytes and public owned export
semantics; or require existing Rust slice/public-field ABI throughout and stop
at A1. Proposed choice is the additive surface, with legacy owned GameSnapshot
exports remaining explicit compatibility objects, not an automatic cache in
every production cursor. Existing external Rust consumers that require the old
GameSnapshot can keep it and pay its original storage cost. No method is removed
or silently changes its return type. Production cursor integration is a distinct
internal API addition, not a rewrite of the script compatibility layer.

A2's borrowed row API must be specified and reviewed jointly across api/query/
interact/nav/host-play/script before implementation. This report does not invent
an already supported trait or waive the current tests. The first experiment A1
does not require that decision. Do not change FlatBuffer or JavaScript encoding
to make normalization easier: project existing ordered values into the existing
encoder and preserve the current fingerprint candidate's byte oracle.

## Quantified envelope and smallest credible benefit

For a family with k live cursors, equal current body B, metadata H, and retained
scratch S, full dedup's net live allocation reduction is approximately
`(k-1)*B - H - S`. When contents diverge the lower bound is zero reduction (and
metadata/scratch can produce a net increase). More generally count unique live
bodies, not Arc handles: `sum(old cursor capacities) - sum(unique capacities) -
new metadata - retained scratch`. Include nested allocations once each.

Using *rounded archival allocation totals*, not current RSS:

| Case | Conditional upper allocation removal | Minimum claim supported now |
|---|---:|---|
| TUI N16 A1, equal two-owner W+L bodies | 41.236 MiB (26.939 widgets + 14.297 loc) before overhead | 0 MiB demonstrated; equality hit rate unknown |
| TUI N1 A1, same simplifying equal-owner model | About half of 3.235+1.875 MiB | No nonzero resident saving established |
| Panel A1, k=3 | Up to two-thirds of that panel's own equal-body W+L population | No current panel N16 owner capture; do not multiply TUI totals by 3/2 and call it measured |
| A2 normalized W+L | Strict gross ceiling below removing all 82.472 MiB at archived N16; one useful core and projections must remain | No numeric improvement beyond A1 proven without layouts/occupancy |

A1's 41.236 MiB allocation ceiling is about 41.8% of the Linux diagnostic RSS
excess *numerically*, not evidence of covering that fraction in RSS. A2's whole
W+L gross envelope is still smaller than that RSS gap even under the invalidly
optimistic assumption all removed malloc bytes were resident. More importantly,
its per-bot envelope is nowhere near the incremental gap. New helpers are not
proposed; moving bytes to another process would not be a saving.

Minimum credible mechanism-level improvement: on one fixture with two exactly
identical populated family builds, one whole redundant owned backing body and
its uniquely cloned nested allocations can be released, minus explicit Arc/
registry/scratch costs. That is measurable positive allocation removal only if
those bytes exceed overhead. The defensible fleet lower estimate is zero until
the experiment. Claiming a guaranteed 10–40 MiB would invent occupancy and hit
rates. As an engineering go/no-go threshold (not an estimate), require at least
10 MiB net projected retained removal at the archived N16 population or a
comparably demonstrated current population before expanding this lane. Otherwise
park A1 and ask whether A2's broader migration is warranted by its measured
compactness. Do not keep an allocation-saving change that violates the 5% CPU or
2 ms p99 regression rules; provisional retention must label missing evidence.

Savings from A1 and A2 overlap: A2 replaces A1 bodies, it does not get another
41.236 MiB credit on top. Definition string sharing overlaps loc/widget family
stacks. Duplicate active-side widgets are already inside WidgetView totals.
Encoder/fingerprint and consumed-buffer work have separate ownership but may
share traffic benefits; this proposal claims none of their savings. WorldState
and UI manifests retain separate derived state; do not count their removal.

## Oracle and migration

1. Baseline fixtures first: reproduce equal and unequal same-generation
   independent builds at each real owner gate. Capture complete snapshot JSON,
   Rust family values, builder return booleans and gate progression. Preserve
   initial empty sentinels and all stale views; do not turn a failed comparison
   into a corrected expected value.
2. Implement A1 behind an internal candidate path only after root authorization.
   Retain the baseline builder as a test oracle; production does not dual-build
   except for the existing independent cursors. Verify exact equality, ptr_eq
   only on equal contents, and old Arc stability after replacement.
3. If A1's discriminator clears, freeze matched binaries and perform the approved
   bounded screen/confirmation, separate from allocation attribution. Preserve
   failed/Unknown evidence; no automatic unchanged retries. All three modes,
   native target proof and long lifecycle are still final obligations.
4. If authorized, specify A2's additive borrowed views and migrate one family,
   Loc first. Keep legacy and normalized projections in deterministic tests,
   not retained twice in production. Then migrate Widgets/SideTabs, with visit
   ordering and all component fields included. Remove production legacy copies
   only once all traced readers use the replacement; report fallback export
   owners explicitly.
5. Test affected api/host/host-play/nav/script/panel/scenario/TUI suites with the
   relevant memory features, byte/fingerprint/bank/restart fixtures and supported
   errors. Client code is unchanged; do not present host cargo tests as client
   integration execution. Final whole-branch Grok4.6 remains required.

| Risk | Required observable oracle |
|---|---|
| CX1–CX5 local/draw mutation under unchanged gens | Two cursors with different gate histories; reproduce tab, scroll/clamp, drag, friends text and tutorial root changes. Rebuilding cursor sees old baseline walk output; quiet cursor stays stale. Compare all nested values and script packets. |
| CX6 distance | Player moves without loc gate; old cursor retains old distance, newly dirty cursor uses new tile; A2 shares placement only and keeps separate frozen distance origins, including None. |
| Cross-family skew | Different rebuilds for modals/widgets/player/loc remain the old combination; compare command refusals, lookup ordering and numerical results. |
| Guardian/script/nav timing | Deterministic packet/input sequence with pending wait and guardian hold; trace observe, drain, guardian claims, wire commands, nav steps and cancellation. No movement of callbacks or actions. |
| Widget occurrence normalization | Duplicate child/root graphs, tab aliasing, hidden components, nested scroll, empty/None fields; compare exact flat and per-tab occurrence order, root tags, coordinates and items. |
| Lifecycle identity | Stop/restart same username while old invocation retains data; distinct slot-instance stores; old commands cannot reach replacement; live bodies and weak entries release with actual owners. |
| Slow retained reader | Hold old observation across multiple publications; stable bytes/values and bounded registry, while honestly accounting retained reader-owned versions. Never evict live observations to force a memory pass. |
| Fidelity/rendering | Preserve 20 ms logical progression and packet/input/movement/animation/camera/audio/interface order; full-rate, no-renderer, 1 fps background and CPU fallback behavior; scene_state==1 last-FBO freeze and overlays. Functional captures plus cadence/latency gates, not average FPS alone. |
| CPU/peak tradeoff | Count equality bytes, builder allocations, retained scratch and live unique bodies; clean CPU/p99 comparison separately; transient old+candidate+new peaks included. |

## Smallest discriminating experiment (not executed by this design task)

One local deterministic A1 test executable/fixture, no server or renderer, with
three cursors and a slot-local weak registry. Use real GameSnapshot builders and
existing test client/cache fixtures, not invented workload/RSS samples. Sequence:
equal populated rebuilds; each CX1–CX6 mutation with unequal owner gate history;
quiet reads; drop/restart with retained old references. Record original per-owner
requested capacities plus nested allocations, candidate unique-body bytes,
scratch and registry bytes, equality comparisons and walk counts. Verify exact
baseline state/packet parity and zero read-path allocation. Candidate live bytes
must be lower on the equal case and stale/changed cases must remain correct.

This discriminates safety and body-removal feasibility before a broad migration.
It does not estimate real hit rate from synthetic fixtures. Only if it passes,
one separately authorized native owner capture of a qualified N16 workload can
measure overlap frequency/unique bytes and apply the 10 MiB threshold. Use
existing evidence if it contains full required values; existing aggregate malloc
reports alone do not. No unbounded profiling program, no 289 work or paid reset.

## Alternatives rejected and root-owned scope choices

- Generation-only sharing: disproved by CX1–CX6; gate sync cannot repair wrong
  content. Expanding a stamp without proving every mutation site is not enough.
- Single post-drain snapshot reused for all observers: removes copies by changing
  what script/navigation sees after local actions; lacks the required ordering
  proof. Not needed to achieve content sharing.
- Force freshness by adding local-input generations: client/fidelity maintenance
  scope and intentional staleness change; not a hidden storage optimization.
- Global interning by component/loc id across clients: ids are not captured
  values or cache identities; introduces cross-account contamination and a
  process-lifetime retention cache. Per-slot exact-value interning first.
- Narrow script/navigation to the benchmark's fields: drops supported surfaces.
- A full legacy vector cache beside every normalized observation: keeps the
  targeted bodies and adds metadata. Not the recommended A2 deployment.
- Per-read hashing/lazy client access: turns queries into repeated scans or lets
  observations mutate after publication; reject.
- Scheduling fewer updates or delaying input/draw for larger equality windows:
  explicitly violates the fidelity contract, regardless of memory benefit.

Root decisions: approve A1's bounded experiment; decide whether the additive
Rust borrowed read surface is authorized for A2; prioritize this partial owner
win against larger fixed/client/renderer work. Thread scheduling, client mutation
tracking and wire-schema redesign are not implementation details of this card.
Implementation details after those decisions: Arc versus compact arena handles,
comparator mechanics, scratch disposal policy and registry synchronization, all
subject to the memory/lifetime/CPU oracle above.

## Work receipt

Only this report is authored by this card. Source tracing and existing evidence
reads completed; calculations used local `bc`. No new tests/builds/live results
are claimed. Reviewer profile defaults were checked: `grok-4.5` / `xai-oauth`.
The report is for same-card independent review, then root synthesis, not permission
to integrate or claim the final budgets pass.
