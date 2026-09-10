# Coordinate CF1 raw feasibility and cumulative ledger review

**Verdict: APPROVE** (CF1 feasibility evidence + ledger only)

Reviewer profile: `reviewer` (Grok 4.5). Independent stream-hash and budget
recompute from local untracked raw archive. No production/test/cap/STATE edits.
No native/SSH/live/private-pack work outside the provided artifact. CF2 is
budget-admissible under the sealed contract but is **not** released or run.

## Scope and frozen base

| Item | Value |
| --- | --- |
| Workspace | `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f` |
| Branch | `codex/memory-diagnostics` |
| Reviewed commit | `27151fb2bc5356159e51a2ecda3a0228ca0e22ac` |
| Evidence archive | `diagnostics/nav-stage-a-native-preparation/coordinate-cf1-result-01/root-coordinate-cf1-evidence.tar.gz` |
| Archive SHA256 | `6b6314fe9e60235ab92feeb65f972921104c66bccbc5fabcf385e4e28f576cdf` (834734 B, 82 members) |
| Auth SHA256 | `62835390bdcbee419029f6c0fcf38f1126a502cde926be643de0c64acd9977ec` |
| Prior reviews (not redone) | `b62f173` / `f13d682` (exact59 + native generated) |
| Explicitly **not** approved | CF2/CA release, CPU/p99/RSS acceptance, Stage B, live actions, performance win claims, final branch review |

Owned deliverables: this report and
`diagnostics/nav-stage-a-native-preparation/coordinate-cf1-review/`.

## Independent checks (all passed)

Probe: `diagnostics/nav-stage-a-native-preparation/coordinate-cf1-review/independent_cf1_review.py`
→ `verdict.json` / `checks.json`.

1. Stream-hashed all **82** tar members against the manifest; archive SHA and
   byte size match the frozen report.
2. Immutable auth/claim/checkpoint/result binding under remote path prefix
   `nav-coordinate-113ce05-concord-01`; result `status=complete`, `phase=CF1`,
   `completed=4`, schema `stage-a-coordinate-v1`. Preparation
   `CF1-authorization.json` byte-equals admission copy and result auth ref.
3. Pack `input.bin` SHA `2f393138…`, 73438581 bytes = ORIGINAL_PACK.
4. All **59** selectors match space-normalized integer rows from original routes
   TSV SHA `49e348ea…`. Tab-join reconstruction mismatches archived selectors
   (confirms root audit-normalization correction; no native rerun).
5. Four children in schedule order dense/refined/refined/dense for rows 1–2:
   record SHA binds; receipts `returncode=0`, `failure=None`,
   `address_guard_active=True`; limits unchanged
   (wall=120,cpu=90,rss=1GiB,address=4GiB,output/file=256KiB).
6. Each `.out`: 9 NDJSON lines; summary `raw_schema=stage-a-raw-v1`,
   `raw_order=sweep-row-lane/8/3`, 24 nonnegative `raw_elapsed_ns`, p99 and
   3-lane CPU consistent; `narrow_allocations=narrow_requested_bytes=0`;
   `input_bytes=73438581`, `logical_cells=65142784`; per-row aggregates agree
   across arms.
7. Source binding `7b101c30…` present on result/auth artifacts.
8. Prior campaign ledger field-exact vs
   `coordinate-cf1-preparation-01/root-prior-ledger-verification.json` and
   `sharded.PRIOR_F1_LEDGER` (4 children, wall 40.090448…, cpu 29.50134, …).
9. Decision `6e53a133…` / commit `b00ba2c5…`; ceilings 122 child / 1800 wall /
   1500 CPU / fresh 118.
10. Budget quantities recomputed: fresh wall `41.6546222899924`, fresh cpu
    `31.437117`; cumulative wall `81.74507052099216`, cpu `60.938457`,
    children `8`; `stopped=false`, unknown=0, no pending reservations.

## Accounting semantics (waited surplus and CPU components)

### Waited-child total vs sum of four probe `child_cpu`

| Quantity | Value |
| --- | --- |
| Fresh `waited_children_cpu` | 16.616442825 s |
| Sum of four per-probe `child_cpu` | 16.495743 s |
| Difference | **0.120699825 s** |

`Budget.waited_children_cpu` is `prior.waited + max(0, total_cpu_delta − own_process_time)`
where `total_cpu = RUSAGE_SELF + RUSAGE_CHILDREN`. Per-probe `child_cpu` is only
the `RUSAGE_CHILDREN` delta strictly around each `launch()`.

**Fully charged; nothing silently discarded.** The 0.1207 s surplus remains
inside `waited_children_cpu` and therefore inside the spent `cpu` gauge. It is
not required to equal the four probe records: other reaped child work (helpers /
bookkeeping between probes) and minor `process_time` vs `getrusage(SELF)`
bucketization can land outside the four instrumented windows. This is not a
leak and not a reason to block CF1 feasibility.

### CPU component identity (~16 µs)

Meaningful invariant:

`cpu ≈ supervisor_cpu + waited_children_cpu + unknown_cpu_charge + publication_reserved_cpu`

| Identity | Delta |
| --- | --- |
| With publication_reserved (0.2) | **−16 µs** |
| Without publication_reserved | +0.199984 s (= publication_reserved_cpu) |

Sealed prior F1 already embedded `publication_reserved_cpu=0.1` inside its
`cpu` total; CF1 carries `0.2`. Root audit’s inclusion of publication_reserved
in the component check is correct. Exact equality across separately sampled
`process_time` / `total_cpu` clocks is not a production invariant; 1 ms audit
tolerance changes no ceiling.

Do **not** treat broad `qualified: true` as the review verdict; the invariants
above are the acceptance surface.

## RSS / routes (observation only)

| Arm | Peak RSS |
| --- | --- |
| dense (both rows) | 149553152 B |
| refined (both rows) | 153616384 B |

Refined is slightly higher on these two routes. Not a memory win; two initial
routes are insufficient for 59-route acceptance or performance claims.

## CF2 expansion admissibility (not released)

After 8 cumulative children under accepted 122/1800/1500:

| Remaining | Value |
| --- | --- |
| Children | 114 (exactly CF2’s ceiling) |
| Wall | ~1718.25 s |
| CPU | ~1439.06 s |
| Stop / pending reservation | none |

**CF2 is admissible under the budget contract** (slots and cumulative ceilings
still open). This review does **not** authorize, claim, or run CF2, and does not
project that CF2 will finish inside the remaining wall/CPU. Expansion remains
root-owned after this verdict.

## Diagnostic errors preserved

Root `audit-normalization-correction.txt` retained: tab-vs-space selector audit
bug (corrected in audit only); exact CPU component equality invalid (~16 µs
sampling). Independent review agrees both are audit-only; production budgets and
native bytes unchanged.

## Verdict

**APPROVE** coordinate CF1 native feasibility evidence and cumulative ledger at
`27151fb`. CF1 four-child exit-0 raw evidence is consistent; ledger fully
charges supervisor/waited/publication components; CF2 may be considered by root
under remaining 114/1718/1439 but is not released here.
