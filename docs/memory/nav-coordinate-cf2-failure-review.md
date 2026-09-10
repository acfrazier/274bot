# Coordinate CF2 feasibility — failed admission evidence review

**Verdict: CONFIRM_FAILURE_PARK**

Reviewer profile: `reviewer` (Grok 4.5). Independent stream-hash and ledger
recompute from the local untracked failed CF2 raw archive. No production,
method, source, test, cap, or STATE edits. No native/SSH/live/private-pack work
outside the provided artifact. No new run. Prior CF1 review `9ff38d7` is accepted
and native prerequisites are not redone.

## Scope and frozen base

| Item | Value |
| --- | --- |
| Workspace | `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f` |
| Branch | `codex/memory-diagnostics` |
| Reviewed commit | `a1b2d329448c01df05c6343f2864650e01c4e263` |
| Evidence archive | `diagnostics/nav-stage-a-native-preparation/coordinate-cf2-result-01/root-coordinate-cf2-evidence.tar.gz` |
| Archive SHA256 | `45bed027638f73fc9c24c44310a212941174cbb159417669a9e7ce7b9156cc14` (887779 B, 316 members) |
| CF2 auth SHA256 | `1df23854e84d11decbdda2332b91623f4ed427b201dfdcd11b1d7513bf3c0a4a` |
| CF1 root auth | `62835390bdcbee419029f6c0fcf38f1126a502cde926be643de0c64acd9977ec` |
| CF1 parent result | `3796341a06095483d835e048a066a8853db43c559e5bf1f3125eb5ea23eeb054` (complete/4) |
| Prior reviews | CF1 `9ff38d7` / `27151fb` accepted |
| Explicitly **not** approved | resume remaining 57, reset/uncharge, cap expand 122/1800/1500, CA, performance win, complete 59-row feasibility, Stage B, live, final branch review |

Owned deliverables: this report and
`diagnostics/nav-stage-a-native-preparation/coordinate-cf2-failure-review/`.

## Independent checks (all passed)

Probe: `diagnostics/nav-stage-a-native-preparation/coordinate-cf2-failure-review/independent_cf2_failure_review.py`
→ `verdict.json` / `checks.json`.

1. Stream-hashed all **316** tar members against the manifest; archive SHA and
   byte size match the frozen failure report.
2. Failure/claim/checkpoint bindings: `status=failed`, `phase=CF2`,
   `completed=57`, `error=ValueError('native idle/steal admission')`; claim
   `claimed`; checkpoint `validated` @ 57; claim/failure authorization refs bind
   CF2 auth `1df23854…`; root ref is immutable CF1 auth `62835390…`.
3. **No** `CF2/result.json` and no continuation token in the archive.
4. Immutable CF1 parent: complete/4, budget wall `81.74507052099216` /
   cpu `60.938457` / children 8; parent result SHA matches authorization.
5. CF2 authorization: ceilings **122 / 1800 / 1500**, decision
   `6e53a133…` / commit `b00ba2c5…`, `review_approved=true`, phase CF2;
   prep auth byte-equals admission copy.
6. Pack `input.bin` SHA `2f393138…`, 73438581 bytes; all **59** selectors match
   space-normalized ORIGINAL_TSV `49e348ea…` and `contract.shard_sha256`.
7. All **61** fresh children (CF1 four + CF2 fifty-seven): exit 0, address guard
   active, limits unchanged (wall=120,cpu=90,rss=1GiB,address=4GiB,output/file=256KiB),
   9-phase order, `raw_schema=stage-a-raw-v1`, `raw_order=sweep-row-lane/8/3`,
   24 nonnegative raw samples, narrow=0, per-row aggregates agree across arms.
8. Coverage: rows **1–30** both dense/refined; row **31** dense only (refined
   unmatched); rows 32–59 not run. No complete 59-row feasibility projection.
9. Cumulative ledger recomputed: **65** children (original F1 4 + CF1 4 + CF2 57),
   wall **478.864614687991**, cpu **364.395005**; under ceilings; no pending
   reservation; `unknown_cpu_charge=0`.
10. CF2-only cost wall **397.1195441669988** / cpu **303.456548**; partial fresh
    from original F1 wall **438.7741664569912** / cpu **334.893665**.
11. CPU component identity with publication_reserved: **−25 µs** (within 1 ms);
    waited surplus over CF2 probe `child_cpu`: **1.142900032 s**, retained in
    `waited_children_cpu` / spent gauge — fully charged, nothing silently discarded.
12. Root `root-failure-audit.json`: `qualified=false`,
    `acceptance_projection=null`, members 316, cross-check deltas match.
13. Post-failure process inventory clean; scope states it cannot reconstruct the
    failed idle delta — **not** past-CPU proof.

## Failure boundary and classification

### Where it stopped

Protocol loop (from `sharded.py`): each child does `bind()` → `storage_guard` →
`budget.reserve()` → launch. `native_preflight(auth)` runs inside `bind()` and
raises `ValueError('native idle/steal admission')` when the fixed 1.0 s
`/proc/stat` delta has `idle/elapsed < 0.90` **or** steal ticks change
(`after[7] != before[7]`), or elapsed ≤ 0.

After **57** validated CF2 children, the **pre-next-child bind** failed that
guard **before reserve**. Checkpoint remains `validated` @ 57 with
`reserved_*=0`. No child probe failed at this boundary.

### What the evidence cannot say

The failure record does **not** retain the exact failed `/proc/stat` counter
delta. Therefore this review **does not distinguish** idle-below-90% from
nonzero steal, and does **not** invent noisy-neighbor or host-contention
attribution. A clean post-failure process list only shows no matching
measurement/build processes afterward; it does not prove past CPU state.

### Honest failure class

`native_preflight_idle_or_steal_reject_mid_cf2` — mid-phase native admission
reject under the sealed idle/steal contract; partial charged evidence only;
`qualified=false`.

## Accounting summary

| Layer | Children | Wall (s) | CPU (s) |
| --- | --- | --- | --- |
| Original F1 (prior ledger) | 4 | 40.090448… | 29.50134 |
| After CF1 (parent result) | 8 | 81.745070… | 60.938457 |
| CF2-only spent | +57 | 397.119544… | 303.456548 |
| **Cumulative at failure** | **65** | **478.864614…** | **364.395005** |
| Arithmetic remainder to 122/1800/1500 | 57 | ~1321.14 | ~1135.60 |
| Unrun CF2 schedule slots | 57 of 114 | — | — |

`budget.stopped=false` is a spent-gauge property only. It is **not** a successful
continuation token and does **not** establish resumability. Remaining children /
wall / CPU figures are arithmetic leftovers under the sealed ceilings, not
authorization to finish the phase.

Publication reserved remains **0.2** CPU / **2** wall (no successful CF2 final
publication tail). Waited surplus 1.1429 s stays inside the spent CPU gauge.

## Protocol-authorized next boundary

**Park / review only.**

Not authorized by this failed result (and not invented here):

- Resume the remaining 57 CF2 children on this claim
- Reset budget or uncharge original F1 / CF1 / CF2 spent work
- Expand 122 child / 1800 wall / 1500 CPU caps
- CA authorization or acceptance pathway
- Performance win or completed feasibility from partials
- New CF2 run treated as continuation of this failure token
- Idle-vs-steal root cause without new instrumentation and its own review

Bounded future investigation (separate cards only, not this evidence):

- Optional method change to retain the failing `/proc/stat` delta on reject —
  requires its own review before any rerun
- Host contention study only under a **new** authorization, not as continuation
  of this incomplete claim

## Verdict

**CONFIRM_FAILURE_PARK** at `a1b2d32`. Failed CF2 partial evidence is consistent:
316-member archive integrity, immutable CF1 parent and CF2 auth, 57 exit-0
children with native hard AS/caps/raw24, cumulative 65/478.86w/364.40c fully
charged including original four and CF1 four, `qualified=false`, no success
result or continuation token. Next admitted action is park/review. Do not treat
partials as a performance win or completed feasibility.
