# Instrumentation overhead four-cell reader

Offline artifact-backed reader for the approved same-binary N16
`off_a → on_a → on_b → off_b` instrumentation overhead protocol.

## Deliverables

| Path | Role |
|---|---|
| `docs/memory/instrumentation_overhead.py` | Reader: binds four ordered receipts via `matched_evidence_adapter.bind_side`, checks protocol, emits host/server/helper deltas |
| `docs/memory/test_instrumentation_overhead.py` | 24 stdlib unittest cases (adversarial + protocol + real single-artifact guard) |
| `docs/memory/instrumentation-overhead-report.md` | This report |

No edits to the existing adapter in this task. No Rust/build/live/server operations.

## API

```text
analyze_instrumentation_overhead(
    receipt_paths,          # exactly four paths, order off_a,on_a,on_b,off_b
    *,
    manifest_path,
    role="candidate",
    counting=False,
    diagnostics=False,
    server_identity_path=None,
    host_conditions_path=None,
    expected_n=16,
)
```

Each side is re-bound by invoking `bind_side`. Caller-supplied bound dicts,
`qualified` booleans, or `overhead="measured"` labels are never trusted as
evidence. Optional `bind_side=` is test-only injection; production default is
the real adapter.

## Protocol checks

1. Exactly four ordered receipts.
2. Each side: `binding_ok`, status `bound`, `qualified`, exit 0, native
   qualification available, managed process evidence available.
3. Distinct run identities and run dirs; non-overlapping chronological
   observation wall spans in declared order.
4. Same frozen `binary_sha256` on all four cells.
5. Profile group toggled as a unit:
   - OFF: scheduling/render/responsiveness/fine all false, GPU false
   - ON: those four true, GPU false (TUI)
   - Order must be off/on/on/off
6. Match keys equal except allowed profile toggles and
   `renderer_settings` (instrumentation availability). Workload knobs such as
   `tui_input_probes`, frontend, N, workload, failure/nav captures must match
   exactly — OFF/ON does not change probe workload.
7. Native match keys equal except allowed profile-enabled / env-flag /
   renderer-config-by-ordinal fields.
8. Real process roles present on every cell. Owned
   controller/launcher/collector PIDs may differ; server and ambient
   (gateway, server_supervisor) identities must be equal.
9. Protocol expects N=16; other N yields `partial` with an explicit remaining
   condition rather than a silent pass.

## Metrics reported (separately)

- **Host** = continuous process role `controller` (CPU_s enclosing observation,
  conservative cpu_cores interval upper, median RSS).
- **Server** = `game_server`.
- **Helpers** = every other role by name (launcher, collector, gateway,
  server_supervisor, …).
- Optional diagnostic channel: `samples.jsonl` host CPU/RSS via analysis gates —
  never subtracted from continuous role accounting.
- No subtraction of helper CPU/RSS or malloc bytes from host.

With two observations per setting the reader emits:

- observed OFF/ON ranges and relative spreads `(max-min)/min`
- two adjacent paired deltas oriented **ON − OFF**:
  `off_a → on_a` and `on_b → off_b`
- these are **not** confidence intervals

### CPU 5% empirical screen (host)

`within_5pct_empirical_screen` only when:

- conservative `ONmax / OFFmin ≤ 1.05`, and
- both OFF and ON replicated spreads ≤ 5%

Otherwise:

- `regression` when the conservative ratio exceeds 1.05 with tight spreads
- `inconclusive` when spreads exceed 5% or values are incomplete

This screen is **not** final acceptance, pair eligibility, or an accepted RSS
saving. `final_acceptance_claim`, `pair_eligible`, and `accepted_rss_saving`
remain false.

### Latency

Missing OFF histograms ⇒ `latency_overhead.status = unmeasured` with reason
`missing_off_histograms`. That is **not** zero overhead. TUI input probe writes
(when present and equal across cells) are not visible acknowledgments or latency
proof; native counters remain authoritative. Four-cell resource results stay
honestly partial on missing OFF latency even when resource deltas are available.

### Renderer

Unprofiled renderer rows stay unavailable. The reader does not fabricate an
actual backend from `draw=false` when render profile is off. If an OFF cell
reports an enabled renderer row, comparison is blocked with an explicit
capability note.

### Continuous helpers

Helper accounting measures helper resources under continuous coverage. It does
**not** measure sampler causal perturbation of the host workload.

## Status values

| status | meaning |
|---|---|
| `unavailable` | structural bind/protocol failure |
| `partial` | some cells/metrics usable; remaining_conditions listed; `instrumentation_overhead_measured=false` |
| `resource_deltas_available` | four-cell resource deltas produced under continuous evidence; still not final acceptance; latency may remain unmeasured |

`instrumentation_overhead_measured=true` only means separable host/server/helper
resource deltas exist under the protocol. It does **not** unlock pair
eligibility, RSS savings, p99 proof, or campaign acceptance.

## Tests (24)

Adversarial coverage:

- reorder / non-chronological profile order
- overlapping observation windows
- duplicate run identity
- changed binary / cache / server identity / sampler module
- missing helper role evidence
- fake measured overhead labels (not trusted toward acceptance)
- unqualified / numeric reset style incomplete sides
- only profile-group toggles allowed; frontend and `tui_input_probes` mismatches rejected
- mixed profile group and GPU-on rejected
- CPU regression and wide-spread inconclusive screens
- owned PIDs may differ while server identities match
- bind_side always invoked (default and injection)
- N≠16 remains partial
- real single managed-pipeline receipt duplicated four times cannot claim
  four-cell overhead

Run:

```bash
cd docs/memory && python3 -m unittest test_instrumentation_overhead -v
```

Result on this worktree: **24 OK**.

## Real artifacts

Latest functional real cell:

`docs/memory/diagnostics/managed-pipeline-qualification-20260907T062612Z/`

- `bound-side.json` / cell receipt: `qualified=true`, managed process resources
  available for six roles (controller, game_server, launcher, collector,
  gateway, server_supervisor).
- This is a single N1 profiles-on diagnostic, **not** the N16 OFF/ON/ON/OFF
  quartet.

The reader therefore does **not** claim overall instrumentation overhead from
that one artifact. Root prepares and runs the real four cells only after review
and idle workers.

## Scope / non-claims

- No final acceptance, accepted RSS saving, or auto pair eligibility.
- No reimplementation of receipt/build/native/process binding.
- No edit to `matched_evidence_adapter.py` in this card (root may land separate
  `tui_input_probes` match-key support; this reader already requires exact match
  on that key when present).
- Continuous helper series ≠ sampler causal perturbation proof.
- Missing OFF latency histograms keep latency overhead unmeasured.
