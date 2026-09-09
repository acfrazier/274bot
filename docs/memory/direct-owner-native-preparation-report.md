# Direct owner native preparation — frozen source handoff

Task: `t_0a97cf2a`. Branch: `codex/memory-diagnostics`. Status: frozen source prepared and local source check passed; Linux generated qualification and live qualification remain false. No production implementation, existing overlay receipt, submodule commit/gitlink, remote, STATE, native/SSH/live/account/cache/server, or frontend operation was performed.

The 20 scoped preparation/report/artifact files first entered history in `03df80d7cc43f0b2eb7310474482f5ce801b6b5d`. A shared-index race swept the already-staged files into a parallel root commit together with the unrelated root-owned `docs/memory/nav-singleton-f1-result.md`. Root directed that combined commit be preserved without reset/amend/rewrite and treated as this preparation's evidence commit. Review scope is the 20 paths under `diagnostics/direct-owner-native-preparation/` plus this report; the F1 report has a separate review and is excluded here.

## Prior blocked result retained

The first attempt correctly stopped when original H plus the exact read-only patches produced two host files whose full hashes differed from the moving reviewed branch. Its fail-closed receipts remain unchanged at `diagnostics/direct-owner-native-preparation/artifact/{provenance-audit.json,preparation-result.json}`. The discrepancy is the already-recorded tiled-navigation test adaptation inherited by reviewed commit `3cdc3e4`; importing it into the original-H derivative is forbidden.

Root's independent full-diff audit established that both files have byte-identical production prefixes and differ only after their unique top-level `#[cfg(test)] mod tests` boundaries. This resumed preparation independently recomputes that distinction from the materialized and reviewed bytes. It does not edit the old receipt or accept a general mismatch exception.

## Frozen provenance and derived identities

The successful command was:

    PYTHONDONTWRITEBYTECODE=1 python3 -B \
      diagnostics/direct-owner-native-preparation/prepare_frozen_source.py prepare \
      --output-dir diagnostics/direct-owner-native-preparation/artifact/frozen

It uses separate temporary `GIT_INDEX_FILE` indexes, `read-tree`, sequential `apply --cached --check` / `apply --cached`, and `checkout-index` into a temporary owned prefix. Archive inputs come only from those indexes. The original identities and unchanged patch receipts are:

- host original `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`;
- client original `3456edc8dabf7b25ada78110ffa56327af9f67a4`;
- reviewed host `3cdc3e4fbeb960fe4c270eb994ce22746787eb2d`;
- reviewed client `5c73a4a27f3d72834c2c2a071668eb197eb39fd9`, a direct child of original C;
- root gitlink binding `edc3a3e`, which points to that reviewed client;
- `host-initial.patch`: 104314 bytes, SHA-256 `5229bab8d7c59046dbb8e134daca2bd9e570043c3016e3277e2db6369fdbb8a9`;
- `host-correction.patch`: 104127 bytes, SHA-256 `d2646cc175a376a295de2bd9261c4c8d8c7a2af57bbeb52171732b73480f4917`;
- `client-overlay.patch`: 19104 bytes, SHA-256 `0a5d43ed382c43d293e17824b1565dc4a5258d5b17971edaea55ad6a58324430`.

All three patch applicability checks pass from the declared originals. All five client members and 19 host members have full-file identity equal to both the unchanged overlay receipt and the reviewed commit. Exactly two host members use the bounded test-only derivation rule:

| member | original-H + exact patches | reviewed full file | production boundary |
|---|---:|---:|---:|
| `crates/host-play/src/lib.rs` | 357877 bytes; `f997f22f00bb205cdad74eec70d9dfc5750c3f8f80d9ba52c024f5ea4ea9120f` | 361346 bytes; `096753fc31636e829058e33f54770090810688346c9be47b4ed49193813801ee` | line 4270; prefix SHA-256 `37e1d14040400d818ee56aa11372f284f7f98ce008fd3d4145b16fa57d0503e5` |
| `crates/host-play/src/memory.rs` | 161591 bytes; `76e633acf9d4dcd1fd03f1a57ea722f5111025790071679119e7e5932ece2991` | 161447 bytes; `74b7a18164f1b6861697ec8c651c2f81a400dbd41c8bf772cdab29ff4b3bb923` | line 2077; prefix SHA-256 `1af24d12119b500b802e02c63f0f33055b49fda16c03a7eae54121a7af06aa69` |

For each, the materialized full hash, reviewed full hash, unique test boundary, production-prefix hash, and prefix byte equality are all mandatory. The complete independently generated difference is `artifact/frozen/derived-vs-reviewed-test-only.patch`: 14080 bytes, SHA-256 `bba0b19eecc45beca79b94f204953535b5c0b259cc6bbdee5e2f6a96cc92a389`. `artifact/frozen/provenance-audit.json` records `verified: true`, exact counts 19/5, and exactly these two derivations. The generated host-play observer tests remain full-file identical to reviewed `3cdc3e4` in the derived tree.

## Deterministic source artifact and manifest

`prepare_frozen_source.py verify` passed against the result. A regression test prepares the source twice into independent fresh directories and requires identical archive bytes and hashes.

- archive: `diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-direct-owner-source.tar.gz`;
- bytes: 5075876;
- SHA-256: `2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d`;
- source members: 1124;
- manifest: `artifact/frozen/frozen-source-manifest.json`, 327668 bytes, SHA-256 `ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e`.

The manifest records every source member's path, mode, Git blob, byte count and SHA-256; the exact overlay diff scope and patch applications; original/reviewed Git bindings; all six feature manifests and parsed feature tables; the TUI package/binary/feature contract; qualification-support identities; and every archived `Cargo.lock`. The archive is a sorted PAX tar with zero uid/gid/mtime and deterministic gzip filename/mtime. It includes no Git metadata, Cargo target, runtime output, account/vault/cache contents, or server binaries; all members originate in the frozen Git indexes.

Frozen lock identities are:

- root `Cargo.lock`: 173350 bytes, SHA-256 `03059d02f8274c8fc9cd7e2f69a48f317e3f44417570ecf4730b31710f8d4b83`;
- client `vendor/fr-client-rust/Cargo.lock`: 83056 bytes, SHA-256 `015530e3685e8175fe5ec12b41cf37a40ee116557d575cd38b6852926ec47cc7`;
- tracked diagnostic `docs/memory/diagnostics/linux-gpu-shade-probe/Cargo.lock`: 45750 bytes, SHA-256 `d0285dae7f5cef20edf41cd8473141767d33b5c82e1a943c97cae4789ae8e41a`.

## Local frozen-tree verification

The local command used a fresh output tree outside the repository:

    PYTHONDONTWRITEBYTECODE=1 python3 -B \
      diagnostics/direct-owner-native-preparation/qualify_linux.py \
      --archive diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-direct-owner-source.tar.gz \
      --manifest diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-source-manifest.json \
      --output-dir /tmp/direct-owner-source-check-t_0a97cf2a-r1008d \
      --mode source-check

Compact JSON and logs were copied to `artifact/local-source-check/`; extracted source and its Cargo target were not copied into the repository or archive. Results:

- source-member and all lock checks: passed, 1124/1124;
- `cargo check --offline --locked --target aarch64-apple-darwin -p tui --features memory-profile-no-alloc,memory-owner-capture`: exit 0 in 35.168 s;
- frozen `host-play` generated direct-owner integration test with the same feature pair: exit 0 in 32.535 s, 2 passed;
- reviewed generated protocol validator: exit 0, 3 passed;
- all recorded steps passed; locks unchanged; runtime capture defaulted off;
- generated observer: 0 allocations, 0 requested bytes, 1188334 ns thread CPU, 1225875 ns wall, 262144 scratch bytes; the 5 ms thread-CPU criterion passes and the fixture explicitly reports `native_qualified:false`.

The local tool receipt binds macOS 15.7.9 arm64, target `aarch64-apple-darwin`, rustc 1.98.0 commit `88d9e12ae178fab0fb5cc050a94da85685d449ea`, Cargo 1.98.0 commit `797e8a9bca276c1c9f9f738d2a20f484fa4eea9d`, Python 3.9.6, invoked/resolved tool paths and executable hashes, and qualification-script hash `3a9857bf0a52a32d1b24bcdaf3509fc22f2e034a688ed5c967f4a5207c46d221`. Existing host cadence dead-code warnings remain in the compile logs; there was no new error.

Tool regressions also pass:

- `PYTHONDONTWRITEBYTECODE=1 python3 -B diagnostics/direct-owner-native-preparation/test_prepare_frozen_source.py -v`: 1 passed, including two-build deterministic equality;
- `PYTHONDONTWRITEBYTECODE=1 python3 -B diagnostics/direct-owner-native-preparation/test_qualify_linux.py -v`: 4 passed;
- `PYTHONDONTWRITEBYTECODE=1 python3 -B diagnostics/direct-owner-native-preparation/support/test_validate_direct_owner_capture.py -v`: 3 passed.

This is frozen-tree macOS evidence, not the earlier current-checkout 14-suite evidence and not Linux qualification.

## Untouched production-tree checks

During the successful prepare operation, host HEAD, client HEAD, both real index hashes, both tracked-status strings, and moving-checkout root/client lock hashes were byte-identical before and after. The captured host HEAD was `cf5cbddc97fa02ed01b50ac9a60d5ce87286bc36`; concurrent scheduler commits continued moving branch HEAD afterward, but fixed Git-object archive inputs are unaffected. The real host index SHA-256 remained `3d2645f932f0664dac051caff8d7592d659f7915e9544c548c8f4575d92ef9c5`; the real client index SHA-256 remained `0c0ad289f3671eb2d6b8364df44960531fbec528104a37b788133955965e24ce`. Client tracked status remained empty; host tracked status remained the same owned preparation-script modification on both sides. No production source or actual index was staged or modified.

## Root-executable Linux qualification

`qualify_linux.py --mode linux` requires native Linux x86_64, the exact archive and manifest, the committed reviewed validator support, executable rustc/Cargo/Python/file/readelf/ldd tools, offline locked dependencies, and a fresh absolute output path. Each bounded step has a 900-second timeout, writes its log before continuing, kills only its own timed-out process group, and retains partial steps plus a false qualification on failure. `CARGO_TARGET_DIR` and extraction stay under that fresh output. LIVE/account/vault/cache/server variables are removed; `BOT_MEMORY_OWNER_CAPTURE=0` and `BOT_CPU=1` are explicit. No account, frontend, cache, server or live test can be launched by the matrix.

The Linux matrix records rustc/Cargo/Python versions, invoked/resolved executable paths and hashes, host target, Cargo feature tree, source/locks, built `tui-play` bytes/SHA-256 plus `file`/`readelf`/`ldd`, and exact step/log hashes. Its required generated coverage is mapped to named steps:

- observer allocation/thread CPU: isolated `host-play-generated-observer`, covering the real pre-observe and nav/COW seams with generated empty owners; it cannot represent populated native scene cost;
- owner budget/mailbox/output guards: focused api/host/host-play owner tests cover checked capacity formulas, visit/deadline limits, fixed rows, three-request/full/stale-slot mailbox rejection, fixed COW scratch/phase timing, and cumulative 256 KiB owner-output failure;
- Stop/cleanup: generated script Stop integration verifies fingerprint clearing and unknown builder capacity; the original-H managed tests verify invalid-spec/launcher-mismatch pre-launch rejection, early failure without retry, deadline cleanup, and refusal to signal a foreign-identity process;
- protocol: the reviewed generated positive fixture and negative protocol/order/cap/file-identity mutation tests;
- native build: exact feature tree plus offline locked TUI build/check and binary identity.

Client unit and integration tests, api integration, script fingerprint, host-play owner tests, and the bounded TUI feature suite are separate matrix steps; host tests are not substituted for client integration. `linux_generated_qualified` requires every step, lock stability, observer criteria, mapped coverage, and a present binary. `live_qualified` remains false unconditionally.

## Platform and release boundary

This macOS run cannot establish Linux qualification. Root must execute the committed Linux script on the admitted native host and independently review the full receipt before considering plan §5's separate live prerequisites. A green generated Linux result still does not admit Concord resources, account/cache/server/controller identities, readiness/workload/normal Stop, live runtime guards, or the one-attempt diagnostic. Root retains those decisions, real releases, result review, and the final whole-branch Grok 4.6 pass. No direct-owner live procedure is released by this task.
