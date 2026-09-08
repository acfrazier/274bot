# Native N1 focused CPU fallback paired proof

Date: 2026-09-08
Commit under review: `83561ac` (`codex/memory-diagnostics`)
Classification: functional evidence only

## Requirement and boundary

This check verifies the completed native Windows N1 `panel`, `active`, `focused-one`
CPU-fallback pair required as a bounded rendering/functionality proof. It does not
claim CPU/RSS optimization, GPU or p99 acceptance, input coverage, focus-detach,
FBO-freeze, lifecycle, scaling, or final campaign acceptance. The CPU renderer's
configured/default cadence is preserved; no CPU performance budget is introduced.
The earlier `baseline-focused-one-cpu-native-20260908-0155` archive remains partial
(missing complete server configuration) and is excluded from this pair.

The original Windows paths are retained as evidence. This Mac run did not attempt
native binding, SSH, launch, restore, or output rewriting.

## Independent checks

The reproducible checker `windows-tile-cpu-paired-proof-check.py` recomputes every
entry in both local `archive-manifest.json` files. Both manifests contain 28 files;
all files were present and their recorded byte lengths and SHA-256 digests matched:

| role | cell | archive SHA-256 recorded by the native receipt | manifest files |
|---|---|---|---:|
| baseline | `baseline-focused-one-cpu-native-20260908-0215` | `a39b8a50ff4a83e3b641a780f4ed58a3352a265dc86897e3202ddcb00d4d798a` | 28/28 |
| candidate | `candidate-focused-one-cpu-native-20260908-0215` | `c267376258bef262ad130f5efb8818fddf3c31c1b07d70a86b68762f970294d4` | 28/28 |

The checker also hashes the sibling `.tar.gz` files directly. Their measured local
SHA-256 values match the native receipt values above; this is a byte-level check of
the actual archived evidence, in addition to the extracted manifest-file checks.

Both native completion receipts have `exit_code: 0`, `functional_only: true`, and
`performance_acceptance: false`. The native-bound records independently report
`status: bound`, `binding_ok: true`, `reason: null`, `pair_eligible: false`, and
`match_keys_missing: null`; their qualification objects report `qualified: true`.

The actual and requested backend agree in both host-condition records:
`cpu_fallback`. Each has one active slot (`n: 1`), `focused-one` policy, panel
frontend, active workload, CPU renderer ordinal 0 enabled/present/drawing/full-rate,
and no GPU-completion profile. The native preflight identifies the Intel adapter
(`Intel(R) Graphics`) on Windows 11; the requested CPU fallback is therefore not
inferred from a filename.

The qualification boundaries were read from each native
`samples.qualification.jsonl` and rerun locally with the required command:

```
python3 docs/memory/qualify_control.py <raw-run-01> --no-write
```

Both reruns exited 0 with `qualified: true`, no errors, one running slot at both
`observe-start` and `observe-end`, and complete active-slot identity. The local
qualification results were:

| role | observe boundary (s) | local observation (s) | client ticks/slot/s | diagnostic steal gain |
|---|---:|---:|---:|---:|
| baseline | 44.7222791 → 164.7259169 | 118.6122029 | 48.9241 | 17 (`live43480_0`) |
| candidate | 43.9736540 → 163.9907355 | 118.6684865 | 48.9262 | 8 (`live130c0_0`) |

The steal values above are the qualification observation values. They must not be
confused with the later navigation PNG captions: the baseline later-navigation
capture shows 24 steals, while the candidate bank and return-route captures show
12. A later candidate observe-end paint records 13, but that is not the caption in
those PNG captures.

## Server, cache, source, and binary provenance

Both match-key records contain complete server configuration: loopback bind
`127.0.0.1`, Node `24.19.0`, and hashes for `world.json`, `maps` addition, and
`wordenc` addition. The server identity records also freeze server commit
`4c95f87efe00b068cadbd229d94736626907bd1a`, the native process start identity,
configuration hash, and loopback port. Cache/catalog/nav-pack/nav-flags identities
are present in each native-bound record and match the paired receipt evidence.

Frozen panel/client identities recorded by the native binding are:

| role | panel binary SHA-256 | build/source commit | source-manifest SHA-256 | client commit | client-source SHA-256 |
|---|---|---|---|---|---|
| baseline | `e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5` | `9268890217d968cfeb7c66ebb11dd5c3dd2c084f` | `84054d9c02394d959cd84681dd85f3589d3a559b94bb37856c6f87cb0629269a` | `abb811bd0afa1acd99319ccd5bc36bfb241080f9` | `ea6402702a56dc29359bf4effd9ca87407734fc9d940bfabdefb92cec8bf41ee` |
| candidate | `a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f` | `fb3589ac28583242b999ac864ea69c4ef8fa5923` | `189dac149beb6f258f5a79bdf09de43818556a5f06ffa783a608f0113a64d8a4` | `fd956c91bf09e059359c8e182a33583e2c626cd3` | `fad2e79248c8d62cff2ceba5c757ef555705e85d3dc6797586c9841d9dbfb53e` |

The differing candidate binary/source identities are recorded rather than silently
collapsed into a same-binary claim. The native receipt also records the checkout
host commit `b4b686fd8765cc9d1aa880346f440bd6b781246e` and marks it as not the build
authority.

## Runtime restore receipt audit

The root paired receipt's `runtime_restore` is consistent with the three preserved
runtime documents under
`docs/memory/diagnostics/cpu-proof-runtime-26426b0/`. The original manifest,
staging receipt, and restore receipt each describe the same four named files and
the same original/CPU SHA-256 pairs; the embedded root receipt repeats those four
rows byte-for-byte. The measured document SHA-256 values are:

| document | SHA-256 |
|---|---|
| `original-runtime-manifest.json` | `2f96e500e461d506587f16bf7ea0f4426d7eb18346a21f9142f78d64544afeb6` |
| `runtime-staging-receipt.json` | `c109ba443ea76c3affcf2fb03928df4d1e35edfbb2c9bbb5f604a3aff7818948` |
| `restore-receipt.json` | `c79cb9ebfe8290d6361e54fc7c979e1432787c4fd7630a33cf434ed594af005a` |

The restore receipt records `original_runtime_restored: true`, inactive native
frontend, VM state `Off`, and `performance_acceptance: false`; the root receipt
repeats those values. This is receipt and hash consistency evidence only. A local
Mac checkout cannot assert the current state of the remote Windows VM or workspace,
and this task did not perform native restore or SSH actions.

## Visual functional inspection

I independently used the available `vision_analyze` image-inspection tool on all
six PNG captures,
three per role, rather than inferring rendering from filenames or JSON metadata.
The baseline and candidate scene-ready captures show a populated textured market/
guard scene, one visible bot, minimap, status/debug panels, and overlays. The bank
captures show the populated bank interior and the food-restock state with 22 food
items visible in the inventory. The return-route captures show the route with the
expected building/wall occlusion and populated minimap/UI. Across all six captures
there is no obvious blank framebuffer or corrupt geometry.

This is visual functionality evidence only; it does not establish animation
smoothness, focus behavior, input correctness, frame cadence acceptance, or a
freeze/lifecycle result.

## Result

The pair is sufficient for the bounded claim: both fresh native N1 CPU-fallback
runs completed, bound to the intended evidence, qualified locally, retained complete
28-file manifest evidence, froze requested/actual backend and slot boundaries,
recorded complete server/source/binary identities, and rendered the expected
functional scenes. It is not evidence of a performance improvement or any final
low-end finish-line gate.
