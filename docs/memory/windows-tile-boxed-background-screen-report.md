# Native boxed-tile N16 focused-plus-background ABBA screen

Status: independently recomputed screening evidence only. This report does not grant retention, provisional acceptance, final acceptance, absolute RSS/CPU budgets, lifecycle acceptance, or visual proof.

## Inputs and verification

The analysis read exactly these four immutable archive directories, and no other tile-boxed archive directory:

- `baseline-focused-plus-background-nativecheck-20260908-0103`
- `candidate-focused-plus-background-nativecheck-20260908-0103`
- `candidate-focused-plus-background-reverse-20260908-0116`
- `baseline-focused-plus-background-reverse-20260908-0116`

Each archive has 22 manifest-listed files; every local file hash and length matched its `archive-manifest.json`. Tar SHA-256 values are recorded in the JSON table: baseline forward `06972a6d54e2294ab224b4f25348b78bc5b25c0db7a894377ce167ec6db2317f`, candidate forward `10230c08b6c4faf37340e006d62555c6cb2b34cc8045e5f64350b415e6164f28`, candidate reverse `29f1bccab7b2056ed2b6502405920478fcee729fbec8f54c56ba83e5ed6d0650`, and baseline reverse `dcae9b20593b8d8cdbc92e84eb5490463eee28439e0ee03074b09a4c70a040c8`.

All four native-bound records state `status=bound`, `binding_ok=true`, `qualified=true`, exit code 0, and native Intel Vulkan startup. The recorded Windows paths remain literal evidence; they are not reopened or rewritten on macOS. Build provenance is taken from each artifact's exact frozen manifest/binary/source fields, not checkout labels. Baseline binary identity is host `9268890` / client `abb`; candidate is host `fb3589a` / client `fd` with binary `a9b581...`; the full values and source manifests are in the JSON table.

The run configuration is identical: panel, N=16 active, 30 s warmup, 120 s requested observation, focused-plus-background, no allocation counting/sidecar/owner census/nav PNG, and scheduling/render/GPU-completion/responsiveness/fine/failure capture enabled. Every qualification boundary has 16 running, error-free slots and `ready=16`, `active=16` in the selected samples.

## Selected observation ranges

The selected range is the raw `samples.jsonl` interval contained by the independently read `observe-start` and `observe-end` qualification boundaries. CPU is `(last user + system - first user - system) / (last sample elapsed - first sample elapsed)`; current RSS median/max, peak RSS field, private commit, and GPU tracking remain separate metrics.

| Cell | Elapsed endpoints (s) | Duration (s) | Samples | RSS median / max MiB | Peak-field max MiB | CPU cores | GPU tracked median / max MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| Baseline forward | 103.2073198..221.7025809 | 118.4952611 | 119 | 2172.9805 / 2484.7852 | 2486.5703 | 0.6357058 | 142.8885 / 143.0433 |
| Candidate forward | 137.3414382..255.9378206 | 118.5963824 | 119 | 1398.5625 / 1659.1016 | 1731.1406 | 0.6056519 | 142.8182 / 142.9449 |
| Candidate reverse | 112.2408380..231.9926042 | 119.7517662 | 120 | 1533.6484 / 1718.8633 | 1722.3555 | 0.6199021 | 143.4410 / 143.4885 |
| Baseline reverse | 103.2066542..221.7968501 | 118.5901959 | 119 | 2139.7813 / 2490.9063 | 2494.6914 | 0.6405989 | 143.2087 / 143.5669 |

The common harness-elapsed intersection across all four runs is exactly `137.3414382..221.7025809` (84.3611427 s). Selected common samples are baseline-forward 85, candidate-forward 84, candidate-reverse 84, baseline-reverse 84. Common-range RSS median and CPU deltas (candidate minus baseline) are:

- Forward AB: RSS `-424.8457 MiB`, peak-field max `-755.4297 MiB`, private-commit median `-786.7305 MiB`, GPU median `-0.1696 MiB`, CPU `-0.0262048` cores, client ticks `-668`, UI draw/frame delta `-41`.
- Reverse BA: RSS `-416.1660 MiB`, peak-field max `-772.3359 MiB`, private-commit median `-768.1680 MiB`, GPU median `-0.0161 MiB`, CPU `-0.0305912` cores, client ticks `+143`, UI draw/frame delta `+9`.

Whole-observation RSS median deltas are `-774.4180 MiB` forward and `-606.1328 MiB` reverse. The large whole/common difference and baseline/candidate endpoint offsets demonstrate the startup/readiness/falling-RSS confounder; two repeats are not a noise distribution.

## Cadence, counters, p99, and missing evidence

Per-slot ordinal mapping, contained histogram-delta accounting, endpoint coverage, histogram resolution, and all reader-produced scheduling/decode/input/GPU details are preserved under each cell's `native_bound.endpoint_notes` and `qualification` sections in `windows-tile-boxed-background-screen-table.json`. The raw qualification files contain only the two boundary rows, so no extra boundary samples were invented. Simulation/client counters and UI draw/frame counters are reported above from contained raw endpoints; renderer role is ordinal 0 full-rate focused GPU and ordinals 1..15 background GPU at the configured 1 fps policy.

The reviewed evidence does not provide a complete accepted per-slot fine decode/input transition distribution: missing input samples and incomplete decode/transition coverage remain explicitly unavailable. GPU callback/completion evidence is diagnostic and is not physical presentation/scanout proof. The resource gate is unavailable because helper/resource provenance is missing; private commit and GPU tracking are not subtracted from RSS. No rendered image is inferred from logs, and these CLI archives contain no visual capture proof.

## Classification and next action

- Intended RSS reduction: **inconclusive screening evidence**. Both AB and BA common ranges are lower for the candidate, but the magnitude is confounded by readiness/startup age and resident decay; the candidate's private-commit reduction and near-equal GPU tracking are descriptive, not an RSS decomposition.
- CPU 5% non-regression: **inconclusive**, not silently passed. Common-range CPU is lower by about 4.1% forward and 4.8% reverse, but the evidence lacks the required clean resource-overhead provenance gate and only has two repeats.
- p99 2 ms non-regression: **unavailable** because complete matched fine endpoint coverage is not established.
- Bounded next action: run the focused-one regression pair under the same frozen provenance and controls. Consider at most one longer confirmation stage only if that pair supports retaining the candidate. Do not grant provisional retention or claim absolute resource budgets pass.

Reproduce the computation with:

`python3 docs/memory/windows-tile-boxed-background-screen-analysis.py`

Machine-readable output, input hashes, manifest checks, exact endpoints, common-range metrics, provenance summaries, and retained endpoint notes are in `docs/memory/windows-tile-boxed-background-screen-table.json`.
