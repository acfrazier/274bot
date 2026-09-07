# Matched evidence adapter — implementation report

Status: contract fixes after review round-1; reader + focused tests green.
No live runs, freeze acceptance, or performance pass claimed.

## Deliverables

| Path | Role |
|---|---|
| `docs/memory/matched_evidence_adapter.py` | Artifact-bound pair reader |
| `docs/memory/test_matched_evidence_adapter.py` | Synthetic + real N1 fixture tests |
| `docs/memory/reference_metrics.py` | Match-key vs side-provenance split (full non-side equality) |
| `docs/memory/matched-evidence-adapter-report.md` | This report |

Design reference: `docs/memory/matched-evidence-adapter-design.md` (approved `4de3b76`).

## What the reader does

For each side, `bind_side(receipt, role, manifest, …)` follows the chain:

1. Load cell `receipt.json` with required fields: `id`, `index`, `kind`, `run_dir`, `exit_code`, `binary`, `effective_cli`, `started_utc`, `ended_utc`.
2. Require receipt↔metadata agreement on run_dir, exit_code, binary (canonical path), and recorded launch CLI (frontend/N/workload/timing/flags/`--binary`).
3. Require **receipt-recorded** `raw_hashes` (or `artifact_hashes`) matching live file SHA-256 for `metadata.json`, `samples.jsonl`, `samples.qualification.jsonl`. Live snapshot alone → `receipt_raw_hashes_missing` / not `binding_ok`. Recheck hashes after qualify/analyze.
4. Recompute binary SHA-256; require equality with metadata and the **exact canonical named manifest binary path** for that role/frontend. Same-hash copy at a different path is rejected. Symlink resolving to the same path is OK. No artifact-root fence (cross-directory frozen paths allowed).
5. Side provenance from the **manifest** side block (pre **and** post sources must match for stable hash; client commit + digest required). Checkout `host_commit` is never build authority. Different source commits/binary hashes across roles allowed when each binds its own role entry.
6. When metadata **and** manifest both carry a fixture field, they must agree. **No manifest→metadata enrichment** of missing match keys.
7. Independently recompute `qualify_control.qualify` and `reference_metrics.analyze_run(qualify=True)`. Receipt/report `qualified` labels are ignored.
8. Observation wall span from analyzed `observation_window` (`obs_start_unix`/`obs_end_unix`), finite increasing and contained in process times. Never whole-process span alone; never cross-run mono origins.
9. Server start identity + port, sampler interval/duration, and host_conditions are required match-key inputs (non-sensitive only; no full env/argv dump).
10. Helper overhead stays `status=unavailable` / `measured=False` without continuous helper accounting. Snapshot before/after and forged `sampler.overhead="measured"` do not unlock measured overhead.

`bind_pair` has **no** `require_match_keys_complete=False` / `allow_n_gt1_without_ordinals=True` bypasses. It requires: both sides `binding_ok` + receipt-recorded raw hashes, both qualified, distinct run identities/dirs, non-overlapping observation wall spans, complete equal match keys, N>1 stable ordinals (else unavailable), endpoint family notes present on both sides with availability-class agreement, and measured overhead before any pair eligibility. Even then `pair_eligible=False` and `final_acceptance_claim=False`.

Failed/unrun cells retained via `preserve_failed_cell`.

## Minimal `reference_metrics` shaping

- `RESOURCE_MATCH_KEY_FIELDS` includes scheduling/failure/nav/fine/terminal instrumentation flags.
- `resource_match_keys_from_meta()` — **no** TUI null `render_policy` → `"none"` default.
- `resource_side_provenance_from_meta()` — role digests that may differ.
- `compare_matched_runs._match_only` compares **all non-side keys** in supplied match_metadata (full equality guard minus `binary_sha256`/`host_sources_sha256` only). Instrumentation flags such as `failure_capture` cannot be dropped by a short whitelist.
- Single-run `RESOURCE_PROVENANCE_FIELDS` still requires binary/host digests for the resource gate’s missing-provenance check.
- `evaluate_resources` may still map TUI null policy to budget key `"none"` for budget lookup only; that is not match-key equality.

## Contract fixes vs ee09dd7 / review round-1

| Gap | Resolution |
|---|---|
| Manifest→metadata enrichment | Removed; missing keys stay unavailable |
| TUI null render_policy → `"none"` in match helpers | Removed from `construct_match_keys` / `resource_match_keys_from_meta` |
| Incomplete receipt/launch binding | Required receipt fields + CLI/binary/exit/run_dir checks |
| Snapshot raw hashes as binding_ok | Require receipt-recorded hashes + post-read recheck |
| Same-hash non-canonical path | Exact canonical path equality (symlink OK) |
| Weak side/build provenance + non-SHA fixtures | Dual pre/post sources, client commit/digest, realistic 64-hex fixtures |
| Process span as observation bounds | Analyzed `observation_window` only |
| bind_pair bypass flags | Removed |
| `_match_only` whitelist dropping flags | All non-side keys compared |

## Tests run

```text
python3 -m unittest test_matched_evidence_adapter -v
# 23 tests OK

python3 -m unittest test_reference_metrics.ResourceAdapterTests -v
# 7 tests OK
```

Coverage includes:

- Positive synthetic pair: binding + independent qualification + differing side provenance + complete match keys + receipt-recorded raw hashes → pair still `overhead_unavailable` (binding ≠ eligibility).
- Negatives (production reader path): missing/null match key (no enrichment fill), null render_policy, nested null renderer_settings, swapped binary hash, receipt binary mismatch, copied same-hash non-canonical path, missing receipt fields, missing receipt raw hashes, missing metadata run_dir, wrong-role manifest bind, duplicate run identity, overlapping observation windows, receipt run_dir miss, failed qualification (ignores receipt label), endpoint mismatch, N>1 without ordinals, forged overhead, match-key mismatch, failure_capture mismatch via `compare_matched_runs`.
- Real N1 control/candidate receipts + shared-nav manifest: remains unavailable (`side_binding_failed` from incomplete legacy receipt chain / missing host conditions / raw hashes); preserved N16 failure cell; never relabeled as accepted; `binding_ok` not claimed on incomplete legacy chain.
- Match-key helpers exclude side digests; instrumentation flags remain in equality.

## Real N1 fixture outcome (read-only)

Paths used (no archive search):

- receipts under `diagnostics/shared-nav-clean-screen-20260907T004029Z/cell_0{1,2}_*_n1_profiles_on/`
- runs `diagnostics/20260907T004143Z_tui_n1_active` and `…004608Z…`
- `shared-nav-build-manifest.json` + frozen binaries under `diagnostics/shared-nav-build-20260906T235815Z/`
- `server_identity.json` (`start_identity`, `port_listen`; argv/env not required)

Legacy N1 receipts lack `raw_hashes`, structured host_conditions binding, and several strict match keys on metadata (`catalog_sha256`, `feature_flags`, `allocator_provenance`, `renderer_settings`, `cache_settings`, `failure_capture`, …). **No enrichment** fills those holes. Result: **unavailable**, not a paired pass. N16 diagnostic `20260907T041253Z` is diagnostic-only and cannot form a pair on this card.

## Remaining capability gaps (precise)

1. **Continuous helper/process/server overhead accounting** — OFF→ON→ON→OFF cell order is defined in batch_plan, but no continuous helper CPU/RSS series + start/stop identity proof exists; sampler labels say unmeasured. Adapter will not invent a measured flag or new threshold.
2. **Launcher match-key completeness** — `failure_capture`, `responsiveness_fine`, `nav_captures`, renderer/cache settings, and related fields must be explicitly recorded (no silent default-to-false on accept). Legacy N1 may omit them → unavailable is correct.
3. **Receipt-recorded raw/manifest hashes** — launcher should persist raw file hashes on the receipt so binding is not a live snapshot. Current N1 lacks this → not `binding_ok`.
4. **Host conditions as structured match identity** — non-sensitive machine blob must be bound into match keys; text-only ambient dumps are not yet a structured receipt field on legacy batches.
5. **Build vs checkout labeling** — metadata `host_commit` is current checkout; build authority is manifest commit + binary hash (implemented).
6. **N>1 stable ordinal / fixture mapping** — not emitted by launcher; slot-level pairing unavailable without predeclared ordinals or an explicit fleet-worst-case rule over every qualified slot.
7. **Endpoint families** — decode / input / GPU completion / TUI flush remain separate; inspected TUI N1 cells still lack decode/input availability for paired endpoint proof.
8. **Paired fine p99 ≤2 ms eligibility** — still unavailable until binding + qualification + endpoint + overhead all succeed; diagnostic_margin_ms only.
9. **Resource gate “available”** — still blocked on missing provenance fields and `overhead_unknown` inside `evaluate_resources`; adapter does not bypass that.

## Non-claims

- No performance acceptance, freeze acceptance, or live/binary/cargo work.
- No edits to launcher, `qualify_control.py`, Rust, instrumentation, thresholds, or old raw artifacts.
- No new overhead protocol.
- Caller `qualified=True` / `overhead="measured"` never unlocks a gate.
- Enrichment of missing metadata from the manifest is **not** acceptable and is not performed.

## Verification commands

```bash
cd docs/memory
python3 -m unittest test_matched_evidence_adapter -v
python3 -m unittest test_reference_metrics.ResourceAdapterTests -v
```
