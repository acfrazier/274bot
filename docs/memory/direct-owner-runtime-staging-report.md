# Direct-owner runtime staging on Concord

Root staged the reviewed diagnostic inputs in a fresh standalone directory:
`/home/acfrazier/274bot-campaign/direct-owner-runtime-392dbecc-20260909`.
This did not launch tui-play, access account/cache fixtures, alter the server,
install packages, or change any existing runtime directory.

Before copying, Concord reported Linux x86_64,14145916 KiB free disk,
MemAvailable949636 KiB and no swap. No exact cargo/rustc/tui-play/heaptrack process
was found. These are staging observations, not the required fresh live preflight.

The transfer tar40257113B has SHA256
2734566a7cfda4230a6f25cf885bcc8f6115fdd86a4c8858ff8a27a7584b10ed.
Root verified it before extracting its10 regular top-level members. The staging
script verified all8 input hashes, cloned the local Git bundles and checked all
1124 source files/modes with no extras. Both checkouts are clean:
- host dcdbeebf36665a1d07156402a5f64c823769e00d,
  branch codex/direct-owner-host-archive;
- client b74dfb3c998b10371b263189774b055ecf880380;
- host source digest b70608d10f203657cbe731c6fd5402d09c96c1053466ce6540a7ab0dacc62564;
- client source digest169ba594a834fb5ea81f392ae77909251cc46889efa2324525fff71794a7ea30.

The binary remains95595424B, SHA256
392dbecc7a86f2fcd7e3f6b515aedceb3f3de3b1d4e140b95463bf20438c0c95.
ELF x86_64 and library checks both exit0; OpenSSL, crypto, GCC, math, libc and
loader resolve on Concord. No app execution or performance claim follows.

Local evidence under diagnostics/direct-owner-native-preparation:
root-runtime-inputs-01.json, root_stage_runtime.py, and
root-runtime-concord-01/{staging-launch.json,staging-result.json,staging.log,
remote-sha256.txt}. Downloaded receipts match remote hashes. Existing original
archive, source manifest, materialization and fresh-build receipt/log accompany
the runtime. The cfg(test) and transcript/coverage overlays are absent.

Runtime manifest is not issued and live_qualified remains false. The controller
extension and native lifecycle proof, exact runtime/private input bindings and
fresh preflight remain mandatory before the one authorized capture. The existing
server/account/cache and all measurement policy are unchanged.
