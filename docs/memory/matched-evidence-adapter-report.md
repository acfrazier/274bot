# Matched evidence adapter — implementation report

Status: smallest implementation step complete (reader + focused tests).
No live runs, freeze acceptance, or performance pass claimed.

## Deliverables

| Path | Role |
|---|---|
| `docs/memory/matched_evidence_adapter.py` | Artifact-bound pair reader |
| `docs/memory/test_matched_evidence_adapter.py` | Synthetic + real N1 fixture tests |
| `docs/memory/reference_metrics.py` | Minimal match-key vs side-provenance split |
| `docs/memory/matched-evidence-adapter-report.md` | This report |

Design reference: `docs/memory/matched-evidence-adapter-design.md` (approved `4de3b76`).

## What the reader does

For each side, `bind_side(receipt, role, manifest, …)` follows the chain:

1. Load cell `receipt.json` (declared run_dir, exit, sampler interval, binary).
2. Load canonical `metadata.json` + raw `samples.jsonl` / `samples.qualification.jsonl`; hash them; require receipt/metadata run_dir and exit agreement.
3. Recompute binary SHA-256; require equality with metadata and the named manifest binary entry for that role/frontend (`control_tui_play` / `candidate_tui_play`, etc.). Paths may cross directories (no root fence); symlink/duplicate paths ok only when content hash matches the named entry.
4. Side provenance from the **manifest** side block (`commit` / `commit_at_freeze_snap`, `sources_sha256_pre`), not from checkout `host_commit`. Different source commits and binary hashes across roles are allowed when each binds its own role entry.
5. Independently recompute `qualify_control.qualify` and `reference_metrics.analyze_run(qualify=True)`. Receipt/report `qualified` labels are ignored.
6. Build strict **match keys** (null/missing never equal) and **side provenance** separately.
7. Helper overhead stays `status=unavailable` / `measured=False` unless continuous helper accounting exists. Snapshot before/after files and forged `sampler.overhead="measured"` labels do not unlock measured overhead.

`bind_pair` then requires: both sides bound + qualified, distinct run identities and run dirs, non-overlapping wall-clock observation spans (never cross-run mono origins), complete equal match keys, N>1 stable ordinals (else unavailable), endpoint availability class agreement, and measured overhead on both sides before any pair eligibility. Even then this module sets `pair_eligible=False` and `final_acceptance_claim=False` — acceptance is not authorized here.

Failed/unrun cells can be retained via `preserve_failed_cell` (e.g. N16 functional failure).

## Minimal `reference_metrics` shaping

- `RESOURCE_MATCH_KEY_FIELDS` / `resource_match_keys_from_meta()` — equality-checked configuration (no `binary_sha256` / `host_sources_sha256`).
- `RESOURCE_SIDE_PROVENANCE_FIELDS` / `resource_side_provenance_from_meta()` — role digests that may differ.
- `evaluate_resources` emits both `match_metadata` and `side_provenance`.
- `compare_matched_runs` equality-checks match keys only; requires side provenance present on each side (backward-compatible fallback if digests still sit in match_metadata).

Single-run `RESOURCE_PROVENANCE_FIELDS` still requires binary/host digests present for the resource gate’s missing-provenance check.

## Tests run

```text
python3 -m unittest test_matched_evidence_adapter -v
# 15 tests OK

python3 -m unittest test_reference_metrics.ResourceAdapterTests -v
# 7 tests OK
```

Coverage includes:

- Positive synthetic pair: binding + independent qualification + differing side provenance + complete match keys → pair still `overhead_unavailable` (binding ≠ eligibility).
- Negatives: missing/null match key, swapped binary hash, wrong-role manifest bind, duplicate run identity, overlapping windows, receipt run_dir miss, failed qualification (ignores receipt qualified label), endpoint mismatch, N>1 without ordinals, forged overhead without continuous helper accounting, match-key mismatch.
- Real N1 control/candidate receipts + shared-nav manifest: remains unavailable (missing match-key fields and/or overhead); preserved N16 failure cell; never relabeled as accepted.
- Match-key helpers exclude side digests so control vs candidate binary/source hashes do not false-mismatch equality checks.

## Real N1 fixture outcome (read-only)

Paths used (no archive search):

- receipts under `diagnostics/shared-nav-clean-screen-20260907T004029Z/cell_0{1,2}_*_n1_profiles_on/`
- runs `diagnostics/20260907T004143Z_tui_n1_active` and `…004608Z…`
- `shared-nav-build-manifest.json` + frozen binaries under `diagnostics/shared-nav-build-20260906T235815Z/`
- `server_identity.json` (`start_identity`, `port_listen`; argv/env not required)

Current N1 metadata still lacks several strict match keys on disk (`nav_flags_sha256`, `catalog_sha256`, `feature_flags`, `allocator_provenance`, `renderer_settings`, `cache_settings`, `failure_capture`, `responsiveness_fine`, …). The adapter may enrich nav/catalog/allocator/feature from the **shared manifest** when binding, but launcher-absent instrumentation keys and continuous helper overhead remain gaps. Result: **unavailable**, not a paired pass. Numeric RSS/CPU and fine p99 diagnostics stay diagnostic only.

## Remaining capability gaps (precise)

1. **Continuous helper/process/server overhead accounting** — OFF→ON→ON→OFF cell order is defined in batch_plan, but no continuous helper CPU/RSS series + start/stop identity proof exists; sampler labels say unmeasured. Adapter will not invent a measured flag or new threshold.
2. **Launcher match-key completeness** — `failure_capture`, `responsiveness_fine`, `nav_captures`, renderer/cache settings, and related fields must be explicitly recorded (no silent default-to-false on accept). Legacy N1 may omit them → unavailable is correct.
3. **Build vs checkout labeling** — metadata `host_commit` is current checkout; build authority is manifest commit + binary hash (implemented). Future launcher should not present checkout commit as build commit.
4. **N>1 stable ordinal / fixture mapping** — not emitted by launcher; slot-level pairing unavailable without predeclared ordinals or an explicit fleet-worst-case rule over every qualified slot.
5. **Endpoint families** — decode / input / GPU completion / TUI flush remain separate; inspected TUI N1 cells still lack decode/input availability for paired endpoint proof.
6. **Paired fine p99 ≤2 ms eligibility** — still `paired_matched_run_evidence_pending` / unavailable until binding + qualification + endpoint + overhead all succeed; diagnostic_margin_ms only.
7. **Resource gate “available”** — still blocked on missing provenance fields and `overhead_unknown` inside `evaluate_resources`; adapter does not bypass that.

## Non-claims

- No performance acceptance, freeze acceptance, or live/binary/cargo work.
- No edits to launcher, `qualify_control.py`, Rust, instrumentation, thresholds, or old raw artifacts.
- No new overhead protocol.
- Caller `qualified=True` / `overhead="measured"` never unlocks a gate.

## Verification commands

```bash
cd docs/memory
python3 -m unittest test_matched_evidence_adapter -v
python3 -m unittest test_reference_metrics.ResourceAdapterTests -v
```
