# Independent client integration review — Astra

Verdict: CHANGES REQUIRED (one high-priority lifecycle regression).

Task t_acc01541, run 1082. Actual reviewer: orch / gpt-6-astra / openai-codex; session 20260910_100829_8e54b7. Review began 2026-09-10T14:08:29Z; investigation ended 2026-09-10T14:17:37Z (548 seconds; report submission follows). No other trial reviewer's report or findings were read. The prerequisite Grok 4.5 handoff was available as permitted input.

## Frozen identity

- Client published base: 9b41e6e06b9fd42dc2247fc813a303c3fcb92941.
- Client reviewed head: 8b1a80918f87089e4eb339f1d0e216c1b163348d; product cf386ae5c82071ef661f4a1eb4d38e22b41718d3.
- Source 289 line: c18f3a1148e9caee73426e162677328ca64d1a83; shared ancestor 4f2048ea10f75b3bb92ff45610b35ba7313b0308.
- Host comparison limited to the fixture helper/test: b2bd5023489ab2e0b6ba690f228217e8984ac91b → 391726831b3dc97d3614bb8ad8ddb48f75c1e85d.
- Manifest SHA-256 independently matched a975118db49f01361b3a085ec4708ff6073ea14bcc3b134b60ac769084b58724. Every evidence entry passed `shasum -a 256 -c`. Host and client HEADs were rechecked unchanged before reporting.

## P1 — Clear the previous game-frame bound before reading a login seed

Candidate locations: `vendor/fr-client-rust/crates/client/src/client/client.rs:2557-2562`, with the new bound established at `:4254`; `vendor/fr-client-rust/crates/client/src/io/packet.rs:101-105,212`.

`tcp_in` now leaves `self.in.frame_end = Some(psize)` after receiving a complete game packet. The next opcode read clears it, but a login/reconnect does not. `login` writes the new eight-byte seed directly through `data_mut`, resets only `pos`, then calls bounded `g8()`. If the last completed game packet was shorter than eight bytes, this panics at Packet::assert_can_read even though the server supplied all eight seed bytes. A fresh TCP connection does not imply a fresh Packet allocation. Adoption explicitly transfers this same inbound Packet at client.rs:2467, so the adopted-session reconnect also inherits the stale limit. This affects the shared 274 and 289 lifecycle, not only the new protocol profile.

The published base did not have frame_end or the g8 bound assertion; this is a newly introduced regression, not a historical reconnect limitation. Primary 289 Java login reads eight seed bytes and resets its cursor before method482 at client.java:8364-8366; a previous game payload length does not constrain those seed bytes.

Focused reproduction without editing source: ran existing `target/client/debug/deps/from_shared-961a032c3c9cb5e8 --exact adopt_reconnects_opcode_18_without_tcp_drop` normally: 1 passed. Then used LLDB on the same binary, stopped at client.rs:2557 on its second (reconnect=true) login, and seeded inbound frame_end with Some(1), representing a preceding one-byte game frame. Continuing panicked at packet.rs:102; test result 0 passed / 1 failed and inferior exit 101. The fake server also failed when the client abandoned the socket. LLDB itself returned zero, which is not a passing test. This is a state-injection reproduction of the stale-bound failure, not a newly added end-to-end short-frame fixture. The static production path above establishes how the state survives into login.

Required correction: reset/replace the inbound frame bound at the login framing boundary before any seed read, without weakening game-frame bounds. Add a meaningful lifecycle regression that receives a short actual game packet before reconnect/adoption and checks the normal handshake succeeds. Cover reused-client cold login as well as reconnect as appropriate; existing tests starting with an unbounded Packet miss this case. Do not implement the fix in this review.

## Other inspected areas

Reviewed the complete published-base-to-candidate diff, not only the final reconciliation patch, with source reads of revision construction/cache parsing, actor/zone/misc dispatch, outbound payload construction, framing/publication, adoption/login/reset, shared rendering and presentation, and the scoped host helper. The primary 289 Java decoder/login anchors and protocol audit were used as source evidence rather than treating the older standalone approval as candidate approval.

No additional blocking defect established in these checks:

- Client/host ownership remains separated: no host crate or foreign runtime introduced in client. Host profile binding and direct host writer adaptation are still future steps, not implicitly qualified by the client constructor work.
- Default/explicit 274 and explicit 289 constructors, shared cache/interface Arc identity, and revision transfer are represented in source and regression receipts. Appearance byte buffers remain boxed at the new actor write sites; no new deep World copy identified.
- Animation-base Arc sharing/borrowed delay access and dynamic sprite recycling remain preserved. Compared relevant unchanged files against the published base. GPU resource accounting continues to include retained textures/buffers, rather than losing accounting during reconciliation.
- Windows socket implementation and HOME/USERPROFILE selection remain present; CLIENT_UNPACK_DIR composes with explicit operator-home handling. This is source preservation, not a fresh Windows run.
- Revision-specific opcode tables are accompanied by ordered payload writers and tests, not merely length assertions. Frame payload checks and generation publication remain distinct from arbitrary TCP segmentation. No invented tick-end opcode or relaxed timeout identified. The login-bound transition is the exception recorded above.
- GPU finish uses exact last-upload RGBA/coverage comparison. Overlay-only updates preserve the held minimap via `live || (overlay_changed && !pending && held)`; a full chrome redraw retains its separate semantics. RGB in a sealed scene window is compared when coverage is not the deciding signal. Unchanged uploads remain lazy, while blink, movement, removal and late overlay writers can invalidate visible pixels. No private epoch mechanism is required.
- Inspected production hint/frozen-minimap coupled test and red/green receipts, not just separate freeze/hint checks. The test poisons CPU minimap chrome and checks the distinctive held minimap with the updated hint and lazy follow-up. Last-FBO freeze and overlay refresh are compatible in this path. CPU tutorial/chat tests are offline presentation checks, not tutorial parity. Ground diagnostics remain opt-in; composed ground tests assert actual rendered terrain where required.
- Host `mint_live_names` retains the changing low base-36 digits of an atomically incremented randomly seeded counter, fixing the previous deterministic truncation collision. Extended tests cover repeated invocations and representative fleet sizes with the 12-character budget. Frontend/harness callers still consume the same Vec<String>; no session-policy change. This is bounded fixture isolation, not mathematically permanent/global uniqueness: finite token space and independent process seeds can collide. No new blocker for the intended harness sizes established.

## Evidence verification

Recomputed totals from raw `test result` lines with jq (not copied from summary prose):

| Frozen raw log | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| docs/compat/evidence/client-baseline-274/cargo-test.log | 774 | 0 | 0 |
| vendor/fr-client-rust/docs/revision-289/evidence/bothost-integration/14-cargo-test-workspace.log | 990 | 0 | 2 |
| vendor/fr-client-rust/docs/revision-289/evidence/bothost-integration/root-explicit-gpu.log | 2 | 0 | 0 |
| docs/compat/evidence/client-milestone/host-crates-after-name-fix.log | 441 | 0 | 7 |

The two ignored client GPU checks were explicitly exercised in the dedicated GPU receipt, including `npc_hint_production_gpu_blink_move_clear_cache_and_freeze`, rather than counted as execution from the workspace ignored entries. Inspected the explicit GPU receipt, combined-freeze failure/pass logs, shared-constructor and CPU tutorial receipts, final clippy log, host fixture red/green evidence and scoped diff. Seven ignored host live tests remain unexecuted live gates. `git diff --check` for the candidate client crates passed. No routine suite rerun was performed; the focused debugger experiment and its unmodified control are the only fresh test executions in this review.

## Restrictions and remaining gates

Only this assigned report was written; no product edits, commits, merges, pushes, branch changes, or other trial report reads. Local Python `-c` and execute_code were blocked by headless approval policy; switched to ordinary shell tools/jq. LLDB lacks a Rust language plugin: an initial field lookup used dot instead of pointer access and failed, then pointer inspection and explicit in-memory Option state injection worked. These retries, context recovery, and tool/security restrictions are elapsed-time confounders, not model quality measurements. No security configuration was changed.

Historical GPU shade/CRC timing failures are historical evidence, not freshly reproduced failures. Current Linux/Windows execution is unavailable here. This review does not approve host 289 sessions, revision 377 qualification, complete tutorial/audio parity, mixed fleets, public-live acceptance, or the final whole-campaign release. Those remain explicit later gates. Root should reconcile this independent finding and request the bounded lifecycle correction and re-review before approving the client integration milestone.
