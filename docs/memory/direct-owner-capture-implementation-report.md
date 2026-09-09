# Direct owner capture — corrective implementation handoff

Task: t_ce3310e4. Branch: codex/memory-diagnostics. This report supersedes the acceptance claims in direct-owner-capture-t_ce3310e4-report.md, which remains unchanged as failed round-1 evidence. The scoped host correction is the commit containing this report; its exact commit ID is in the same-card review handoff. No client commit, gitlink update, remote operation, native capture, frontend or real-account launch was performed.

## Corrected implementation

- Actual slot registration supplies an opaque instance token; the existing synchronous observe closure borrows the existing interface template without another retained Arc. The pre-observe host/client rows and actual nav snapshot rows are separate scalar observations, with source ticks, generations, family gates, scene/base/tile and frame metadata. Equality is not inferred.
- PhasePublisher is used by the existing memory harness at observe+30s, observe+90s and teardown+30s. It publishes at most three requests; no retry or waiting barrier is added. Normal observe-end script_stop remains in place and records begin/end. Capture failure is retained and returned at normal completion, not used to extend the diagnostic or delay Stop for missing replies.
- Censuses run only for requested frames; the script census uses the existing slot lock. Initial keyframe, first delta and the first natural post in each active request window are armed separately. Buffer length/capacity are observed before the natural move, without forcing a tick or keeping the Vec.
- Capacity walkers include entity descendants, Packet spare capacity, both World/client heightmaps, linked-square chains without recursive double counting, action vector element storage, nested fingerprint families, and template/private/shared interface identity accounting. Diverged outer overlays do not hide the still-live template outer storage. The snapshot players row includes local-player descendants; it is not a separate snapshot local_player row.
- Capture uses fixed field arrays, bounded identity scratch and nonblocking bounded publication. Fingerprint chat-option scans charge the visit/deadline budget. Unknown builders and native owners remain null. Runtime OFF returns before census work; feature OFF keeps the original production path. The existing pinned serde version is enabled as an optional host-play capture dependency for scalar serialization; no dependency version or third-party implementation was patched.
- Output tracks requests, both fragments, all four encoded metadata records and Stop; fragments alone cannot certify success. It enforces the owner-specific cumulative 256 KiB cap, reserves a terminal failure receipt, and writes scalar fragments directly rather than allocating a JSON Value tree per field. Serialization runs after owner borrows in the harness poll path.
- The required docs/memory/validate_direct_owner_capture.py is the protocol validator. It checks complete record/family coverage, duplicate keys/rows, slot identity, request/frame/source/Stop order, deadlines, capacities, opaque-null semantics and exact expected JSONL SHA-256. It never certifies native admission or RSS reconciliation. The old scripts/validate_owner_capture_jsonl.py is retained as failed implementation evidence, not the admission validator.

## Executed verification

Raw commands, exits and per-binary test summaries are in diagnostics/direct-owner-capture-implementation/{regressions.json,final-retries.json,test-summary.json}. All 14 required feature-on/off suite selections have successful final runs:

- api and host: cargo test --offline -p PACKAGE, without and with memory-owner-capture.
- script: the same with load retained, and load,memory-owner-capture for the enabled selection.
- host-play and tui (the package is tui, not the binary name tui-play): memory-profile-no-alloc retained, plus memory-owner-capture in the enabled selection.
- Client, separately from vendor/fr-client-rust: cargo test --offline -p client --lib and --tests, each without and with memory-owner-capture. These are separate invocations; host tests are not substituted for client integration tests.
- The complete host-play suite includes snapshot_frame_equivalence and existing script/scene/bank/guardian/lifecycle regression tests. Ignored live tests were not enabled. No new live qualification is claimed from these suites.
- Additional final API run (api-on-final-green.log, exit 0) includes independent equal/different shell epochs and differing spare chat-line capacities, plus nested action option-header/string capacity checks.
- python3 -m unittest discover -s docs/memory -p test_validate_direct_owner_capture.py -v: 3 tests pass, including a complete explicitly generated protocol fixture and table-driven invalid mutations/file identity cases. These scalar fixtures are parser tests, not fabricated measurement evidence.
- Source/documentation git diff --check: passed. The staged raw evidence intentionally preserves Cargo trailing blank lines and patch context-space lines; an unrestricted staged whitespace check flags those artifacts. Private/new Rust helper/test files were rustfmt-checked/formatted; unrelated large production files were not reformatted.

Failure history is retained, not reclassified as passing: premature output completion (output-red.log), omitted chat-option budget (fingerprint-budget-red.log), missing diverged template outer row (cow-template-red.log), and the initial client WorldOwnerRows Debug test compilation failure. The first matrix also used the binary name tui-play rather than package tui. During the final output correction, library compilation exposed serde previously being dev-only; host-play/tui enabled selections initially exited 101. Capture-only serde feature wiring fixed that, and final_retries.py reran both complete suites with exit 0. The extra API epoch test initially used the optional chat field as a vector; api-on-final.log preserves that compile error and api-on-final-green.log the corrected successful suite. Existing cadence dead-code warnings remain.

## Generated observer measurement (macOS, not Linux qualification)

Command: cargo test --offline -p host-play --features memory-owner-capture --test direct_owner_capture -- --test-threads=1 --nocapture.

The final generated-overhead-green.log records 2 passing tests and this measured isolated test-process observation:

- allocations: 0; requested allocation bytes: 0;
- thread CPU: 936167 ns; wall span: 1092291 ns;
- reserved COW scratch: 262144 bytes;
- native_qualified: false.

A positive allocating Vec check precedes the measured scope. The counter is installed only in this test executable, not the production capture. Measurement covers pre-observe client/World/host snapshot and the actual nav/COW hook with generated empty owners; it does not represent populated native scene overhead, script/native CPU, persistence cost, or a whole-process allocation total. Scalar rows, visits and individual spans are emitted by capture; touched scratch bytes are not measured. Do not subtract these timings from previous clean RSS/CPU evidence. Enabled capture remains perturbed diagnostics, including its placement within the existing observe timer.

## Exact overlays and file binding

Original host: c0709aba2f8b45e42193225cf8f4e7325b5ca9bf.
Original and still-current client HEAD: 3456edc8dabf7b25ada78110ffa56327af9f67a4.
Failed initial host instrumentation: 70a09540bb554cf28d07ecc2c9afd0f7b0157542.

The complete source manifest is diagnostics/direct-owner-capture-implementation/overlay-manifest.json. It enumerates every touched host/client source member with exact bytes and SHA-256, including added client helper/test files. Compact patch artifacts live beside it:

- host-initial.patch: initial scoped source instrumentation only;
- host-correction.patch: this correction and new host tests/helper, excluding unrelated campaign changes;
- client-overlay.patch: tracked changes AND both new files, relative to original client HEAD.

Final patch SHA-256 bindings:

- host-initial.patch: 5229bab8d7c59046dbb8e134daca2bd9e570043c3016e3277e2db6369fdbb8a9
- host-correction.patch: d2646cc175a376a295de2bd9261c4c8d8c7a2af57bbeb52171732b73480f4917
- client-overlay.patch: 0a5d43ed382c43d293e17824b1565dc4a5258d5b17971edaea55ad6a58324430

Client member SHA-256 bindings (paths relative to the submodule):

- crates/client/Cargo.toml: 4197688f08ed259ad96f2dc43dda6aa8433960769d30b3e4be51c8f1fd3ce26e
- crates/client/src/core/world.rs: cd6cb7d14acc5f0406d217ae95691b6c1bffcb5089b53f7442bbe17ab8891d9e
- crates/client/src/io/packet.rs: 7485b52922b60ec17f00e01e3dbd90280dfebdd3dd0021bf1b553ecfa361e718
- crates/client/src/core/world_owner_capture.rs (new): caafa9426d7cbbc2a870e41cb348f1e9189ed5cb8905fd5b6477f4d94f3da8e8
- crates/client/tests/direct_owner_capture.rs (new): 0998c62ad4c44a1ff48eec084aa8e0fe3d58d28f8b38af75885f8fec6dc37d62

bind_overlay.py used independent temporary Git indexes loaded from original H/C. Sequential git apply --cached --check and application of both host patches passed; client overlay check/application also passed. These operations neither changed a frozen checkout nor staged the real index. This proves offline overlay applicability, not a frozen-native build. Tests here compile the current checkout; they are not relabeled as H/C binary evidence. Root must materialize original objects plus these reviewed overlays and admit the resulting exact tree/locks/features/tools/binary before any native build or run.

## Explicit unknowns and release boundary

FlatBufferBuilder capacity remains opaque before and after Stop. Queued/inflight buffers after move, isolate interact/paint builders, compiled trait-object/V8/native storage, Packet pools/stream queues, audio, ancillary ground-object collections and unwalked immutable interface/model graphs remain explicit unknown coverage. Encountered unsupported model descendants invalidate a claimed complete subtotal; no mesh/model allocation is assigned an invented size. Shared requested storage is not private RSS, and inline owner headers must not be counted again as independent heap blocks.

Root prerequisites remain: independently reviewed host commit plus exact dirty client overlay; root-owned client commit/gitlink reconciliation; original-H/C native build admission; generated Linux observer allocation/CPU and guard/cleanup qualification; exact cache/account/server/resource admissions and existing launcher/supervisor hard limits; workload/readiness/normal Stop proofs; and the separately released one-attempt live procedure. The local protocol validator alone cannot establish those external identities or resource guards, and deliberately reports native_qualified=false. No success ledger, residual-RSS attribution, production optimization, or campaign-completion claim is made here.
