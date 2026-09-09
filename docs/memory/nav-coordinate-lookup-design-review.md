# Review: reuse validated coordinates in tiled collision reads

**Card:** `t_96a6b960`
**Reviewed document:** `docs/memory/nav-coordinate-lookup-design.md` at `aa2d740ba86abf28f539f619a7b9cdc1fbb6fca8`
**Source anchor:** frozen tiled `8385babb23fd15b876506d4a3f6154984a6b2df1` `crates/nav/src/collision.rs` (blob `7446a044…`; identical on current `HEAD`)
**Branch at review:** `codex/memory-diagnostics`
**Lens:** artifact (round 1) — cold design + frozen source comparison only
**Reviewer model/provider (actual):** `grok-4.5` / `xai-oauth` (profile `reviewer` defaults; no task model/provider override)
**Outcome:** **APPROVED** — bounded design only

This approval does not admit a new candidate, change Stage A tooling bindings
(29b/8385), choose F1-vs-refinement sequencing, waive gates/measurements, or
claim a CPU/latency win. Final whole-branch Grok 4.6 remains independent later.

---

## Scope checked

- Design: `docs/memory/nav-coordinate-lookup-design.md` (sole file in `aa2d740`)
- Frozen path: `8385babb:crates/nav/src/collision.rs` lines 328–551
  (`packed_pair_at`, `pair_at_index`, `pair_at_index_or_panic`,
  `walkable_word`, `walkable`, `standable`) plus constructor plane strip and
  directory sizing
- Oracle note: `crates/nav/src/collision_tiled_oracle.rs` `assert_all_cells`
  (present-plane skip for short synthetics)
- AGENTS.md / `docs/execution.md` (design review before architecture
  implementation; design approval ≠ measured win)
- No source/tool edits, builds, probes, SSH, live, cache, server, remotes,
  STATE, or delegation by reviewer

---

## 1. Source-path claim (independent)

At `8385babb`, the three coordinate APIs that use packed storage do:

1. `plane(level)` → reject unknown levels (`None` → 0/false)
2. `lx = x - origin.x`, `lz = z - origin.z` → reject negatives and
   `lx >= width` / `lz >= height`
3. `idx = plane * plane_cells() + lz * width + lx`
4. packed branch: `pair_at_index_or_panic(idx)`

`pair_at_index_or_panic` → `packed_pair_at` (reject `index >= logical_cells`
with panic message
`collision index {index} past logical length {logical_cells}`) →
`pair_at_index`, which **re-derives** `plane`, `lz`, `lx` via

```text
plane_cells = width * height
plane = index / plane_cells
within = index % plane_cells
lz = within / width
lx = within % width
```

then tile-local arithmetic and directory/dense-pool read.

Cited spans match exactly: public pair API 330–336; private index path
385–417; coordinate APIs 474–549. HEAD `collision.rs` blob equals the frozen
blob (`7446a044…`).

**Verdict on redundancy:** demonstrable **source-level** reverse of already
validated coordinates. Design correctly labels this a hypothesis, not
executed-instruction or measured-performance proof. Optimized codegen may
fold some work; inspect/measure later, do not treat this review as a win.

---

## 2. Proposed shape vs invariants

| Invariant | Frozen behavior | Design requirement | Review |
|---|---|---|---|
| Helper scope | directory/dense read after coords known | private inline helper takes validated `(plane, lx, lz)`; keep descriptor + local-cell bit math + checked slice access | OK |
| Linear index API | `packed_pair_at` / `pair_at_index` public contract | still decompose index, then same helper | OK |
| Flat index expression | `plane * plane_cells + lz * width + lx` | keep same expression; no saturating arithmetic | OK — preserves overflow wrap of existing usize math |
| Logical length / short plane | geometric in-bounds level on absent upper plane panics via `pair_at_index_or_panic` | form idx, check `logical_cells`, same panic message; do not treat as uniform/open | OK — critical: directory only spans `planes = logical_cells / plane_cells`; skipping the check would change panic class to directory OOB |
| `walkable_word` vs flags | always packed path; ignores raw flags | must still ignore raw flags | OK |
| `walkable` / `standable` packed | `WALK_BLOCK` vs `!blk` / face ignored for stand | distinct masks unchanged | OK |
| Raw flags branches | after bounds, `flags[idx] & mask` with **no** logical-length gate | keep existing buffer index/failure order | OK — do not add logical check onto flags path |
| Plane selection | `plane(level)` is absolute `0..=3`, not `level - origin.level` | do not subtract origin.level | OK |
| Non-goals | router/layout/decoder/ownership/API/unsafe/cache/deps | explicitly forbidden | OK |
| Ownership of later impl | — | only `collision.rs` + narrow nav tests/report; new commit/provenance; never relabel frozen 8385 exe | OK |
| Stage A / F1 | tooling binds 29b/8385 | unchanged; F1 evidence before refine-vs-continue decision; design approval makes neither choice | OK |

No router, directory layout, constructor/decoder, allocation model, public API,
or ownership-boundary change is proposed. Same tiled representation only.

---

## 3. Tests / falsification adequacy

Design requires later implementation to:

- compare all three coordinate APIs to frozen 8385 behavior and the logical
  packed-pair oracle
- cover 256 faces × both blocked values; uniform/dense; 31/32/33 and
  non-square partial tiles; negative/nonzero origins; 1–4 planes; unknown
  levels; **missing synthetic planes**; absent/present/short raw flags
- preserve wire/blocked padding equivalence and existing nav route/pack/paint
  regressions
- include **exact panic cases** (message/path), not only success cells
- bounded clean release-build read/route comparison for the CPU hypothesis
  with identical checksums; assembly change ≠ accepted performance win
- layout/allocation equality (no new collision storage or read allocation)
- full-byte differential + all 59 selectors remain qualification basis for any
  later native candidate; new commit needs fresh freeze, tool binding review,
  build, and admission — current Stage A worker/manifest stay on 29b/8385

**Gap this design correctly closes:**
`collision_tiled_oracle.rs` `assert_all_cells` **skips** levels
`>= present_planes` because short synthetics panic on both arms. Success-only
oracle coverage is therefore insufficient for the short-plane contract; explicit
`should_panic` / catch_unwind cases against the **tiled** message
(`collision index … past logical length …`) are required. Dense oracle
`.expect("dense oracle geometric index")` must not be used as the panic-string
oracle.

No material test-plan omission found for a design of this width. Implementation
must not weaken panic ordering (bounds first, then logical-length panic on
packed path) or fold raw-flags OOB into the packed panic path.

---

## 4. Decision boundary (non-waiver)

Approved as a **possible later refinement** only:

- Current F1 / Stage A path and frozen 8385 admissions proceed unchanged.
- Root still reviews F1 evidence before choosing refine-first vs finish-8385.
- If opt/measure shows no useful benefit, park rather than expand into search
  optimization.
- Any later acceptance still needs approved CPU/latency/peak/noise gates, cold
  behavior consistent with the accepted startup tradeoff, and Stage
  B/resident/lifecycle/absolute/final-review work.
- Collision storage removal remains shared-per-world, not per-bot. No savings
  claim from this inspection.

---

## Verdict

**Approve** the bounded design in `aa2d740` / `nav-coordinate-lookup-design.md`.

Independent checks: frozen coordinate→index→recompose→tile path matches the
document; short-plane panic and raw-flags semantics can stay exact under the
stated helper shape; non-goals and admission/tooling constraints are explicit;
falsification plan is adequate and stronger than current short-plane test skips.

**Not approved by this card:** implementation, candidate admission, tool rebind,
measurement waiver, performance claim, or F1 sequencing choice.
