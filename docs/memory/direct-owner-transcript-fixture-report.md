# Direct-owner snapshot transcript fixture correction

## Scope and disposition

This card adds one separately hash-bound integration-test-only overlay under
`diagnostics/direct-owner-native-preparation/transcript-test-overlay/`. It does
not modify the frozen archive, frozen manifest, existing TUI or coverage patch,
existing coverage helper, committed golden, production host/client source,
lockfiles, `STATE.md`, submodule, or remotes. The derived test source must never
enter a production or live binary build.

The local generated result is green for the requested correction: both expected
RED probes exposed the original cache dependency, then 4/4 corrected focused
commands passed under empty and generated-prepopulated cache conditions, with
and without `snapshot-dedup`. All bounded child process groups were absent after
each command. This was Darwin arm64 evidence only; native Linux, live, and
performance qualification remain false.

## Root-cause evidence

The preserved Linux evidence is
`diagnostics/direct-owner-native-preparation/root-coverage-native-02-failure.tar.gz`,
SHA-256
`86a99fa43829eafeb286bb5f6ee2e2a830d29094d2d959d3acbfd00858549ee1`.
Within it:

- `root-coverage-native-02/result.json` has SHA-256
  `34b1ba09becac22ebfe18265b18cb570d84d26857c001d5258e46beb799719b0`;
- `root-coverage-native-02/logs/feature-off-host-play-profile.log` has
  SHA-256
  `5a7985d6f0413098eb91d4055f3d3245fc1820902f1f30518cd793fec65022e6`;
- that log records eight passing tests and
  `transcript_matches_golden_and_peer_feature_build` failing at the unchanged
  full-transcript equality assertion;
- the live transcript uses loc ids 0 and 1, while the committed golden uses
  4671 and 4672. The displayed widget, side-tab, loc-name, dirty/generation,
  gate-history, and path fields otherwise agree.

The frozen original test establishes the source of those ids:

- `crates/host-play/tests/snapshot_frame_equivalence.rs:30-50` configures
  `cache_dir="/tmp"` and constructs the fixture with `Client::new(cfg())`;
- `snapshot_frame_equivalence.rs:173-176` assigns the planted loc id from
  `cache.locs.len()`;
- frozen `vendor/fr-client-rust/crates/client/src/client/client.rs:852` loads
  that configured cache in `Client::new`;
- `client.rs:1330-1339` returns `Cache::default()` when no JAG exists and
  otherwise unpacks the cache's `config` JAG.

Thus the generated fixture's supposedly fixed ids were actually selected by the
host cache's existing loc-table length. The Linux mismatch is fixture ambient
state, not evidence of a direct-owner production regression.

## Minimal correction and source-only contract

`transcript-test-only.patch` changes only
`crates/host-play/tests/snapshot_frame_equivalence.rs`:

- before SHA-256:
  `4fd5aa58f64c555ed99657d0b37ccf65769fd3513b294a3fccbbd90a0ccab39a`;
- after SHA-256:
  `90abb21d1a3ec7f91c6bcffc59dd9312ae5d962a08c9d4d59a2f02a09ebc5d66`;
- patch SHA-256:
  `1e5ac15b25d113a180f8212fdd282bd63d76e79c14aa200560c2184106deb995`.

The corrected fixture creates `Cache::default()`, fills exactly 4671
index-aligned default `LocType` entries, and passes that generated shared cache
to the existing production `Client::from_shared` constructor. The existing
`plant_populated` path then appends ids 4671 and 4672 exactly as before on the
machine that produced the golden. No owner implementation is copied or mocked,
and all production host and host-play observation paths remain exercised.

The cache directory is an explicit generated-test path rather than `/tmp`.
`Client::from_shared` prevents cache unpack and the `/crc` probe, so the test
cannot borrow private cache data or contact a server. The source-only verifier
requires:

- exactly seven derived members after the reviewed TUI and coverage overlays;
- this integration test as the only new derived member;
- all other 1117 source members byte-equal to the 1124-member frozen manifest;
- the committed golden unchanged at SHA-256
  `783a7d5077829887147240a17545b69351ad2938c7304785489c899fb2dd81d4`;
- the full `live == golden` assertion and peer-feature `live == peer_tx`
  assertion retained;
- all three lockfiles unchanged before and after testing.

The runner also rejects a transcript patch that names any path other than the
single integration test or mentions the golden, Cargo manifest, or lockfile.

## Adversarial generated RED/GREEN comparison

The runner creates two private generated fixtures beneath its fresh output:

1. an empty directory;
2. a valid 68-byte generated `config` JAG with 13 default loc records,
   SHA-256
   `eb4d8ed6ef9eb68ea6e67b1bf0b5f3911990a4adbf029410249d0fc257be9629`.

Before applying the correction, it temporarily changes only the original test's
cache-dir seam to the explicit generated path, runs the real original
`Client::new` fixture, and restores the exact before hash. These probes are
expected RED evidence:

| Probe | Result | Observed original id | Log SHA-256 |
|---|---:|---:|---|
| empty cache | exit 101, expected failure | 0 | `4dcfb5aea2ba50273a54f603f103ceaf16303ebe451fd53b0695a1332c20f2a8` |
| 13-loc generated JAG | exit 101, expected failure | 13 | `aac63c45524191c5499c52695572ffd3838ae3220c7b65c481701e4cfd0ec704` |

The different observed ids under the two generated inputs directly falsify the
original fixture's independence from ambient cache state. The temporary probe
source has SHA-256
`f4f81fd8cff139a77396af44e24c82cfb8ec2cd2ff1ac93f714b3db5d660fc63`;
the runner refuses to apply the correction unless the source is restored to the
exact original hash afterward.

After applying the correction, the same full golden/peer test passed in all four
cases:

| Cache condition | Feature set | Result | Log SHA-256 |
|---|---|---:|---|
| empty | `memory-profile-no-alloc` | 1 passed, exit 0 | `ac998f468bcb22314e6ee73c11b90742b0b1c33768c387ae27f9ed7c777b7101` |
| empty | `memory-profile-no-alloc,snapshot-dedup` | 1 passed, exit 0 | `d3a0e9dbe0eb46845ef2f6d0a6f9a3aad7ef5f55ba59d376a1266885f6696f25` |
| generated 13-loc JAG | `memory-profile-no-alloc` | 1 passed, exit 0 | `f42a351db620933acbf7769439833ac41ef22e200f28469252811c79e9249622` |
| generated 13-loc JAG | `memory-profile-no-alloc,snapshot-dedup` | 1 passed, exit 0 | `29d6ef2175a9c5be5f6491bc1ec7c905af5cb2bc0016c094ed7547045c483e6f` |

Every command used `--offline --locked --exact --test-threads=1` and the reviewed
coverage helper's bounded process-group runner. The sanitized environment sets
`BOT_CPU=1`, `BOT_MEMORY_OWNER_CAPTURE=0`, removes live/private inputs, and uses
only the owned generated cache and target paths. No real `/tmp` cache, account,
vault, server, network, frontend, live test, or production binary was read or
run. Existing host dead-code warnings remain non-failing.

Local compact evidence is retained at
`diagnostics/direct-owner-native-preparation/transcript-test-overlay/local-focused-02/`:

- `overlay-receipt.json` SHA-256
  `5397b7277562261e290795c296a17905461b4d0ad6f9ceecff0f2c81cb79a76b`;
- `steps.json` SHA-256
  `6a58b332a911d4e40d00f75e6e28670d29d95379893374568bce5eec88c32600`;
- `original-probe-steps.json` SHA-256
  `6b2f45e3c19560abe73ff178042f06a45b840d485d0bc9d62533a00207a66249`;
- six raw logs are individually hash-bound in the receipts.

## Reproduction and root-native command

Local focused reproduction in a fresh output directory:

    PYTHONDONTWRITEBYTECODE=1 python3 -B \
      diagnostics/direct-owner-native-preparation/transcript-test-overlay/stage_transcript_overlay.py \
      --output-dir <fresh-local-output> \
      --run-focused --timeout 900

The runner is SHA-256
`c14b1b5648b993d10b0979de5fc14f49b97d147b502edcee1f91169bd51b9b54`.
It binds the immutable archive/manifest and the already-reviewed TUI patch,
coverage patch, and coverage helper by their exact hashes, delegates staging,
`verify_tree`, lock checks, and bounded commands to that reviewed helper, then
adds only the transcript-test overlay.

After same-card review, root can execute the focused probes/correction plus the
entire existing 19-command gap, feature-off, and feature-on matrix on native
Linux x86_64:

    PYTHONDONTWRITEBYTECODE=1 python3 -B \
      diagnostics/direct-owner-native-preparation/transcript-test-overlay/stage_transcript_overlay.py \
      --output-dir <fresh-linux-output> \
      --build-target x86_64-unknown-linux-gnu \
      --run-focused --run-full-coverage-matrix --timeout 900

Root should require `prepared=true`,
`original_ambient_dependence_exposed=true`, 2/2 expected probes with ids 0 and
13, 4/4 corrected focused exits zero, 19/19 existing matrix exits zero, all
cleanup `group_absent=true`, exact 1124-member/three-lock identity checks before
and after, `all_requested_passed=true`, and native platform
`Linux/x86_64`. Only then can this runner set `native_linux_qualified=true`.

A green native result remains generated functional evidence only. It does not
approve live or performance qualification, a production build from the derived
tree, account/cache/server/controller admission, or the unrelated Linux GPU
texture-brightness failure.
