# Native boxed-tile N16 focused-one regression screen

Status: independently recomputed screening evidence only. This report grants no retention, absolute budget pass, lifecycle acceptance, visual proof, or final acceptance.

## Inputs and verification

Only these two immutable archives were read:

| cell | archive tar SHA-256 |
|---|---|
| baseline focused-one | `d227bbe6aa59b9a009c1cb18814c79beb93083e92ab5b530c3ed030f28cb675f` |
| candidate focused-one | `211fdffc60fd9b2ddfd0b01614e0f41d46e9b61797c8e1b5865a2fb08a89ca1a` |

Both archives contain 22 manifest-listed files; every listed length and SHA-256 matches. The script independently invokes `qualify_control` with `counting=False, diagnostics=False`: both cells exit 0, qualify 16/16, and have no qualification errors. Native-bound records are independently retained as `status=bound`, `binding_ok=true`, `qualified=true`, exit 0. Original Windows paths remain literal evidence and were not reopened locally.

The exact binary/build/source identities, fixture/cache/server identities, native conditions, geometry, and settings are retained in the JSON table. Baseline uses binary `e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5`, build commit `9268890217d968cfeb7c66ebb11dd5c3dd2c084f`, host source aggregate `84054d9c02394d959cd84681dd85f3589d3a559b94bb37856c6f87cb0629269a`, and client commit `abb811bd0afa1acd99319ccd5bc36bfb241080f9`; candidate uses binary `a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f`, build commit `fb3589ac28583242b999ac864ea69c4ef8fa5923`, host source aggregate `189dac149beb6f258f5a79bdf09de43818556a5f06ffa783a608f0113a64d8a4`, and client commit `fd956c91bf09e059359c8e182a33583e2c626cd3`. Both are panel N=16 active, 30 s warmup/120 s requested observation, focused-one, same nav pack/cache identity, no nav captures, Intel Vulkan, and one actual GPU renderer with 16 active slots.

## Whole observations

| cell | elapsed endpoints (s) | duration (s) | samples | RSS median/max MiB | peak-field max MiB | private commit median/max MiB | GPU tracked median/max MiB | CPU cores |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| baseline | 122.6818798..241.2606363 | 118.5787565 | 118 | 779.9707 / 1005.1641 | 1006.7188 | 1050.3789 / 1050.7578 | 17.6039 / 17.6039 | 0.5417022 |
| candidate | 114.1094341..232.5906787 | 118.4812446 | 119 | 789.3320 / 954.5703 | 960.1172 | 1004.3203 / 1005.9219 | 17.6805 / 17.6805 | 0.5385874 |

CPU is independently recomputed as `(last user + system - first user - system) / matching elapsed delta`. RSS, peak RSS, private commit, and GPU-tracked memory are separate; none is subtracted from another.

## Common harness-elapsed window

The common elapsed intersection is `122.6818798..232.5906787 s` (109.9087989 s). Actual per-side selected endpoints and counts are preserved in the JSON table: baseline `122.6818798..232.2057215` with 109 samples; candidate `123.1248318..232.5906787` with 110 samples. This is the required elapsed alignment, not an assertion of equal startup age.

Candidate minus baseline in the common window:

| metric | delta | relative to baseline |
|---|---:|---:|
| RSS median | -15.0469 MiB | -1.8817% |
| RSS max | -50.5938 MiB | -5.0334% |
| peak RSS field max | -46.6016 MiB | -4.6291% |
| private commit median | -45.1641 MiB | -4.2998% |
| private commit max | -44.8359 MiB | -4.2670% |
| GPU tracked median | +0.0766 MiB | +0.4353% |
| CPU process cores | -0.0025697 | -0.4777% |

The whole-observation RSS median is +1.20% for the candidate, while the common-window median is -1.88%. This reversal, different startup ages, and falling resident values are why the focused memory result remains inconclusive rather than a positive retention result.

## Contained per-slot readers and limitations

The reviewed readers were rerun for whole and common windows, with ordinal/generation mapping and per-slot simulation/fine-decode/GPU role counters retained in the JSON table. Scheduling has all 16 slots available with diagnostic p99 upper bound 21 ms on both sides. Fine decode rows are contained and available diagnostically, but complete coverage is not established; dropped, lost, pending, boundary exclusions, and p99 bucket bounds are preserved per slot. GPU completion is diagnostic callback timing, not presentation or scanout, and the stable-interval gate is unavailable because complete qualified stable coverage is absent. Input is unavailable on both sides: `no_slot_with_available_input_p99`; no input samples are invented.

Managed process/cache accounting is available. Instrumentation-overhead calibration and the standalone missing-resource-provenance acceptance gate remain unavailable. No final CPU, live visual, lifecycle, input, final-target, or resource-acceptance gate is claimed.

## Classification and bounded confirmation boundary

- Intended RSS reduction: **inconclusive**. Common RSS and private commit are lower for the candidate, but the single pair cannot separate boxed-field retention from readiness/resident decay.
- CPU 5% non-regression: **inconclusive**. The descriptive common CPU delta is -0.4777%, inside the margin, but repeated variation and clean overhead calibration are insufficient for acceptance.
- p99 2 ms non-regression: **unavailable**. Fine readers do not provide complete paired proof, and input is absent.

At most one predeclared longer confirmation stage is justified: one focused-one baseline followed by candidate, each with the same 30 s warmup and **600 s observation**, fresh preflight, identical fixture/cache/server/geometry/Intel Vulkan conditions, and no reverse rerun or additional short pairs. The question is whether candidate and baseline converge to distinct stationary RSS/private-commit levels after startup decay. Stop and reject if either cell fails exit 0, native binding, 16/16 qualification, exact conditions, or common overlap; retain nothing if candidate common RSS is not lower, CPU exceeds baseline by more than 5%, or required fine-gate coverage remains unavailable. This is a recommendation only; it does not claim that the longer stage was run.

Reproduce with:

`python3 docs/memory/windows-tile-boxed-focused-screen-analysis.py`

Machine-readable hashes, exact provenance, whole/common endpoints and durations, separate resource metrics, per-slot counters/p99s, qualification output, and limitations are in `docs/memory/windows-tile-boxed-focused-screen-table.json`.
