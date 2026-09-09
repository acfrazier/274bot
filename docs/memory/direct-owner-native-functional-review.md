# Native owner functional evidence and GPU disposition

Independent milestone review of frozen native functional evidence and the
proposed original-client GPU defect split. Not live admission, not GPU
qualification, not whole-branch Grok 4.6, not campaign completion.

## Identity

| Field | Value |
|---|---|
| Reviewer profile | `grok46` |
| Actual model / provider | `grok-4.6` / `xai-oauth` (no task model/provider overrides, no delegation) |
| Task | `t_82ed565a` |
| Branch | `codex/memory-diagnostics` |
| Frozen evidence commit | `cc0dc0600961c3ed06b7f47c914e7596b2df8f82` |
| Workspace HEAD at review | `57ae3a7d0ac131237e017f8ef2e936b5e962d029` (STATE/controller successors after `cc0dc06`; not in scope) |
| Transcript result | `docs/memory/direct-owner-native-transcript-result.md` at `cc0dc06` (working tree identical) |
| GPU baseline report | `docs/memory/direct-owner-native-gpu-baseline-report.md` at `a02435c` (working tree identical) |
| Transcript fixture | `a3abbddf8b8f90e795eb3ff691399644e2bbc346` (same-card approved Grok 4.5 / xai `20260909_162751_9bdc09`) |
| Coverage follow-up | `e01c288c000a245947d2a9524c90e2a250f38560` |
| UTC | 2026-09-09T20:45:15Z |
| Plan | `docs/memory/direct-per-bot-owner-capture-plan.md` §§4–6 |
| Design | `docs/memory/direct-owner-managed-extension-design.md` §§1–3 |

No production/helper/assertion/STATE edits, no uncommitted controller
evaluation, no SSH/native/live re-runs, no private inputs, no remotes, no
Rust builds. Archives were hashed in place and non-source members streamed;
source trees were not re-extracted.

## Verdict

**APPROVE** native **affected-suite generated** functional evidence for the
reviewed transcript fixture on Linux x86_64.

**ACCEPT** the proposed original-client GPU defect disposition: keep the
exact failed GPU shade test **explicitly open**, and allow the bounded
**non-GPU** TUI owner diagnostic to proceed **after** the remaining
non-GPU prerequisites. This is not GPU qualification and does not convert
failed client commands into passing suites.

Keep helper `native_linux_qualified=false`. Do not treat focused
`all_requested_passed=true` or the later host-play rerun as a blanket
19-command green matrix. Live, GPU panel/regression/cadence/visual, private
preflight, and campaign completion remain withheld.

## Independent archive identities

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `root-coverage-native-02-failure.tar.gz` | 5318282 | `86a99fa43829eafeb286bb5f6ee2e2a830d29094d2d959d3acbfd00858549ee1` |
| `root-coverage-native-03-remainder.tar.gz` | 5348687 | `3b50cc0b2893aa3203a3115fdada7d225aab9fc82a4eab04309baa29b9ca3b8e` |
| `root-transcript-native-01.tar.gz` | 5337021 | `9c9b5c785b6cb065ad7f0d9562b159076c6e1beddc39575c99d3770a4f4437a8` |
| `root-gpu-baseline-and-client-01.tar.gz` | 711019 | `1d3793c0484253820523b0c0194d3beb8a0b3e2ef128d9dfd2586ea420e7f5ed` |
| Frozen source tar | 5075876 | `2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d` |
| Frozen source manifest | 327668 | `ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e` |
| Clean-build receipts tar | 5104 | `fc4a92eb2cff5e9929cd94c3a0f323312dd5988969c34d83793648c928457f7e` |
| Original-client GPU prep tar (manifest) | 3276800 | `448b2ea7e4f76658bed71d5ec1016bc084230894530f9b35073cabd24c67e18b` |

All seven `root-clean-build-evidence-manifest.json` bound files hash-match
disk. Frozen manifest `source_member_count=1124` from the member list, not
from an approval boolean.

Helper/patch bytes at the named commits equal disk:

- transcript helper `c14b1b5648b993d10b0979de5fc14f49b97d147b502edcee1f91169bd51b9b54`
- transcript patch `1e5ac15b25d113a180f8212fdd282bd63d76e79c14aa200560c2184106deb995`
- coverage helper `6e1aab8eae3bd217f1f2b320f8b1b45e7a8fae924563dd7c74449ed4af5a3082`
- coverage patch `dcb84047157f75c7ad21868b72eb7c3434cb818e8ba3e4b732f6b40d435c1b6e`

## Original 19-command matrix (preserved failure)

Coverage02 required 19 names (5 gap + 7 feature-off + 7 feature-on).
Coverage02 executed 9 and left 10 `not_run`; coverage03 executed those 10.
Independent log recomputation:

| Bundle | Commands | Pass | Fail | Failures |
|---|---:|---:|---:|---|
| coverage02 | 9 | 8 | 1 | `transcript_matches_golden_and_peer_feature_build` (feature-off host-play) |
| coverage03 | 10 | 7 | 3 | same transcript test (feature-on host-play); `gpu_textured_shade_scales_texel_brightness` (client integration, both features) |
| Combined | **19** | **15** | **4** | transcript both features; GPU both features |

Raw loc fields in the two host-play failures are planted `[0]`/`[1]` versus
golden `[4671]`/`[4672]`. Wrapper `all_requested_passed=false` on both
archives. Do not relabel these archives as passing.

Unaffected passing commands in this pair (TUI 92/0/0 both features, API/host/
script/client-unit, gap tests) remain separate generated evidence. They were
not re-executed after the transcript overlay; adding one host-play
integration test does not rewrite those logs.

## Native transcript affected suites (cc0dc06)

Linux x86_64 builder paths; helper `--run-focused` only, not
`--run-full-coverage-matrix`. Focused overlay-receipt SHA-256
`e79a5700026a98be8ac8158197782990a72f86c5ded72f4239366d3fa0bfafc4`
matches the wrapper. Platform `Linux` / `x86_64`.
`native_linux_qualified=false`, `live_qualified=false`,
`performance_qualified=false`, `requested_step_count=4`,
`all_requested_passed=true` for those four corrected focused commands only.

Independent loc-id readback from the panic dumps, not receipt booleans:

| Probe | Exit | Planted loc ids | Golden |
|---|---:|---|---|
| original empty cache | 101 | 0, 1 | 4671, 4672 |
| original 13-loc generated JAG | 101 | 13, 14 | 4671, 4672 |

Four corrected golden/peer commands exit 0, 1 passed each. Both complete
host-play profile suites then exit 0: feature-off aggregate **197** passed /
0 failed / **7** ignored across 13 harnesses; feature-on **212** / 0 / **7**.
Full `snapshot-dedup` integration **16** / 0 / 0. Existing ignored live tests
were not run.

All nine raw log SHA-256 values match the transcript audit and wrapper
`result.json`. Every listed step has `timed_out=false` and
`cleanup.group_absent=true`. Locks before and after equal the frozen trio
(`Cargo.lock` `03059d02…`, client `015530e3…`, shade-probe `d0285dae…`).

Seven derived members streamed from `root-transcript-native-01.tar.gz` match
the audit hashes, including overlay `crates/tui/src/bin.rs`
`aee97345…` and corrected
`crates/host-play/tests/snapshot_frame_equivalence.rs` `90abb21d…`.
Production frozen `crates/tui/src/bin.rs` remains `a859ca06…`. Clean-source
golden SHA-256 `783a7d5077829887147240a17545b69351ad2938c7304785489c899fb2dd81d4`
is unchanged.

The helper sets `native_linux_qualified` only when Linux x86_64 **and**
`--run-focused` **and** `--run-full-coverage-matrix` **and**
`all_requested_passed`. That 19-command path still contains both client
integration GPU failures, so the helper flag cannot become true while the
GPU defect remains open. Affected-suite proof is the correct instrument.

## GPU baseline (a02435c)

Original client `3456edc8dabf7b25ada78110ffa56327af9f67a4`. 183 original
source hashes are identical before and after. Streamed frozen GPU bytes
equal the original-client hashes in the GPU result and the preparation
manifest:

| Path | SHA-256 |
|---|---|
| `crates/client/tests/gpu_texture.rs` | `053eaef148d5a15e9bfe5da2d55393717707aef675e30a26aa40a22351513e9f` |
| `crates/client/src/render/backend/gpu.rs` | `46d5531d2b00e86a0c4fc67e2437372f0e45c211460a0f3b2faa09fe6424df64` |
| `crates/client/src/render/world.rs` | `9e2cc21a2c42c9f9956b7bd7e0a7790f0138a975b66413b41e8ca64a432e54e9` |

Independent log recomputation:

| Step | Exit | Harnesses | Passed | Failed | Ignored |
|---|---:|---:|---:|---:|---:|
| original `gpu_texture` exact shade | 101 | 1 | 0 | 1 | 0 |
| candidate `--tests --no-fail-fast` feature-off | 101 | 66 | 767 | 1 | 0 |
| candidate `--tests --no-fail-fast` feature-on | 101 | 66 | 771 | 1 | 0 |

Sole failure in every command: `gpu_textured_shade_scales_texel_brightness`
at `gpu_texture.rs:485` — `shade 16 must scale the red texel to ~223, got 255`.
`SKIP_GPU` is 0. `R274_TEST_FORCE_NO_GPU` is null. The only “adapter” lines
in the candidate logs are the passing test name
`gpu_backend_selected_when_adapter_available`. No adapter identity is
recorded. Do not import older container forensics.

These remain failed commands. No renderer or assertion change is in this
package.

## GPU / TUI disposition against admitted source

Clean materialization and frozen stream agree on host
`crates/host/src/lib.rs` `47c513a9…` and client
`render/renderer.rs` `c09c7192…`.

Admitted host facts:

- `slot_want_cpu` (665–667) is per-slot `prefer_cpu` **or** `BOT_CPU==1`.
- Draw-off slots construct no renderer and detach any existing head
  (`!client.draw` at 456–485 sets `slot.renderer = None`).
- A drawing slot builds with `Renderer::new_prefer(..., !want_cpu)` (533);
  `BOT_CPU=1` forces the CPU fidelity path if a head is created.

Admitted client fact: `prefer_gpu()` (renderer.rs:40–45) returns false when
`BOT_CPU==1`.

Plan §4 requires RasterOff. Plan §5.5 requires recording `draw=false` and
renderer absent. Design §1 excludes the CPU-fallback **measurement
profile**; that is not a ban on `BOT_CPU=1` or on draw-off. GPU client tests
in this package ran **with** `BOT_CPU=1` and still executed the shade
assertion, so `BOT_CPU=1` does not skip GPU tests. The TUI diagnostic path
is draw-off / no renderer, not that client GPU harness.

Therefore the original-client shade failure can remain an explicit open
defect while a bounded non-GPU TUI owner diagnostic proceeds after other
prerequisites. Source facts do not prove a future runtime followed
draw-off; runtime receipts must still show `draw=false` and renderer
absent. Final GPU panel, regression, cadence, and visual gates are **not**
waived.

## Production binary versus test overlays

Clean-build receipt (manifest-bound, hashes match):

- command: `cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p tui --bin tui-play --features memory-profile-no-alloc,memory-owner-capture`
- exit 0; ELF x86-64 pie, dynamically linked, not stripped
- binary 95595424 B, SHA-256 `392dbecc7a86f2fcd7e3f6b515aedceb3f3de3b1d4e140b95463bf20438c0c95`
- source 1124 members; frozen tar `2c36d254…`; host digest `b70608d1…`; client digest `169ba594…`
- production input: original H/C Git objects plus the three bound reviewed
  **production** patches
- `live_qualified=false`

Frozen production `tui/src/bin.rs` is `a859ca06…`. The cfg(test) TUI overlay
`aee97345…` exists only on derived test trees. Coverage/transcript overlays
must not enter a production or live binary build. This clean-build receipt
is source/build identity, not live admission (design §3 still requires a
runtime manifest plus conjunctive checkout checks).

## Findings

| Severity | Finding |
|---|---|
| Note | Helper `all_requested_passed=true` on the focused receipt is four corrected commands (`requested_step_count=4`). It is not 19-green. |
| Note | Coverage02/03 remain failed archives. The transcript rerun supersedes those two transcript failures as **later** affected-suite evidence; it does not rewrite the 02/03 records. |
| Note | The 19-command helper flag cannot become true while client-integration GPU failures remain in that matrix. Keep the GPU defect explicit instead of forcing a blanket flag. |
| Note | Candidate GPU logs mention “adapter” only as a passing test name. No backend/adapter identity was recorded. |
| Note | `BOT_CPU=1` on the GPU client commands did not skip `gpu_textured_shade_scales_texel_brightness`. Do not claim that env skips GPU tests. |

No reject-level defect in the frozen evidence or the proposed split.

## Remaining requirements (not this card)

Distinguish these from the approved generated affected-suite row:

1. **Source/build admission** — bind the clean derivative `392dbecc…` through
   design §3 runtime manifest / lineage checks. Do not pair the older
   `8e400e2d…` qualification binary with `dcdbeebf`/`b74dfb3`.
2. **Controller / native lifecycle proof** — reviewed direct-owner controller
   extension and its generated lifecycle tests. Uncommitted controller files
   were not evaluated here.
3. **Private fixture / server / headroom** — plan §5 preflight (account,
   cache snapshot, server identity, MemAvailable, no conflicting jobs).
4. **Single live capture** — one N1 real-PTY TUI diagnostic after the above;
   record draw=false and renderer absent. No retry.
5. **Performance / final branch** — no RSS/savings claim; whole-branch
   `branchreviewer` remains required.
6. **GPU** — panel measurements, regression, cadence, and visual gates stay
   required. This disposition does not authorize a renderer fix under owner
   instrumentation.

## What this does not approve

- Live release, Concord resources, or plan §5 procedure
- GPU qualification or any waiver of final GPU gates
- Relabeling helper `native_linux_qualified` or coverage02/03 as green
- Using overlay-derived trees for a production/live binary
- Controller implementation, private inputs, or campaign completion
- Performance comparison or residual-RSS attribution
