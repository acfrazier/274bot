# Client runtime settings — implementer report

Task: t_0e0b1b07  
Worktree: codex/memory-diagnostics (`t_a1f4796f`)  
Owned paths: `crates/host-play/src/lib.rs`, `crates/host-play/src/memory.rs`, this report

## What landed

### Capture (`crates/host-play/src/lib.rs`, `memory-profile` only)

- `ClientRuntimeSettingsSnapshot` — pure `Copy` scalar snapshot of live client fields:
  - `lowmem` ← `c.config.lowmem`
  - `midi_active`, `midi_volume`, `wave_enabled`, `wave_volume`
  - `draw` ← `c.draw`
  - `loop_cycle` ← `c.loop_cycle` (client main-loop counter / freshness; **not** server/player generation)
- `capture_client_runtime_settings(&Client)` builds the snapshot with field reads only — no path clones, no heap, no clock.
- Status publication (existing `statuses` lock update when username matches, after `slot_frame`) sets  
  `s.runtime_settings = Some(capture_client_runtime_settings(c))` under `#[cfg(feature = "memory-profile")]`.
- `SlotStatus::default()` leaves `runtime_settings: None` until first observe.
- Cache path is **not** captured per frame (PlayOptions / bot_client_config boundary already owns it).

### Qualification serialize (`crates/host-play/src/memory.rs`)

Per-slot `runtime_settings` JSON keys when observed (`Some(snapshot)`):

| key | meaning |
|---|---|
| `lowmem` | live `Client.config.lowmem` bool |
| `midi_active` | live MIDI enable |
| `midi_volume` | live MIDI volume (i32) |
| `wave_enabled` | live wave enable |
| `wave_volume` | live wave volume (i32) |
| `draw` | live client draw flag |
| `loop_cycle` | client main-loop counter (freshness) |
| `loop_cycle_meaning` | always `"client_mainloop_counter"` |

Semantics:

- `SlotStatus.runtime_settings == None` → JSON `null` (never observed; **not** “all false”).
- `Some(snapshot)` with false booleans → JSON **object** with those falses (observed false ≠ null).
- Ordinals still come only from fixture `names` order via `qualification_slot_rows`; status matching is by username, not status-vector index.
- Additive key on the slot row; legacy keys/ordinals preserved. `renderer` is only attached when a match exists (key omitted when absent).

Optional per-slot `renderer` evidence (when render profile enabled and `slot_id_for(name)` matches a row from **one** `host::render_profile::read()` call per boundary):

| key | meaning |
|---|---|
| `available` | always `true` when object present |
| `source` | `"host::render_profile::read"` |
| `slot_id` | matched id |
| `generation` | observation generation |
| `renderer_present` | bool |
| `backend` | observed backend label string (`as_str()`); not requested GPU |
| `draw` | observed draw |
| `full_rate` | observed full_rate |
| `updated_ms` | observation timestamp ms |
| `ended` | whether the matched row is ended |

Not included in scoped `renderer` JSON: `prefer_cpu`, histories, interval buckets, GPU completion counters.

Global settings markers (no longer claim lowmem/audio wholly unavailable):

- `client_lowmem_actual`:  
  `{ available: true, source: "per_slot", path: "slots[].runtime_settings.lowmem", note: ... }`
- `client_audio_actual`:  
  `{ settings_available: true, settings_source: "per_slot", settings_path: "slots[].runtime_settings.{midi_active,midi_volume,wave_enabled,wave_volume}", settings_note: ..., physical_output_available: false, physical_output_reason: "host speaker/device/sink ownership is not observed; do not infer physical audio output from midi/wave booleans" }`
- `client_renderer_actual` when profile on:  
  `{ available: true, source: "per_slot", path: "slots[].renderer", note: "scoped host::render_profile::read match by slot_id_for(name); ..." }`  
  when off: unavailable marker (requested GPU is not actual backend).

### Ordinary / cfg behavior

- Snapshot type, `SlotStatus.runtime_settings`, capture helper, and status write are `#[cfg(feature = "memory-profile")]` only.
- `memory-profile-no-alloc` enables `memory-profile`, so the same capture/serialize path is present.
- Default / non-profile `SlotStatus` shape and ordinary build have **no** added fields or capture work.

## Tests

- `slot_status_walk_defaults_cleared` — `runtime_settings.is_none()` under feature.
- `capture_client_runtime_settings_sees_live_scalars` — Client stub; mutate lowmem/midi/wave/draw/loop_cycle; assert exact snapshot (false booleans and non-defaults), not hardcoded defaults.
- `runtime_settings_none_vs_observed_false_are_distinct`
- `qualification_rows_match_settings_by_name_not_status_order`
- `renderer_evidence_for_matches_slot_id_and_skips_missing`
- Settings serialize tests assert `source: "per_slot"` + path/settings_path shape and physical-output unavailable.
- Legacy slot key set includes additive `runtime_settings` (null until observe).

## Commands (verified)

```text
# Targeted filter (memory-profile)
cargo test -p host-play --features memory-profile --lib -- \
  capture_client_runtime_settings runtime_settings_none \
  qualification_rows_match_settings renderer_evidence_for \
  serialize_qualification_boundary slot_status_walk_defaults

# Full host-play lib with memory-profile
cargo test -p host-play --features memory-profile --lib

# Full host-play lib with memory-profile-no-alloc
cargo test -p host-play --features memory-profile-no-alloc --lib

# Ordinary host-play lib (no memory-profile)
cargo test -p host-play --lib
```

Prior review re-run: memory-profile 175 ok; memory-profile-no-alloc 175 ok; ordinary 114 ok.

## Actual vs unavailable

| signal | meaning |
|---|---|
| `runtime_settings: null` | slot never observed / no status row yet |
| `runtime_settings: { lowmem: false, ... }` | observed live scalars (false is real) |
| MIDI/wave booleans | client settings only — **not** physical audio device/sink ownership |
| missing `renderer` key | profile off, no match, or no observations |
| `renderer.backend` | observed residency label from render_profile; requested GPU ≠ actual backend |
| global lowmem/audio markers | pointers to per-slot paths; do not invent process-wide values |

## Residual risks

- Capture only on the existing status-publication path; never-observed slots stay `null`.
- Renderer evidence is independent of runtime_settings and only present when render profile is enabled and a matching slot_id exists.
- Live `Client.config.lowmem` can still differ from wall `PlayOptions.lowmem`; both remain in the qualification settings block (wall vs per-slot pointer).
