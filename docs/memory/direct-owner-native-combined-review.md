# Combined Linux owner generated-qualification evidence

Independent review of the combined native evidence package. Not live
admission, not RSS/savings, not whole-branch Grok 4.6.

## Identity

| Field | Value |
|---|---|
| Reviewer profile | `grok46` |
| Actual model / provider | `grok-4.6` / `xai-oauth` (no task model/provider overrides, no delegation) |
| Task | `t_ffc1ef40` |
| Branch | `codex/memory-diagnostics` |
| Evidence commit | `7cc7545a186007bd03b323657a02730a293a36ff` (`docs: assemble native owner qualification corrections for review`) |
| Workspace HEAD | `6620e9e18e1f5eb2e0ee5d2d2a40e928eeccd5ab` (STATE-only successor; not in scope) |
| Report | `docs/memory/direct-owner-native-combined-report.md` at `7cc7545` |
| Manifest | `diagnostics/direct-owner-native-preparation/root-combined-native-evidence-manifest.json` |
| UTC | 2026-09-09T18:23:04Z |
| Plan | `docs/memory/direct-per-bot-owner-capture-plan.md` §§4–6 |

No source edits, SSH, native/live re-runs, private inputs, STATE, or remotes.
Manifest-bound files were hashed locally; expensive tests were not repeated.

## Verdict

**APPROVED** as combined native **generated** qualification of this production
diagnostic, with an **explicit test-only TUI exception**.

Root may write a **separate** combined-native-admission document from this
package. Keep `linux_generated_qualified=false` on the original failure
receipt. Do **not** treat helper `linux_qualified=false` as a remaining TUI
failure; that flag is hardcoded generic helper scope.

**Withhold live** until reviewed controller extension, source-Git derivative
admission, and private preflight (plan §5). This card is not that release.

## Manifest

Schema `root-direct-owner-native-evidence-v1`, `review_status=pending`,
`live_qualified=false`. **54** evidence paths; **54/54** exist; every
`bytes`+SHA-256 matches disk. No extras in the bound set were required for
membership.

| Bundle | SHA-256 | Members |
|---|---|---|
| Frozen source archive | `2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d` (5075876 B) | tar 1214 (1124 files + 90 dirs); manifest `source_member_count=1124` |
| Frozen source manifest | `ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e` (327668 B) | 1124 `source_members` |
| Original failure archive | `06059fae5610cf32f705648e27224d29aba776a7015204e95277e32e0f811977` (42525 B) | 24 |
| Client-correction + release archive | `70eda2037c2ae073797f0e392fb8797515f7e8e422c74db302f6d39e18e12346` (128730 B) | 18 |
| TUI overlay receipts archive | `a82eccc84c6a978b948c924994dda10ac95df7526bf86c94fcf7d78b3decdedf` (7766 B) | 3 |

Provenance in the frozen manifest: host original
`c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`, client original
`3456edc8dabf7b25ada78110ffa56327af9f67a4`, reviewed host `3cdc3e4…`,
reviewed client `5c73a4a…`, production input = original Git objects plus the
three bound reviewed patches. `snapshot_dedup=false`. Feature pair
`memory-profile-no-alloc,memory-owner-capture`. Source-archive / qualifier-tool
review remains prior `a7e112eba01529572259eee8015dc68d77159b07`;
`qualify_linux.py` SHA-256
`3a9857bf0a52a32d1b24bcdaf3509fc22f2e034a688ed5c967f4a5207c46d221`
(24446 B) matches the Linux `environment.json` and the client-correction
script assert. No production optimization is in this package.

## Original 15-step Linux matrix (preserved failure)

`qualification.json` still records `linux_generated_qualified=false`,
`all_steps_passed=false`, `live_qualified=false`, `locks_unchanged=true`.
Do not edit it.

Platform (environment.json): Linux 6.8.0-139 x86_64 glibc 2.39, rustc/cargo
1.98.0, target `x86_64-unknown-linux-gnu`,
`BOT_MEMORY_OWNER_CAPTURE=0`. Qualifier requires Linux x86_64; this is not
the Mac `source-check` fixture.

Executed steps are 15, not the 10-name coverage dict. Independent log
readback (command header + exit + cargo/unittest counts):

| Step | Exit | Raw result |
|---|---:|---|
| cargo-feature-tree | 0 | offline locked `--target x86_64-unknown-linux-gnu -e features -p tui` with the feature pair; log has `memory-owner-capture` and **zero** `snapshot-dedup` |
| cargo-build-tui | 0 | debug `tui-play`; wall 137.284 s |
| api-owner-lib | 0 | `ok. 9 passed; 0 failed; 0 ignored; 5 filtered` |
| api-owner-integration | 0 | `ok. 1 passed` |
| host-owner-lib | 0 | `ok. 6 passed; 214 filtered` |
| script-fingerprint-lib | 0 | `ok. 5 passed; 42 filtered` |
| script-stop-integration | 0 | `ok. 1 passed` (`stop_keeps_builder_unknown_and_drops_fingerprint`) |
| client-world-owner-lib | **101** | no tests; `error: no matching package named serde_core` under `--offline` |
| client-packet-integration | **101** | same offline `serde_core` miss |
| host-play-owner-lib | 0 | `ok. 7 passed; 182 filtered` |
| cargo-check-tui | 0 | |
| host-play-generated-observer | 0 | `ok. 2 passed`; see observer below |
| protocol-validator-generated | 0 | unittest `Ran 3 tests … OK` |
| tui-feature-tests | **-15** | `running 92 tests`; last line is incomplete `live_prepare_bone_burier…` with no `ok`; wall 308.714 s; `timed_out=false` |
| managed-guard-cleanup-generated | 0 | unittest `Ran 4 tests in 2.263s OK` |

That is **12 passing + 2 offline client 101 + 1 owned TUI SIGTERM (-15)**.
`root-tui-fixture-stop.json` records SIGTERM of the identified cargo test
group because a synthetic slot worker stalled. Hang-stack.txt is a failed
ptrace attach, not a usable backtrace; the stop receipt is the authority.

Debug `tui-play` from this run (`binary.json`): 506772032 B, SHA-256
`b14bce3e215c30d11b7f50ba39a13dd9d717d86c463c73c19a2f4e2d888d39d5`, ELF
x86-64 with debug_info. This is **not** the later release binary.

## Client correction (same production tree)

`root_correct_client_dependencies.py` SHA-256 `69be5d73…` matches the
manifest. It asserts the original three non-zero exits, relocates generated
`docs/memory/__pycache__/*.pyc` **out** of the source tree, re-verifies 1124
members, fetches with `cargo fetch --locked --target x86_64-unknown-linux-gnu`
(not a lock update), then re-runs the two original client commands.

| Step | Exit | Wall s | Raw result |
|---|---:|---:|---|
| fetch-exact-client-lock | 0 | 0.566 | downloads only; lock hashes rechecked after the step |
| client-world-owner-lib | 0 | 16.519 | `ok. 3 passed; 0 failed; 0 ignored; 75 filtered` |
| client-packet-integration | 0 | 3.702 | `ok. 1 passed` (`packet_probe_preserves_bytes_and_cursor`) |

Lock identities after correction equal the frozen manifest
(`Cargo.lock` `03059d02…`, client lock `015530e3…`, shade-probe lock
`d0285dae…`). `full_linux_qualified=false`,
`original_qualification_preserved=true`. No production source repair.

## TUI test-only overlay (not the production tree)

Overlay commit `25af151560be1287dc1ae4e81d7d1a35e8cd43fc` is unchanged
through `7cc7545`. Same-card Grok 4.5 / xai session `20260909_140618_7af3eb`
approved that helper (parent `t_08ab7a27`). Working-tree helper/patch hashes
equal the combined manifest and that commit:

- `stage_tui_test_overlay.py` 13235 B, SHA-256 `c21b38ecce6ae3d7afa93c83651a104f97f13d4f4bb6fe090dbdac2ee70661ea`
- `tui-test-only.patch` 3091 B, SHA-256 `3bf08949fee5f44a2c6dc82525004a9cc0f47759526d43bda6f77cc9f33a51e4`
  (cfg(test) guard only on `crates/tui/src/bin.rs`;
  `c0709aba..f9675b66`)

Linux overlay receipt (`b7a02f1e…`, 3437 B): `linux_qualified=false`,
`live_qualified=false`, `non_test_source_modified=false`, production
`bin.rs` `a859ca06…` vs derived `aee97345…`, 1124 members, all **three**
locks verified after-stage and after-tests. Helper always stores
`linux_qualified=false`; do not rewrite it.

Command (no `--target`; host debug dir):
`cargo test --offline --locked -p tui --features memory-profile-no-alloc,memory-owner-capture -- --test-threads=1`.
Log 21881 B, SHA-256
`801a28843c4d170bc9c812c105374078e507afd1f634282e3a7287996db02843`.
Path `/home/builder/274bot-campaign/direct-owner-a7e112e-1738/root-tui-test-overlay-01/target/debug/…`
is actual Linux builder execution, not the Mac fixture.

Raw counts: `test result: ok. 92 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.23s`
plus two empty `0 passed` harnesses (`src/main.rs`, doc-tests). Both
preparation fixtures executed: `live_prepare_bone_burier… ok` and
`live_prepare_thiever… ok`. Cargo `Finished … in 1m 11s`.

Distinguish Mac `tui-test-overlay/verification-receipt.json` (not in the
54-file set): `/private/tmp/…`, 4.71 s, log SHA `70b07a82…`,
`native_execution=false`. That fixture is not this Linux log.

Helper wall **74.175 s** appears only in the combined report / STATE, not in
`overlay-receipt.json` (the helper does not record `wall_s`). Independently
verifiable timing is cargo Finished 1m 11s + tests 2.23 s. Subsequent `/proc`
absence is likewise report-only. Neither is a qualification blocker.

## Production release binary (original tree, not overlay)

`root_build_release.py` SHA-256 `29666169…`. Command:
`cargo build --offline --locked --release --target x86_64-unknown-linux-gnu -p tui --bin tui-play --features memory-profile-no-alloc,memory-owner-capture`.
Exit 0, wall 134.926 s. `RUSTFLAGS` / `CARGO_ENCODED_RUSTFLAGS` /
`RUSTC_WRAPPER` null. `BOT_CPU=1`, `BOT_MEMORY_OWNER_CAPTURE=0`. Source
member count 1124 and all three lock hashes verified **before and after**.
Copied binary 95595384 B, SHA-256
`8e400e2d622af2c00280bfb91c741753e0da3df1736bb7d6fe7d0f8251394194`.
`file`/`readelf`/`ldd` exit 0: ELF 64-bit LSB pie, x86-64, dynamically
linked, not stripped, **without** `debug_info`. Distinct from the 506 MiB
debug artifact. `full_linux_qualified=false`, `live_qualified=false`. No
Concord install or live launch. Binary bytes are identity-bound, not stored
in the 54-file set.

## Generated observer (not live)

Linux `generated-observer.json` + host-play log line:

`GENERATED_OBSERVER {"allocations":0,"allocated_bytes":0,"thread_cpu_ns":672701,"wall_ns":671373,"scratch_reserved_bytes":262144,"native_qualified":false}`

Criteria all true vs 5 ms / 512 KiB scratch. Scope is generated empty-owner
pre-observe + nav/COW seam in an isolated test process. **Not** populated
scene cost. Mac prior source-check (`artifact/local-source-check/`, not in
the 54-file set) is a different platform: 1188334 ns thread CPU, same 0/0
alloc and `native_qualified=false`. Do not substitute it for this Linux row.

## Coverage vs “15”

`linux_generated_qualified` requires every executed step, observer criteria,
unchanged locks, coverage-dict subset, and a present debug binary — not the
coverage-dict length alone. Client unit/integration, api integration, script
fingerprint, and TUI are matrix steps even though they are absent from
`coverage_contract`. Combined counterparts now exist for all 15, with TUI
on the hash-bound cfg(test) tree only.

Focused owner-lib names that actually ran: visit cap
(`budget_stops_on_visit_cap`), mailbox three-request and full, busy-mailbox
non-retry, runtime/config off, COW scratch, cumulative 256 KiB output cap,
Stop fingerprint-clear + unknown builder. Frozen source has **no** test
named deadline or stale-frame/stale-slot. The qualifier’s coverage prose
overstates those two names; logs are the authority. That wording gap is
**not** a missing executed matrix step. Plan §6 feature-off crate suites and
deadline/stale-frame additions remain live-launch prerequisites, not this
generated package.

## What this does not approve

- Live N1 capture, Concord resources, account/cache/server, controller
  `e707e2d` extension, private preflight, or plan §5 procedure
- Relabeling the test-only overlay as production source or replacing the
  frozen archive / release binary
- RSS savings, residual attribution, whole-campaign completion, or
  automatic retry
- A `qualify_linux.py --mode linux` re-run that would still hang TUI on the
  **unpatched** production tree; admission is composite
- Whole-branch `branchreviewer` / Grok 4.6 campaign pass

## Combined native admission (root)

Root **may** produce a separate combined-native-admission record that binds
this review, `7cc7545`, the 54-file manifest, original 12/15 + client 3+1 +
TUI overlay 92/0/0, observer 0/0/~0.673 ms/262144 generated-not-live, and
release binary `8e400e2d…` on the original 1124-member tree. Keep original
failure qualification false. Keep helper `linux_qualified=false`. Keep live
false until the reviewed controller extension, source-Git derivative, and
private preflight exist.
