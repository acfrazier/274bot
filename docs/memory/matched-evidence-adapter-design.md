# Artifact-backed matched evidence adapter

Status: design only. This document defines the smallest trustworthy reader for a
future paired comparison. It does not qualify any existing pair, measure
performance, authorize runs, or change `reference_metrics.py`.

## Purpose and non-claims

`compare_matched_runs()` currently accepts already-shaped dictionaries. Its
`match_metadata` equality and `overhead == "measured"` checks are useful
last-mile guards, but they do not establish that a dictionary came from a real
run, that its labels identify the binary that ran, or that its two sides were
independently qualified. A caller-provided `qualified=True`, `run_id`, or
`overhead="measured"` must therefore never unlock a gate.

The adapter should consume existing receipt, raw run, qualification, build
manifest, and report-helper outputs, verify their bindings, and only then emit
the dictionaries consumed by the existing comparison helpers. Missing, null,
empty, malformed, or unverifiable values are unavailable; they must never match
by equality.

## Authoritative artifact chain

For each slot, the reader follows this chain and verifies every link:

1. Batch cell `receipt.json` is the authoritative declared cell identity,
   command, run directory, sampler artifact, and exit status.
2. `metadata.json` in that exact `run_dir` is authoritative for what the
   launcher actually recorded: frontend, N, workload, flags, render policy,
   terminal geometry, hashes, commits, binary path/hash, run directory, start
   and end timestamps, and final `exit_code`.
3. The raw `samples.jsonl` and qualification input are authoritative for the
   measured observation and slot/generation evidence. No report prose or
   manually typed summary is a substitute.
4. `qualify_control.qualify(run_dir, counting=..., diagnostics=...)` is the
   authoritative workload qualification computation. The adapter must invoke
   this existing path read-only when implemented; it must not trust a copied
   `qualified` field in a report or receipt.
5. `shared-nav-build-manifest.json` is authoritative for the named control or
   candidate binary, its SHA-256, source/build provenance, client commit,
   feature/allocator configuration, and asset/catalog hashes. The binary hash
   in metadata must resolve to the manifest entry and the file hash must be
   recomputed.
6. Existing `reference_metrics.analyze_run(..., qualify=True)` and its gate
   results remain the authoritative metric/endpoint calculations. The new
   reader validates provenance and pairing; it does not reimplement histogram,
   scheduling, decode, input, GPU, or resource math.

Paths are authorized by the explicit receipt fields, manifest entries, and
provenance-lock paths, then canonicalized and hash-checked; there is no blanket
single-artifact-root rule. This permits the approved frozen binaries in the
shared-nav-build directory, run directories in their timestamped sibling
directories, batch receipts, and the shared build manifest to be bound exactly
as recorded. Symlinks and duplicate references should resolve to one canonical
file and be rejected if their content/hash does not match the recorded artifact.

## Required fields and current availability

`available now` means the field exists in the inspected artifacts and can be
used without guessing. `missing now` means the future adapter must reject the
current evidence until the launcher/receipt/build output records it.

| Evidence | Authoritative fields | Match rule | Current cells |
|---|---|---|---|
| Cell/slot identity | receipt `id`, `index`, `kind`, `run_dir`, `effective_cli`, `started_utc`, `ended_utc`, `exit_code`; metadata `run_dir`, `pid`, `started_unix`, `ended_unix`, final `exit_code` | receipt and metadata paths/times/status must agree; run identity is the canonical run-directory identity plus completed artifact hash, not a label | Partly available: receipts and metadata exist; no stable explicit run UUID or slot ordinal persisted by launcher |
| Raw sample binding | metadata `run_dir`; hash of `metadata.json`, `samples.jsonl`, `samples.qualification.jsonl` (adapter-computed); sample phases and terminal boundaries | files must be in that run directory, readable, complete, and hashed; samples must be nonempty and have valid observation boundaries | Run paths and files available; file hashes are not recorded by current launcher/receipts, so adapter-computed hashes can detect later mutation only if saved in a new receipt |
| Workload qualification | `qualify_control` result: `qualified is True`, no errors, expected workload, counting/diagnostic mode; metadata `exit_code == 0` | recompute from the exact run; require successful completion and requested mode; never compare `None` or absent fields | Computed qualification receipt exists for the two N=1 cells; this is independently recomputable. N16 candidate failed and must remain excluded |
| Binary identity | metadata `binary`, `binary_sha256`; manifest binary entry `path`, `sha256`, role, filename; actual file SHA-256 | actual hash = metadata hash = manifest entry hash; binary role must be the declared side; paths must be exact/canonical | Available for control/candidate TUI binaries; hashes differ as intended |
| Build provenance | manifest side `commit` / `commit_at_freeze_snap`, `branch`, source digest, client commit/source digest, `features`, allocator; metadata `host_commit`, `client_commit`, `host_sources_sha256`, `client_sources_sha256`, `rs2b0t_commit` | candidate and reference need exact named provenance independently; source commits may differ; client/assets/settings required for a matched cell must agree | Mostly available in manifest and metadata. Current `metadata` does not carry manifest role, feature flags, allocator provenance, or catalog hash |
| Navigation/catalog fixture | metadata `nav_pack_sha256`, `nav_flags_sha256`; manifest nav hashes and catalog `js_scripts_json_sha256`, `rs2b0t_commit` | exact equality across sides for the authorized fixture; absent/null never matches | nav pack hash available and matches manifest; nav flags and catalog hash are absent in current run metadata (visible only in external manifest/report) |
| Frontend/workload/scale | metadata `frontend`, `n`, `workload`, `terminal`, `terminal_size`; render policy fields | exact equality: frontend, N, workload, terminal mode/geometry; use the approved profile key | Available for current TUI N=1 cells; failed N16 is not eligible |
| Render/allocator/instrumentation policy | metadata `render_policy`, `render_policy_requested`, `scheduling_profile`, `render_profile`, `gpu_completion_profile`, `responsiveness_profile`, `responsiveness_fine`, `diagnostic_sidecar`, `stack_logging`, `single_renderer`, `sustain`; explicit feature/allocator fields required | exact equality for a paired metric; profile-on vs profile-off is an overhead experiment, not a candidate/reference pair; no absent flag defaults to false | Launcher flags are available; `feature_flags`, `allocator_provenance`, renderer settings, cache settings are absent/null in current resource metadata |
| Server condition | receipt sampler `pid_was`, command, interval, duration, output; `server_identity.json` `pid`, `start_identity`, `port_listen`, `preflight_utc`; server resource summary PID/interval/sample count/exit; batch host conditions | exact server identity/start identity, target, sampler version/config, interval/duration, and successful complete output; helper overhead must be separately evidenced. Require explicit non-sensitive relevant settings/version/start identity, not a full argv/env dump | PID/interval/duration/output paths and server `start_identity` are available; server target is available from the manifest/batch. Sampler/helper overhead proof and selected pressure counter are missing; argv/env are explicitly not recorded and are not required |
| Host/helper condition | batch `host_conditions`; Docker/VNC/Chroma identities and stats where present; sampler/helper process identities and CPU/RSS time series | same machine/OS/arch/CPU entitlement/RAM, server/helper versions and accounting scope; concurrent contamination rejects the cell | Host conditions and ambient helper snapshots are available; process-level helper CPU/RSS deltas and pressure are unavailable |
| Metric endpoint | raw field names and profile flags: decode, input visible acknowledgment, TUI flush, GPU callback/completion; `reference_metrics` gate status/endpoint notes | exact endpoint semantics must match; decode is not input, TUI flush is not GPU completion, GPU completion is not scanout | Scheduling rows exist; decode/input are unavailable in inspected TUI N=1 cells; no physical presentation endpoint |
| Observation contamination | observation window from raw timestamps plus existing contamination intersection; batch quiet-boundary receipts | both runs must have clean, non-overlapping observation windows and approved order; any overlap makes pair unavailable | N=1 cells are workload-qualified but resource overhead is unknown; do not infer clean acceptance from their report labels |

The current `metadata.json` also records the current checkout commit, even when
a frozen older binary ran. The binary SHA-256 and manifest build commit are the
authority for binary provenance; `host_commit` must not be relabeled as the
binary build commit.

## Pair identity and matching algorithm

A pair consists of two independently qualified completed runs with declared
roles `reference` and `candidate`. The reader must:

1. Load the cell receipt, exact run metadata, raw files, qualification result,
   and manifest entry for each side.
2. Verify all artifact hashes and the full chain above before constructing any
   comparison dictionary.
3. Require distinct canonical run directories, distinct completed run
   identities, and distinct non-overlapping observation windows. A run cannot
   serve as both sides, and a duplicate artifact hash cannot masquerade as a
   second run.
4. Split the comparison input into two classes. **Match keys**, which must be
   present and equal on both sides, are frontend, N, workload, fixture hashes,
   terminal geometry, allocator, server target/config, cache/renderer policy,
   profile flags, sampling configuration, host/helper machine conditions, and
   endpoint semantics. Any missing or null member makes the pair unavailable,
   never equal. **Side provenance** is separate and must be present and
   role-correct but may differ: binary SHA-256, build/source commits and source
   digests, client/build manifest entry, and manifest role/path. The current
   `reference_metrics._resource_match_metadata` and `compare_matched_runs`
   include `binary_sha256` and `host_sources_sha256` in equality-checked
   metadata, so the adapter must shape those as side fields (or the smallest
   later helper change must compare only match keys) rather than feeding the
   full provenance-bearing dictionary as `match_metadata`.
5. Require exact named provenance on each side. The candidate's source/build
   commit is allowed to differ from the reference's; each must match its own
   manifest role and expected binary hash. Do not require identical source
   commits, and do not use `host_commit` as a substitute for build provenance.
6. Recompute both gate results through `qualify_control` and
   `reference_metrics`; accept only the requested gate/endpoint status. A
   string label in a receipt, report, or caller payload cannot qualify a run.
7. Return a structured unavailable reason with the missing field/path when any
   check fails. Diagnostic arithmetic from existing helpers may still be
   emitted with `status=unavailable` (for example `diagnostic_margin_ms` or
   resource numeric rollups); it must not be labeled `paired_within_margin`, an
   eligible paired result, or an acceptance pass when qualification,
   provenance, endpoint, or overhead evidence is missing.

The existing `compare_matched_runs` can remain the final diagnostic delta
helper. Its `single_pair_no_variation` result is not significance or acceptance.
A later repeated-run aggregator may consume only adapter-approved pairs and must
preserve failed/unrun cells instead of selecting a favorable subset.

## Fresh-account run names and logical slots

Run names such as `20260907T004143Z_tui_n1_active` are fresh launcher names,
not stable logical bot slots. They cannot prove that slot 0 in one run is the
same logical account/fixture as slot 0 in another. For N=1, the reader can pair
only the run-level distribution/endpoint if the metric explicitly permits it;
it must not claim account identity from the name. For N>1, require a stable
predeclared ordinal/fixture mapping emitted in both raw artifacts, or use a
predeclared fleet-level rule that is explicitly worst-case over all qualified
slots. Never select a convenient slot, window, or generation after seeing the
result. Missing stable ordinal mapping means slot-level pairing is unavailable.

The adapter must preserve failed and unrun cells (for example, the inspected
candidate N16 failure and the not-run control N16/overhead cells) in its output
with their failure reason. They are not silently dropped from a matrix.

## Resource overhead evidence

Resource readings have four different owners and must remain separate:

- host process CPU seconds and RSS/peak RSS from `samples.jsonl`;
- game-server CPU/RSS from `server_resources.jsonl`;
- sampler/launcher/helper CPU/RSS and their lifetimes;
- instrumentation perturbation from profile-enabled versus profile-disabled
  runs on the same binary/configuration.

No subtraction of malloc bytes, RSS values, or unrelated helper snapshots is
valid. `sample_self` startup/residual snapshots do not measure helper overhead.
The old receipts explicitly say sampler overhead is unmeasured; their numeric
CPU/RSS values remain diagnostic only.

The existing predeclared batch's approved same-binary/configuration overhead
order is `candidate_n16_overhead_off_a → candidate_n16_overhead_on_a →
candidate_n16_overhead_on_b → candidate_n16_overhead_off_b` (`off → on → on →
off`). That is the order to consume; do not redefine it as a different minimum
protocol. Every cell needs its own raw samples, receipt, qualified completion,
helper accounting, and matched host/server conditions. Preserve failed/unrun
cells. The adapter should report process, server, helper, and instrumentation
deltas separately;
it must not invent confidence margins or turn a noisy sequence into `measured`.
If the sampler cannot provide continuous helper CPU/RSS and start/stop identity,
that is an environmental capability gap, not an adapter pass.

## Fine p99 and the 2 ms rule

Fine histograms and mono clock brackets can produce a single-run conservative
bound. A pair may be eligible only after both runs pass the artifact, qualification,
endpoint, and matching checks above. Then apply the existing rule
`candidate_upper - reference_lower <= 2 ms` using the candidate's fine upper
bound and reference's fine lower bound. Existing `paired_fine_p99_margin` may
still provide diagnostic arithmetic when proof is incomplete, but its result
must remain `status=unavailable` with reason `missing_capability` (with a
precise missing field) or `paired_matched_run_evidence_pending`; do not expose
`paired_within_margin` or call that diagnostic margin a pass.
Decode and input remain separate endpoint families. TUI flush remains distinct
from GPU queue callback completion, and neither is a physical display scanout
measurement.

Process-local monotonic clocks are not comparable across runs. The adapter may
compare distributions and duration-normalized metrics after independent bracket
validation, but must not compare raw `elapsed_mono_ns` values as if they shared
an origin.

## Smallest implementation step

Implement one new reader module adjacent to `reference_metrics.py` (or one
clearly named section owned by that file), plus focused tests in its existing
Python test module. It should own only:

- artifact path/hash and receipt-to-metadata-to-manifest binding;
- strict non-null matching-key construction;
- side-provenance construction outside equality-checked `match_metadata`,
  including the minimal shaping/equality adjustment needed because authorized
  control and candidate binary/source hashes differ;
- recomputed qualification invocation;
- preservation of failed/unrun cells;
- emission of adapter-approved inputs to existing metric/comparison helpers.

Do not alter `qualify_control.py`, the clock/latency instrumentation, run
authorization, resource thresholds, endpoint semantics, or reports in that
step. Test synthetic cases for missing-vs-null fields, swapped binary hashes,
source commits differing by role, reused run identity, overlapping windows,
receipt/metadata disagreement, failed qualification, endpoint mismatch, stable
ordinal absence, and measured-overhead claims without helper accounting. Add one
read-only fixture test using the two inspected N=1 artifacts and assert
`unavailable` for missing provenance/overhead rather than a paired pass.

Until that reader and real overhead artifacts exist, the current N=1 numeric
RSS/CPU values, fine/clock diagnostic arithmetic, and manually written labels
remain visible evidence only. They do not establish a paired performance result
or unlock any approved acceptance gate.
