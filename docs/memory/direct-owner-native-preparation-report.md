# Direct owner native preparation — blocked provenance handoff

Task: `t_0a97cf2a`. Branch: `codex/memory-diagnostics`. Status: blocked before frozen archive creation. No production implementation, existing receipt, submodule commit/gitlink, remote, STATE, native/SSH/live/account/cache/server, or frontend operation was performed.

## Result

The exact original-object staging contract cannot currently satisfy the read-only reviewed receipt. Preparation stopped before archive creation and before any frozen-tree Cargo command, as required by the fail-closed brief.

The preparation command was:

    diagnostics/direct-owner-native-preparation/prepare_frozen_source.py prepare \
      --output-dir diagnostics/direct-owner-native-preparation/artifact

It used temporary `GIT_INDEX_FILE` indexes, `read-tree`, sequential `apply --cached --check` / `apply --cached`, and `checkout-index` under a temporary directory. All three bound patches passed their applicability checks from the declared originals:

- host original: `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`;
- client original: `3456edc8dabf7b25ada78110ffa56327af9f67a4`;
- `host-initial.patch`: 104314 bytes, SHA-256 `5229bab8d7c59046dbb8e134daca2bd9e570043c3016e3277e2db6369fdbb8a9`;
- `host-correction.patch`: 104127 bytes, SHA-256 `d2646cc175a376a295de2bd9261c4c8d8c7a2af57bbeb52171732b73480f4917`;
- `client-overlay.patch`: 19104 bytes, SHA-256 `0a5d43ed382c43d293e17824b1565dc4a5258d5b17971edaea55ad6a58324430`.

Git object and binding checks passed: original H points to original C; root binding `edc3a3e` points to reviewed client `5c73a4a27f3d72834c2c2a071668eb197eb39fd9`; the reviewed client commit is a direct child of original C. The copied protocol validator/test support is byte-identical to reviewed host `3cdc3e4fbeb960fe4c270eb994ce22746787eb2d`.

The exhaustive member audit passed all 5 client members and 19 of 21 host members. Two host members fail:

| member | original-H + exact patches | read-only receipt / reviewed `3cdc3e4` |
|---|---:|---:|
| `crates/host-play/src/lib.rs` | 357877 bytes; `f997f22f00bb205cdad74eec70d9dfc5750c3f8f80d9ba52c024f5ea4ea9120f` | 361346 bytes; `096753fc31636e829058e33f54770090810688346c9be47b4ed49193813801ee` |
| `crates/host-play/src/memory.rs` | 161591 bytes; `76e633acf9d4dcd1fd03f1a57ea722f5111025790071679119e7e5932ece2991` | 161447 bytes; `74b7a18164f1b6861697ec8c651c2f81a400dbd41c8bf772cdab29ff4b3bb923` |

The mismatch is reproducible and explained by the overlay construction lineage. Initial instrumentation commit `70a0954` has parent `cbd34c494b5b01bcd4b6c70e439baa131ec3e629`; that parent already differs from original H in these same two files by 220 insertions / 79 deletions. The differences are tiled-navigation test adaptations (`WorldCollision::from_packed_parts` and related tests). `host-initial.patch` contains only the commit delta from `70a0954^` to `70a0954`, and `host-correction.patch` contains correction hunks. Applying them to original H therefore correctly omits the intervening tiled-navigation source, but the receipt member hashes were computed from the reviewed moving branch and include it. Importing the missing H-to-parent delta would violate the explicit instruction not to stage current tiled-navigation source.

Full identities for every declared member are in `diagnostics/direct-owner-native-preparation/artifact/provenance-audit.json`; the fail-closed summary is `artifact/preparation-result.json`.

## Verification performed

- `PYTHONDONTWRITEBYTECODE=1 python3 -B diagnostics/direct-owner-native-preparation/support/test_validate_direct_owner_capture.py`: 3 passed.
- `diagnostics/direct-owner-native-preparation/qualify_linux.py --help`: exit 0; import/CLI smoke only.
- Root and client `git status --short --untracked-files=no`: empty after failed preparation.
- The preparation receipt records `production_tree_unchanged: true`; the actual indexes were never selected as `GIT_INDEX_FILE` for staging/apply.
- Moving-checkout lock hashes remained unchanged across preparation: root `Cargo.lock` SHA-256 `03059d02f8274c8fc9cd7e2f69a48f317e3f44417570ecf4730b31710f8d4b83`; client `Cargo.lock` SHA-256 `015530e3685e8175fe5ec12b41cf37a40ee116557d575cd38b6852926ec47cc7`.

No source archive exists, so no archive byte count or SHA-256 is claimed. No frozen `cargo check` or generated observer run was attempted after the provenance failure. Existing 14-suite evidence remains current-checkout evidence only.

## Required root correction

Root must supply a reviewer-approved, original-H-based host overlay receipt whose final materialized member identities are internally consistent. The correction must preserve only the reviewed direct-owner instrumentation and must not add the intervening tiled-navigation delta. Valid resolution requires either a dedicated reviewed H-plus-instrumentation source commit/tree or corrected member bindings with an explicit independent proof that the direct-owner hunks match reviewed `3cdc3e4`. This worker is not authorized to edit the existing receipt files or production source.

After that correction, rerun preparation into a fresh output path. Only a successful exhaustive member audit may produce the deterministic source archive/manifest and unlock the local offline locked TUI check. The root-owned Linux script then records compiler, target, Cargo feature graph, tools, binary, source, locks, generated observer allocation/thread CPU (5 ms bound), owner budget/mailbox/output guards, Stop/managed cleanup fixtures, and protocol validator results in a fresh output tree. It forces runtime capture off except for the isolated generated fixture and never launches accounts/frontends/live tests.

## Platform and release boundary

This host is macOS and cannot establish native Linux qualification. Even a later green generated Linux receipt will not qualify a live run. Root still owns Concord identity/resource admission, exact cache/account/server/launcher guards, workload/readiness/normal Stop and owned-process cleanup, one-attempt live release, result review, releases, and the final whole-branch Grok 4.6 pass. No direct-owner live procedure is released here.
