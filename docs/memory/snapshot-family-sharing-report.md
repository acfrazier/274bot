# Snapshot family Arc sharing — stop after provenance audit

Implements the mandatory provenance gate from
`docs/memory/next-snapshot-ownership-proposal.md` (approved `a6a418c`) under
`performance-finish-plan.md` §§4A/4B. **No Arc family-sharing mechanism was
shipped.** Gen-only (and any practical non-content) share stamps are unsafe for
Widgets/SideTabs and Loc. No buffer-pool fallback, no gameplay change, no
client submodule edits, no live/RSS claims.

## Checkout

| Item | Value |
|------|-------|
| Branch | `codex/memory-diagnostics` |
| Workspace | `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f` |
| Proposal base | `a6a418c` (Arc share gate/replace/quiet contract) |
| Code change | **none** (report only) |

## Verdict

**Reject / stop this mechanism** for same-client multi-shell Arc publication of
`widgets` / `side_tabs` / `loc` under today’s rebuild gates.

Reason: builders read multiple **non-generation** client inputs that change
visible snapshot content **without** bumping the gates used for share
equality. Publishing under `InvIfaceGate` (`gens.iface`+`gens.inv`) or under
`loc_gen`+`loc_model_stamp` alone lets a later owner install an earlier owner’s
body after those inputs drift — content that an **independent walk on that
later rebuild** would not produce. Extending the share stamp to cover every
such input either (a) becomes a near-full content/input hash of iface overlays
and loc distance context every install, or (b) misses a site and silently
shares wrong bodies. Neither is the reviewed “proven provenance” bar; the brief
forbids implementing a fallback or changing gates/gameplay here.

## Current ownership (unchanged)

| Owner | Storage | Rebuild trigger |
|-------|---------|-----------------|
| Host `SlotLoop::snapshot` | `crates/host` | `rebuild_dirty` after drain |
| Host-play `nav_snapshot` | `crates/host-play` | tick_edge observe rebuild |
| Panel `Session::nav_states` | `crates/panel` | observe callback full rebuild |
| Scenario runner | terminal release already | out of production multi-shell path |

Distinct publication epochs (observe-before-drain vs after-drain) remain
required. No single mutable `GameSnapshot`.

### Gates today (`crates/api/src/snapshot.rs`)

| Family | Gate | Rebuild body |
|--------|------|----------------|
| Widgets | `InvIfaceGate` ← `client.gens.iface`, `client.gens.inv` | `widget_roots` + `walk_widget_tree` → `widget_view` |
| SideTabs | same `InvIfaceGate` | all 14 `side_icon` roots; `active`/`visible` from `active_icon` / `side_modal_id` |
| Loc | `track(gens.scene → loc_gen)` **or** `loc_model_stamp(client)` change | 104×104×4 sweep → `loc_view` (includes **distance** from local player tile) |

`InvIfaceGate::moved` always writes current iface/inv into the owner gate when
consulted (share install would have needed the same sync). Quiet frames skip
walks when gates match.

## Provenance audit — builder inputs

### A. Widgets

**Root selection (`widget_roots`)** reads:

| Input | Packet gen bump? | Local / other mutation without iface+inv bump? |
|-------|------------------|-----------------------------------------------|
| `main_modal_id` | IF_OPEN*/CLOSE family → `gens.iface` | Cold login reset; modal close helpers tied to packets |
| `main_overlay_id` | IF_OPENOVERLAY → iface | — |
| `side_modal_id` | IF_OPEN*/CLOSE → iface | — |
| `side_icon[active_icon]` | IF_SETICON → iface | — |
| `active_icon` | IF_SHOWICON → iface | **`handle_tab_clicks`** (mouse, no gen); cold login → 3; **`TUT_FLASH`** (no gen in `bump_gens`) |
| `chat_modal_id` | IF_OPENCHAT/CLOSE → iface | — |
| `tut_com_id` | **none** | **`TUT_OPEN` / `TUT_FLASH` path** — not listed in `bump_gens` iface arm |

**Walk / `widget_view`** reads per component:

| Field / source | Packet gen? | Non-gen change? |
|----------------|-------------|-----------------|
| layout, type, button_type, models, scripts, colours, hide, text, … from iface | IF_SET* / open → iface | **`clientComponent`** (draw path) rewrites friends/ignore **text** / **button_type** / scroll_height **every draw** without gen |
| `scroll_position` ← `com.scroll_pos` | IF_SETSCROLLPOS → iface | **Scrollbar input** (`scroll_pos` ± cycles / grip); **draw clamp** and drag-autoscroll writebacks in `draw.rs` |
| TYPE_INV `items` ← `link_obj_type` / `link_obj_number` | UPDATE_INV_* → inv | **Local inventory drag/drop** mutates link slots in `ifaces_mut` **before** server inv packets (no `gens.inv` in that path) |
| item def strings | cache | shared immutable after load (OK for same client) |
| `ifaces_len` / visited | table size | load/rebuild paths; treated as part of iface lifetime |

### B. Side tabs

Same walk/`widget_view` inputs as widgets, plus:

| Input | In side_tabs body? | Gen-covered? |
|-------|--------------------|--------------|
| `side_icon[i]` | root id / available | IF_SETICON → iface |
| `active_icon == index` | `active` flag | **tab clicks / TUT_FLASH without gen** |
| `active && side_modal_id == -1 && available` | `visible` | side_modal via iface packets; **active without gen** |
| nested `widgets` | full trees for every available tab | same component hazards as A |

### C. Loc

| Input | Gate today | Non-gen drift? |
|-------|------------|----------------|
| wall/scene/gd/decor typecodes | scene gen and/or `tile_model_stamp` aggregate | model_stamp covers door/multiloc class intended by gate |
| `map_build_base_*`, `minusedlevel` | scene/rebuild | rebuild/scene family |
| **local player tile** (`local_world_tile`) | **not in gate** | player route moves without scene/model stamp → **`LocView.distance` changes only when a loc rebuild actually runs** |
| cache loc defs | shared | OK |

Single-owner stale distance until next loc dirty is **existing** behavior.
Cross-owner **share on first install** after player moved under equal
scene+model stamp is **new** wrong content vs independent walk.

### D. Mutation / generation sites (client)

Anchors in `vendor/fr-client-rust/crates/client/src/client/client.rs` unless noted:

| Site | Effect | Bumps iface/inv/scene? |
|------|--------|------------------------|
| `bump_gens` | maps IF_* / UPDATE_INV_* / zone/loc packets | yes for listed opcodes |
| `handle_tab_clicks` | sets `active_icon` | **no** |
| scrollbar handler ~7940–7971 | mutates `scroll_pos` | **no** |
| inv drag drop ~4557+ | mutates `link_obj_*` | **no** inv gen |
| `clientComponent` ~4881+ | mutates friend/ignore text, button_type, etc. | **no** (called from draw) |
| `TUT_OPEN` / `TUT_FLASH` | `tut_com_id` / may bounce `active_icon` | **no** in `bump_gens` |
| cold login success | resets tabs/modals | no gens until later packets |
| `render/draw.rs` scroll clamp / layer_scroll writeback | `scroll_pos` | **no** |
| zone / LOC_* / rebuild | scene + world stamps | scene / model_stamp |

## Exact counterexamples (share under equal gate stamp)

Notation: owners O₁ = host-play observe shell, O₂ = host after_drain shell
(same slot/client). `FamilyPub` holds O₁’s Arc after O₁ rebuild.

### CX1 — Side tab click between observe and after_drain (Widgets + SideTabs)

1. Client at `gens.iface=I`, `gens.inv=V`, `active_icon=3`.
2. O₁ rebuilds Widgets/SideTabs: gates move to (I,V); body has side root for
   tab 3 and `SideTabView.active` true only on index 3. Publish stamp `(I,V)`.
3. Same frame mainloop: `handle_tab_clicks` sets `active_icon=5` (no gen bump).
4. O₂ rebuilds with client still `(I,V)`. Gate says moved if O₂ was behind;
   **gen-only share** installs O₁ Arc.
5. Independent walk for O₂ would use `active_icon=5`: different `widget_roots`
   side root, different `active`/`visible` on side_tabs.

**Failure mode:** shared install makes previously “would-be-fresh-on-walk”
content match O₁’s older tab selection. Broadening O₂’s **rebuild gate** to
`active_icon` would fix share but would also rebuild on every tab click for
owners already at (I,V) — **forbidden silent broadening of stale behavior**.
Share stamp may include `active_icon` only if gates stay iface+inv; then O₂
with gate already at (I,V) still skips rebuild (stale OK), while O₂ with gate
behind must not share when `active_icon` differs — that needs the extra stamp
field. Tab click is only one of several non-gen fields (below).

### CX2 — Scroll without gen (Widgets / nested side widgets)

1. O₁ rebuilds at (I,V); `WidgetView.scroll_position` = S₀ for layer L.
2. User drags scrollbar → `scroll_pos` = S₁; **no** iface/inv bump.
3. O₂ first rebuild at (I,V) shares O₁ body → still S₀.
4. Independent walk reads S₁.

Same class: draw-time scroll clamp / drag-autoscroll writebacks.

### CX3 — Local inv drag without inv gen (Widgets TYPE_INV items)

1. O₁ rebuilds inventory widget items from `link_obj_*` at (I,V).
2. Local drag rearranges slots in `ifaces_mut` (no `gens.inv`++).
3. O₂ share at (I,V) keeps pre-drag items; independent walk sees post-drag
   layout until UPDATE_INV_* arrives.

### CX4 — `clientComponent` text without gen (Widgets)

1. O₁ rebuilds friends-list widget text at (I,V).
2. Draw calls `clientComponent`, rewrites `com.text` / `button_type` from
   `friend_server_status` / friend tables (no gen).
3. O₂ share keeps O₁ strings; independent walk clones new text.

### CX5 — `TUT_OPEN` without iface gen (Widgets roots)

1. O₁ rebuilds with `tut_com_id=-1` at iface I.
2. `TUT_OPEN` sets `tut_com_id` (not in `bump_gens` iface list).
3. O₂ share omits tutorial root; independent walk includes it.

(Existing single-owner gate already stays stale until some other iface bump;
cross-owner first install share still diverges from a walk that runs because
the receiving owner’s gate was not yet at I.)

### CX6 — Loc distance vs player move (Loc)

1. O₁ `rebuild_loc` at `scene=S`, `loc_model_stamp=M`, player tile T₀ → distances d(T₀).
2. Player walks to T₁: player/route update, **scene and model stamp unchanged**.
3. O₂ first loc rebuild: gates dirty relative to empty/default; gen+stamp match
   pub `(S,M)` → share d(T₀). Independent walk computes d(T₁).

**Note:** owners already synced at (S,M) correctly keep stale distances today;
share must not change that. The defect is **install from registry when the
receiving owner’s walk would still run** under equal scene/model stamp but
different baked distance context.

## Why “just extend the stamp” is not proven here

A safe share stamp must include **every** input that can change view bytes
without the owner rebuild gate. Minimum extras already identified:

- Widgets/SideTabs: `active_icon`, full `side_icon[]`, modal/overlay/tut roots,
  **all** `scroll_pos` (and any other overlay fields `widget_view` reads that
  local/draw paths write), **all** `link_obj_*` (or inv content hash), friend
  overlay text fields / whatever `clientComponent` writes, any future local
  iface mutation.
- Loc: local player tile (or stop baking distance into the shared body — API
  shape change, out of scope).

That is effectively **input-content hashing of the iface overlay + loc
context** on every candidate install. Costs approach a walk; omission risk is
high; it is not the bounded “iface+inv / scene+model_stamp” provenance the
proposal hoped to prove. Debug-only deep `PartialEq` after dual rebuild does
not make production share safe.

**Gate sync on share** (proposal) cannot repair CX1–CX6: syncing gates to
(I,V) or (S,M) after installing the wrong Arc makes the next quiet consult
**skip** a walk that still needed different content.

**Replace-only Arc bodies** and quiet-frame no-churn rules are moot if share
is not shipped.

## Mechanism status

| Item | Status |
|------|--------|
| Per-slot `FamilyPub` Arc widgets/side_tabs/loc | **Not implemented** |
| Receiving-owner gate sync on share | N/A |
| Replace-only bodies | N/A |
| Cross-client / global intern | Not introduced |
| Deep compare on every read | Not introduced |
| Single mutable snapshot | Not introduced |
| Consumed-buffer pool fallback (§4B) | **Not implemented** (brief: no fallback this card) |

Public accessors, serde shape, guardian/observe ordering, tick-edge
publication, Stop ownership: **untouched**.

## Tests

No new regression tests landed: no mechanism to prove. Existing snapshot /
host / host-play / panel suites were not re-run as a green-wash of a no-op
code path; this card is audit+stop only.

If a future card revisits sharing, required tests remain those in the
proposal oracle (equal-stamp ptr_eq, changed stamp no share, retained epoch
isolation, teardown, gate stay/move, serde shape) **plus** explicit
reproductions of CX1–CX6 under any extended stamp.

## Allocation vs RSS (limits only — no measurement claim)

From proposal / attribution context (perturbed malloc, not RSS):

| Bound | Note |
|-------|------|
| Upper removed **allocation** if safe share existed | ~(k−1)(W+L) while stamps match; TUI k≈2 → ~½ of process WidgetView+LocView stacks; panel k≈3 → ~⅔ |
| This card | **0** removed — mechanism rejected |
| RSS | **No claim**; allocator retention ≠ RSS |

## Identity / registry lifetime (design note only)

Had share shipped: slot-private `FamilyPub`, drop with slot Stop; no process
global intern; at most one published blob per family plus older Arcs still
held by lagging owners (bounded by live shells × short epoch skew); no
generation ring buffer. **Not applicable** after stop.

## Files

| Path | Action |
|------|--------|
| `docs/memory/snapshot-family-sharing-report.md` | this report |
| crates / client | unchanged |

## Follow-on (root owns)

- Do not schedule Arc family share without a new design that either (1) proves
  a complete stamp with tests for CX1–CX6, or (2) changes publication so
  non-gen inputs cannot diverge between co-living shells without a gate bit
  (e.g. local input gen) **without** quietly refreshing today’s stale views.
- Independent §4B consumed-buffer pool remains a separate candidate when
  authorized.
- Matched live allocation/RSS screens and whole-branch review unchanged and
  out of this card.

## Summary

Provenance audit of Widgets/SideTabs/Loc builders and client mutation sites
**fails** the share-safety gate. Exact counterexamples: tab click, scroll,
local inv drag, draw-time `clientComponent` text, tutorial open without iface
gen, and loc distance after player move under equal gate stamps. **Stop.** No
implementation, no fallback, no gameplay change.
