# Runtime configuration + qualification ordinal evidence (host-play)

Owned artifact for card: record fixture ordinals and selected runtime options on successful qualification boundaries.

Source: `crates/host-play/src/memory.rs`  
Boundaries: successful `observe-start` / `observe-end` via `Run::write_qualification`  
Not complete: performance/overhead claims, live client renderer/audio/lowmem actuals, cache content hashes.

## Top-level successful boundary JSON

Produced by `serialize_qualification_boundary` (slots first in object construction, then `elapsed_s`, then additive `settings`):

| key | type | notes |
|-----|------|--------|
| `phase` | string | `"observe-start"` or `"observe-end"` |
| `elapsed_s` | number | wall elapsed since run start; meaning unchanged |
| `slots` | array | one row per `Run.names` entry (authoritative order) |
| `settings` | object | **additive**; successful boundaries only |

Legacy top-level keys `phase` / `elapsed_s` / `slots` preserved. Failure-boundary path shares the ordinal slot producer but still **omits** `settings`.

## Slot row schema (`slots[]`)

Ordinal authority is `self.names.iter().enumerate()` only — never status-map / hash-map iteration order.

| key | type | notes |
|-----|------|--------|
| `ordinal` | u64 | 0-based fixture ordinal; stable across runs for same name list |
| `name` | string | fixture account name (legacy) |
| `responsiveness_slot_id` | number | `host::responsiveness_profile::slot_id_for(name)`; **run-local** FNV id — do not compare numerically across separate process runs |
| `cadence_slot_id` | number | `host::cadence::slot_id_for(name)`; same run-local rule |
| `state` | string | `Debug` of script state (legacy) |
| `error` | string \| null | last script error (legacy) |
| `runtime` | object | `memory_script_progress` (legacy) |
| `client` | object \| null | `ingame`, `scene_state`, `x`, `z`, `level` when status present (legacy) |

N=1 and N=16 emit exactly one row per name; no omissions/duplicates; name order equals `Run.names` order.

## `settings` schema (successful boundaries)

Built by `qualification_settings_value` from private `Play.options` (child `memory` module field access — no new public wall API), plus run config flags already on `Run`.

### Selected Play / harness options (native evidence)

| key | type | source |
|-----|------|--------|
| `cache_dir` | string | `PlayOptions.cache_dir` as selected |
| `cache_dir_canonical` | string \| null | `Path::canonicalize` when path exists |
| `cache_dir_canonical_available` | bool | true iff canonicalize succeeded |
| `cache_dir_canonical_reason` | string | **only when** canonicalize failed |
| `cache_content_hash` | null | always null at boundary |
| `cache_content_hash_reason` | string | `"not_hashed_at_boundary; launcher/preflight may hash path independently"` |
| `host` | string | non-secret `PlayOptions.host` |
| `port` | number | non-secret `PlayOptions.port` |
| `lowmem_requested` | bool | wall `PlayOptions.lowmem` at `Play::new` — **not** live client lowmem |
| `mainland` | bool | `PlayOptions.mainland` |
| `frontend` | string | run frontend label |
| `n` | number | `Config.n` |
| `workload` | string | `Config.workload` as str |
| `render_policy_requested` | string | run `RenderPolicy` as str |
| `single_renderer` | bool | run flag |
| `diagnostics` | bool | run flag |
| `failure_capture` | bool | run flag |

### Env labels vs actual profiler enablement

| key | type | meaning |
|-----|------|---------|
| `env_flags_requested` | object | process env reread at boundary time; **requested labels only** |
| `env_flags_requested.BOT_SCHEDULING_PROFILE` | bool | env `== "1"` |
| `env_flags_requested.BOT_RENDER_PROFILE` | bool | env `== "1"` |
| `env_flags_requested.BOT_GPU_COMPLETION_PROFILE` | bool | env `== "1"` |
| `env_flags_requested.BOT_RESPONSIVENESS_PROFILE` | bool | env `== "1"` |
| `env_flags_requested.BOT_RESPONSIVENESS_FINE` | bool | env `== "1"` |
| `scheduling_profile_enabled` | bool | `host::cadence::enabled()` actual accessor |
| `render_profile_enabled` | bool | `host::render_profile::enabled()` |
| `gpu_completion_profile_enabled` | bool | `host::render_profile::gpu_completion_enabled()` |
| `responsiveness_profile_enabled` | bool | `host::responsiveness_profile::enabled()` |
| `responsiveness_fine_enabled` | bool | `host::responsiveness_profile::fine_enabled()` |

Do not treat `env_flags_requested.*` as proof the corresponding profiler was initialized.

### Unavailable live client state (explicit; not defaults)

Each is `{ "available": false, "reason": "<… without invasive instrumentation>" }`:

| key | reason gist |
|-----|-------------|
| `client_lowmem_actual` | per-slot `ClientConfig.lowmem` / live `Client.lowmem` not on `Play` statuses |
| `client_audio_actual` | per-slot audio/music gate not on statuses |
| `client_renderer_actual` | per-slot renderer backend/mode not on statuses |

These markers must **not** be used to claim match completeness for actual renderer/audio/lowmem.

## Remaining unavailable for root reader integration

Root launcher / matched-evidence reader still must:

1. Hash cache contents independently from emitted `cache_dir` / `cache_dir_canonical` (native does not hash).
2. Not invent `client_*_actual` from defaults; treat `available: false` as gap.
3. Map process-local instrumentation samples onto fixture ordinal via `responsiveness_slot_id` / `cadence_slot_id` within the **same run** only; generations are separate.
4. Failure-boundary rows lack `settings` — do not assume settings present on every phase line.
5. Not declare performance/overhead or actual renderer settings complete from this artifact alone.

## Non-goals preserved

- No bulk cache hashing in boundary or native hot path
- No actions / log drain / status mutation for this path
- No extra per-tick world deep copies
- No name-mint algorithm changes
- No launcher / Python reader / client / nav / panel / TUI edits in this commit

## Verification

```text
cargo test -p host-play --features memory-profile --lib memory::tests
```

Pure helpers covered: stable ordinals N1/N16, name-order independence from status hash order, settings distinctions (`lowmem_requested` vs unavailable actuals), legacy keys present, no cache hash, production serialize path.