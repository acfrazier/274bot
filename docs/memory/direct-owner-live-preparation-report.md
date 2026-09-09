# Direct-owner live invocation preparation report

Status: OFFLINE PREPARATION ONLY. No native qualification, account admission, cache/server access, frontend launch, PTY, or live attempt was performed. No qualified-live or savings claim is made.

## Delivered

`diagnostics/direct-owner-live-preparation/prepare_invocation.py` is a fail-closed source/provenance contract generator. It requires explicit root-supplied paths and verifies, without reading private account material:

- frozen source archive SHA-256 `2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d`;
- original host `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf` and client `3456edc8dabf7b25ada78110ffa56327af9f67a4` identities;
- native generated qualification schema and `linux_generated_qualified: true`, while requiring `live_qualified: false` and `frontend_launched: false`;
- schema-faithful direct-owner build manifest/binary identity (`candidate.commit`, nested `candidate.client.commit`, `binaries.candidate_tui_play`), with the locked `memory-profile-no-alloc` + `memory-owner-capture` feature pair, `allocation_counting: false`, and `snapshot_dedup: false`;
- existing N1 controller review identity `e707e2d`, exact reviewed controller digest `8634665855d87aa93d27f10bc386f87d04be4c522b24e5379174f45cc2f37312`, and nav pack SHA-256 `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`.

The generated contract hard-codes the requested single attempt: N=1, real 120x40 PTY, 30 s warmup, 120 s observation, 60 s after-script-Stop teardown, A at observe+30, B at observe+90, C at teardown+30, 0.5 s polling, MemAvailable floor 256 MiB, frontend RSS ceiling 512 MiB, output ceiling 64 MiB, and 360 s wall ceiling. It records required root admissions and explicitly preserves the protocol validator boundary: `validate_direct_owner_capture.py` cannot establish native qualification, RSS reconciliation, or account/cache/server admission.

## Existing harness audit and exact gap

The reviewed managed launcher already provides useful bounded primitives: explicit JSON cell specs, 0.5 s process sampling, one attempt, immutable binary/manifest/cache/raw-file hashes, qualification boundary capture, direct-child cleanup, identity-checked frontend cleanup, foreign-server non-signalling, and durable failure receipts. The reviewed N1 controller reuses the e707e2d account-selection/seed path and rejects missing source/server/build identities before launch.

It does not meet this procedure as-is:

1. `run_current_tui_calibration.py` fixes warmup/observe at 120/600 s and its memory guard at 128 MiB; it has no 30/120/60 override or 256 MiB runtime guard.
2. `run_managed_cell.py` has no frontend RSS ceiling, no output-tree 64 MiB ceiling, no 768 MiB/free-256 MiB/no-swap/conflict preflight, and no server-health admission beyond identity sampling. Its sampler measures processes but does not enforce these requested limits.
3. The controller's current `max_wall_s` is derived as 960 s, not 360 s, and the current launcher command cannot be changed by a spec without also satisfying its controller/diagnostic timing consistency checks. Its feature validator also accepts only `memory-profile-no-alloc`; the owner-capture pair needs a narrow reviewed extension, not a manifest-only bypass.
4. Existing cleanup receipts cover owned launcher/collector/frontend paths, but the complete direct-owner procedure still needs a root-reviewed receipt that binds workload/readiness, normal Stop, raw owner JSONL, resource-series guard events, exact owned-tree cleanup, and no foreign signals in one result.

Narrow fix required before any release: add a reviewed controller/managed-launcher contract extension for these exact timing and guard values, with fail-closed preflight and cooperative 0.5 s runtime checks for MemAvailable, frontend RSS, output bytes, and wall time. Keep the existing one-attempt ownership/cleanup code; do not weaken it, add retries, change the server, or rewrite the N1 controller selection extension. The extension must preserve all raw metadata, resource rows, owner rows, workload/readiness, Stop, and cleanup receipts on every failure.

## Root steps still required

1. Complete and independently review Linux generated qualification on the exact frozen source archive; this task cannot mark it qualified.
2. Materialize the original H/C objects plus only the reviewed owner overlay, exact lockfiles/features/tool identities, and build the direct-owner binary. Recheck source, binary, controller, nav, catalog, cache, and server identities.
3. Supply private root-only disposable account/owner fixture admission. Do not use the operator highmem vault and do not write account names/passwords to the owner ledger or this report.
4. Supply exact cache snapshot/version/file hashes, canonical unpack root, server PID/start identity/health and public artifact hashes, and host preflight receipts: MemAvailable >=768 MiB, free output >=256 MiB, no swap, no conflicting owned work.
5. Apply the narrow launcher guard/timing extension and run exactly one real 120x40 PTY attempt using e707e2d. Require ingame and `scene_state==2`, seeded/XP-proved active Thiever workload, A/B/C frame matching, natural initial-keyframe/first-delta metadata, normal Stop, final Idle/fingerprint absence, zero required inflight/isolate gauges, and ready=1.
6. On any failure, stop/reap only the owned tree, preserve all artifacts and receipts, verify no owned descendants remain, and do not retry or signal the server/unrelated helpers. Validate owner JSONL separately with the existing validator; do not turn its `native_qualified: false` or `rss_reconciliation: false` fields into success booleans.
