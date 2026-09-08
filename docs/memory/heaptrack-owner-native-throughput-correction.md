# Native replay last-event throughput correction

Task `t_b03522bc`, `codex/memory-diagnostics`. Local macOS diagnostic correction;
submitted for same-card Grok 4.5 review, not Linux readiness or production release.
Root retains STATE, prior failed evidence and final whole-branch Grok 4.6.

## Decision and measured cause

Retain one minimal correction for review: `interpreted()` stores last-event
`(line, offset, previous_timestamp_ms)` as an optional fixed scalar tuple and
constructs its JSON object once at the existing return boundary. Previously it
allocated and destroyed that object for every allocation/free in both interpreted
passes. No peak, mark, checkpoint-map, parser, hash, allocator or guard change.

The controlled single-change comparison supports this path as a dominant cost
on the two churn fixtures: phase 2 CPU falls 60.86% / 58.36%, and phase 3 CPU falls
57.93% / 52.89%. This is substantially more than the failed Linux gate's 1.927%
shortfall. It is an isolated code-path ablation, not an independent CPU sampling
profile: it measures the combined JSON construction/destruction and associated
charged-allocation work, not the allocator's separate fraction. Numeric parsing,
hashing and guards still run. Raw-pass CPU is approximately unchanged; the
output-heavy diversity case barely improves phase 3. That distinction supports
the churn attribution rather than a claim of universal replay acceleration.

No allocation counts or RSS savings are claimed. Remaining peak/mark JSON work
and checkpoint-map scanning are source leads only and deliberately left alone.
No second candidate or repeated timing-until-green was attempted.

## Preserved rejected baseline

Baseline native source equals frozen `8b4d6f5a3686e85e5fed55574b205936d4be45a1`;
starting branch HEAD was documentation-only `bc3f775`. Audit checks every frozen
native source/manifest/lock/generator hash against that commit. The parent
`t_c07bdca6` approved the accuracy of the rejected-qualification report, not the
qualification. Original corrected Linux result remains failed: 16 core and 36
native/Python tests passed, then only two of nine timing cases ran. Wide250k
phase3 median 4,416,362.4607893815 B/CPU-s missed 4,503,128. Its raw SHA remains
`b7a2af38b7adb1b0f2e2c0d611a53ac2fa168bbf3b1d6e952bf33f6d0db47e83` as supplied
in the reviewed parent handoff. No old failure evidence was edited or rerun.

## Predeclared finite comparison

Evidence root: `diagnostics/native-throughput-correction/`.
`predeclared.md` was written before baseline timings and any native edit.
`compare.py` freezes binaries and source hashes before each role, generates only
three fixed cases using unchanged `qualify.generated`, and runs three fresh
owned children per case, serially. The roles ran baseline then candidate, nine
cells each, no discarded or replacement timing runs. Every input <=2,000,000
bytes; cases `(1850048, False)`, `(1850048, True)`, `(1000, 'diversity')`.
Actual stream sizes, record counts and fixture hashes are in each results JSON.

Each request uses `outputs=True`, CPU30/wall60 lower-only limits, existing
`qualify.owned()`, unchanged native guards and the same fixture executable
architecture. Three phase CPU/wall measurements include phase-3 canonical
comparison, classification, lossless output and adapter output readback. The
final adapter JSON send is outside phase3 as in the unchanged qualification
architecture, while owned-wrapper elapsed time includes process exit/readback.
No calls to qualification `main`, no changed thresholds/cases/repetitions/clocks,
no Linux/SSH/Windows/Concord build, new capture, interpreter or production prefix.

Local host: macOS15.7.9 arm64; Python3.9.6; rustc1.98.0
`88d9e12ae178fab0fb5cc050a94da85685d449ea`, aarch64-apple-darwin, LLVM22.1.8.
Builds are release/offline/locked, outside timing. Background OS and agent work
are uncontrolled. Pre-role process snapshots contain no cargo/rustc/native
replay/panel/tui-play matches; this is not continuous exclusivity proof. Warm
small fixtures and millisecond CPU cells do not model cold I/O, large live maps,
production symbol populations or sustained native sampling.

Predeclared retention diagnostic: >=10% lower CPU in both interpreted phases on
both churn cases with exact compared semantics. `recompute.py baseline baseline`
fails this gate as intended (`retention-red.log`); actual candidate comparison
passes. This gate is not a replacement for the unchanged Linux qualification.

| Case | Baseline phase1/2/3 median CPU seconds | Candidate phase1/2/3 median CPU seconds | Phase2/3 reduction |
|---|---|---|---|
| Short churn | .028509 / .062281 / .065776 | .027928 / .024375 / .027670 | 60.86% / 57.93% |
| Wide churn | .013143 / .019583 / .021881 | .013202 / .008154 / .010308 | 58.36% / 52.89% |
| 1000-entry diversity | .000344 / .000874 / .014851 | .000339 / .000722 / .014485 | 17.39% / 2.46% |

Median sum-of-three-phase CPU per actual repetition (not sum of medians):
short .157650 -> .080172; wide .054779 -> .031598; diversity .016069 -> .015546.
`arithmetic.json` retains full precision. No throughput/capacity extrapolation.

Exact fixture executable SHA256:

- baseline `7471872766bd57c1a11c0d8e76116f96760ca9dc7343502a301b909d475011c8`
- candidate `84c8fff64f610aa50c8637611eb77324e64deb7897a9e93208ad18a4c082122d`

Exact production-child SHA256 (built, not used for timing cells):

- baseline `81244acc916bdecef0a18189cb63fa90045d9ea33ca8879b4b5e0c613ae57156`
- candidate `85b4175f5ca18ac7fb98361d160344d7936e3f4e5f14a7a01b5580693a171b1e`

Candidate replay.rs SHA256:
`ee6efba6427028e202c3130d02f22f4d405a28f4888ce2db5dbff296e84a6218`.
Full frozen source/lock/compiler/executable identities live in each role's
`freeze.json`; its deliberately pre-run `status:incomplete` is not the result.
Each `results.json` reports `comparison_passed`, not native qualification.

## Semantics and resource invariants

Every timed result compares raw, first, snapshots, canonical and canonical_equal
against Python. Every output filename/content is compared, exact TSV text and
structured JSON, excluding only resources and tables.high_charged_bytes as in
the existing differential adapter. Candidate also compares the entire normalized
result with its baseline repetition. No event/definition/hash/cutoff field is
excluded. All 18 comparisons pass for these declared fixtures.

The optional tuple starts None, changes only after an accepted allocation/free,
retains the previous actual timestamp and start-of-record offset, and is never
reset by marks or metadata. Its final JSON has identical unsigned integer fields,
key order and nullability. Both interpreted passes still compute/compare full
results; the last_event object remains inside phase2/3 and charged at conversion.
The tuple is fixed stack storage, not an uncharged growable table. All heap
allocations still go through Charged, including the final object and serialization.
Transient old+new realloc charging and container metadata accounting are unchanged.

Guard pulses/check locations, numeric/definition validation, strict first peak,
request bracketing, repeated-mark ordinals, full byte/event/metadata/trace hashes,
raw-one/interpreted-two streams, snapshot construction and output are unchanged.
No lowered validation, phase3 exclusion, allocator bypass, cap increase, dependency,
parallelism, architecture or public-runtime change is in this patch.

## Executed regression coverage and failures

Commands (all from campaign checkout; tests after timing, serial commands):

    cargo build --release --offline --locked --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml --bins
    cargo test --release --offline --locked --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml --bin heaptrack-owner-child -- --test-threads=1
    python3 -B -m unittest discover -s docs/memory/heaptrack-owner-native/tests -v
    python3 -B -m unittest discover -s docs/memory -p test_heaptrack_owner_replay.py -v
    cargo fmt --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml -- --check
    git diff --check

Results: 16/16 core pass, 1.11s; native/Python 39 run, 38 pass and one explicit
Linux AS skip, 9.452s (original 36 plus three new regressions); original unchanged
21 run, 19 pass and two Linux AS/proc skips, 6.135s. Release builds, formatting
and source diff checks pass; existing dead-code build warnings remain. Staging
the unedited raw logs makes the all-file whitespace check report the core test
log's original blank line at EOF; that evidence byte is intentionally retained.
Portable native
RSS behavior and simulated resource samples do not prove Linux /proc enforcement.

`audit.py` verifies all21 mapping keys and every mapped Python/core target against
actual discovered tests and successful core log entries. Existing tests cover
first strict peak including a later equal peak (`three_pass_baseline_and_first_peak`
line9 assertion), requested-between-marks and selected second-pass snapshots,
duplicate timestamp ordinals, invalid request bounds, real event tail, mutation
between passes and [1,2,1] actual stream opens. Full mapped grammar, numeric,
identity, canonical, graph, ownership, hashing, charged growth, collision, depth,
phase-end/inner-loop, protocol, kill/reap and lower-cap coverage remains exercised.

Three added regressions cover no events with an unused descriptor (last_event
null and zero peak), free-event position surviving later marks/comments/suppression
metadata with full output and requested checkpoint comparison, and malformed
unterminated tail rejection after a last event. Both first/second-pass equivalence
and output are exercised rather than testing tuple construction alone.

Important newly discovered existing discrepancy, NOT fixed or normalized away:
with no allocation events AND no descriptor definitions, Python omits `a` from
first.definitions but baseline/candidate native emit `a:0`. The first new no-events
fixture caught this before the code edit (`event-baseline.log`). `audit.py`
reproduces it on both frozen executables and retains the exact three definitions
objects in `audit.json`. Therefore this report does NOT claim universal native/
Python equivalence or a fully closed correctness gate. Root/reviewer must decide
that separate schema discrepancy before any future proof release. No new card was
created and no semantic repair was smuggled into this optimization.

The same initial test run also had a test-author mistake: it expected an event's
end offset rather than start offset. Corrected test passes on unchanged baseline
(`event-baseline-corrected.log`), then candidate full suite. The no-events regression
with an unused descriptor isolates nullability without concealing the separately
retained zero-descriptor defect. Initial failed log and red retention gate remain.
No candidate correctness or timed comparison failure occurred.

## Artifacts and review boundary

All full timing result JSONs (including every output), role freezes and results
are in `raw-results.tar.gz`, 601417 bytes, SHA256
`6896dfe1ffec3628d213949df243e96ece50bb465cc6589b7745401e179a2686`.
`archive-manifest.json` binds all22 members and unchanged Python/reference source;
packager re-read every archived member and compared it to its original. Individual
files and copied executable binaries remain locally under the evidence root;
only the small compressed evidence is committed, not executables/process listings.

For arithmetic-only independent review on a fresh checkout, extract the archive
into a new directory and read its role results/raw cells; `recompute.py` expects
baseline/results.json and candidate/results.json beside it. In the active checkout
those originals already exist; do not overwrite them or rerun compare.py. The
existing `audit.py` additionally needs the locally retained binaries/process
snapshots to reverify their hashes. Neither audit nor recompute runs timed cases.

Review must independently recompute timings and challenge invariants and the
zero-descriptor discrepancy. Configured reviewer verified as grok-4.5/xai-oauth;
actual review session/verdict remains pending. No Linux build or qualification,
production retry, owner ranking, lifecycle acceptance or campaign completion is
authorized by this local speedup. Root must resolve correctness and separately
release exact-source Linux building/proof; all previous capture/replay failures,
resource ceilings and final whole-branch Grok4.6 gates remain intact.
