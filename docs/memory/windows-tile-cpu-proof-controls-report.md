# Windows native CPU fallback functional-proof controls report

Scope and status

This deliverable adds bounded native controls only. No native, SSH, VM, build,
frontend launch, polling, archive, or live command was run from this checkout.
Results from these cells must be labeled functional/visual proof only; they are
not CPU optimization, CPU performance-budget, RSS, latency, cadence, or GPU
acceptance evidence.

Frozen provenance

Baseline/reference uses host `9268890217d968cfeb7c66ebb11dd5c3dd2c084f`, client
`abb811bd0afa1acd99319ccd5bc36bfb241080f9`, binary
`e2deb1db371366f674c18e39d04f7309480310072ade224ffa1433c84d4559b5`, staged at
`C:\ProgramData\274bot-Test\renderer-owner-census-9268890`.
Candidate uses host `fb3589ac28583242b999ac864ea69c4ef8fa5923`, client
`fd956c91bf09e059359c8e182a33583e2c626cd3`, binary
`a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f`, staged at
`C:\ProgramData\274bot-Test\tile-boxed-fb3589a`.
The controller records the exact reviewed source aggregates, file counts, role,
fixture, server identity, native conditions, and binary digest per cell.

Server identity schema (e715 parity)

`server_identity_payload` writes the same reviewed GPU boxed identity shape:
`configuration.world_json_sha256`, `maps_addition_sha256`,
`wordenc_addition_sha256`, `bind_host`, `node_version`, plus top-level
`config_sha256`, `public_key_sha256`, `sample`, `launch`, `pid`,
`start_identity`, `port_listen`, and `server_commit`. Digests come only from
real server files via `sha`; values are never invented. The no-launch contract
loads the controller's `require_server_identity_complete` helper and rejects
PID-only or malformed configuration rather than treating process identity as
complete provenance. Local tests exercise the payload helper and real no-launch
callback with absent and config-mutated negatives.

Prior native baseline N1 (`baseline-focused-one-cpu-native-20260908-0155`)
finished workload-qualified under the incomplete PID-only identity and remains
partial functional evidence only. It is not rewritten or retroactively accepted
as full server-configuration provenance. Root will launch fresh baseline and
candidate IDs after same-card review of this control fix. Native runtime files
already staged for the prior cell are out of scope here.

Cell contract

Use fresh paired N=1 cells for each role and `focused-one` /
`focused-plus-background`. Each uses active focused-one gameplay, 30s warmup,
120s observation, and 60s teardown. The diagnostic argv explicitly contains
`--cpu-fallback --nav-captures --failure-capture --no-diagnostics`, render,
scheduling, and responsiveness flags, and deliberately contains no
`--gpu-completion-profile`. Owner census, debug, stack logging, and counting are
off. The configured CPU renderer cadence is not altered. Inherited `BOT_CPU` is
scrubbed; only explicit `--cpu-fallback` can set CPU intent. The real
`run_managed_cell` parser returns and validates an `argparse.Namespace`, and the
contract checks backend metadata (`cpu_fallback`), complete server configuration
shape/digests, and distinct no-launch versus launch output identities.

Required execution order

1. Austen/admin performs AST validation and fresh per-cell privileged preflight
   (server/native conditions, stages, and no overlap).
2. Austen/admin stages the reviewed controller into the selected ProgramData
   binary stage, then runs `stage-contract-tile-cpu.ps1` to copy the same
   controller into `run-<CellId>-contractcheck.py` and verify its SHA-256 against
   the controls source. Root must
   stage the already-reviewed CPU runtime files only after the clean GPU cells;
   this task does not stage them.
3. BotTest Interactive Limited runs the distinct `-contractcheck` controller,
   which binds `TILE_CPU_PREFLIGHT_CELL_ID` to the target cell, verifies the
   staged contract-runner hash, assembles and validates the real spec/parser namespace,
   complete native-conditions schema, and complete server-identity configuration
   without starting the client, and writes a separate contract receipt.
4. Austen/admin consumes and verifies that receipt, then root performs one
   supervised launch, polling, and archive of managed output plus every raw
   receipt-referenced run/capture directory. A failed cell is preserved, not
   retried with weakened controls.

Pending native gates and inspection

Windows PowerShell AST parsing, privileged preflight/staging, limited contract,
receipt consumption, the single supervised launch, process completion, workload
qualification, and raw archive integrity remain root-owned gates. After a
successful functional cell, a capable image inspector must inspect the scene,
bank/interaction, return-route, focus/overlay, and focused-background captures;
filenames or capture existence are not visual proof. CPU fallback remains a
separate functional/visual check and cannot inherit GPU completion conclusions.
This control-commit does not claim native proof.
