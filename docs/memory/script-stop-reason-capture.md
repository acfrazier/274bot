# Script stop-reason diagnostic capture

## Scope

Opt-in diagnostic retention of `ScriptRunner.stop(reason)` / `host.stopReason`
for memory-harness forensics. This does **not** change Stop semantics, the
generic isolate stop log, host action APIs, wire shapes, or normal metrics rows.

## When it runs

- Feature: `script/memory-profile` (host-play memory builds).
- Mode: `BOT_MEMORY_DIAGNOSTICS=1`, resolved **once** when `LoadIsolate::spawn`
  creates the per-isolate `Counters` Arc (not re-read on each tick/stop).
- Default builds and memory-profile runs without diagnostics perform **no**
  extra JS read on stop.

## Capture rules

On already-detected `stopRequested`, before the Runtime drops:

1. If diagnostics are off, do nothing (no JS property read).
2. If on, inspect **only** an own **data** property descriptor of
   `globalThis.__rs2b0t_host.stopReason` via `Object.getOwnPropertyDescriptor`.
   Accessors are not invoked (avoids throwing/hanging getters changing Stop).
3. Accept only `typeof value === 'string'`. Missing, non-string, accessor, or
   eval failure → diagnostic **unavailable** (`stop_reason: null` in progress).
4. Genuine empty string `""` is retained and is distinct from unavailable.
5. Bound to **1024** Unicode scalars (JS slice before bridge + Rust re-cap).
6. Store once on the isolate `Counters` Arc. No new `ThreadMsg`, no channel
   payload, no Runtime/world/snapshot retention for the reason. Registry stays
   weak.

## Surfaces

| Surface | `stop_reason` |
| --- | --- |
| `Counters::snapshot` / normal metrics rows | **never** included |
| `LoadIsolate::memory_progress` (diagnostics on) | string, `""`, or `null` after stop; omitted before stop / when diagnostics off |
| Slot / host-play `memory_script_progress` | passes through isolate progress (existing path) |
| Log line | unchanged: `script requested stop on tick {n}; isolate stopping` |

## Storage cost

At most one `String` ≤ 1024 scalars plus enum tag on the existing per-isolate
`Counters` Arc, only written after a script-requested stop in diagnostic mode.

## Tests

`crates/script/tests/load_isolate.rs` (feature `memory-profile`):

- sentinel capture + exact generic log
- diagnostics off omits field
- empty string vs unavailable
- missing / non-string → null
- accessor not invoked
- oversized bound to 1024
- new isolate does not retain prior reason

## Non-goals

Does not diagnose or fix the original N16 stop cause. A separate bounded
diagnostic live run may use this field after review approval.
