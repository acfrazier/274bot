# Platform fixture preparation

Updated 2026-09-10. Current fixture refresh is recorded below. Scope: isolated 289 engine setup and loopback asset readiness.
These receipts do not establish client login, host actions, rendering or script
qualification. Concord is TUI-only; Linux compilation belongs on the Hyper-V
builder. Windows native and macOS frontend checks remain in the campaign plan.

| Machine | Fixture directory | Node | Startup result |
|---|---|---|---|
| Windows `DESKTOP-SL99R6C` / `austen@10.0.0.205` | `C:\Users\Austen\274bot-campaign\multirevision-20260910\server-289` | `C:\ProgramData\274bot-Test\tools\node-v24.19.0-win-x64\node.exe` | World ready, HTTP 200, three loopback listeners |
| Hyper-V `274bot-builder` | `/home/builder/274bot-campaign/multirevision-20260910/server-289` | sibling `node-v24.20.0-linux-x64/bin/node` | World ready, HTTP 200, three loopback listeners |
| Concord | `/home/acfrazier/274bot-campaign/multirevision-20260910/server-289` | `/usr/bin/node`, v24.20.0 | World ready, HTTP 200, three loopback listeners |

The Windows firewall had retained the Mac's old IP; the operator fixed it.
The existing Hyper-V VM was off; root started it and rechecked SSH. Its current
Rust toolchain is 1.98.0, with 8 GB RAM and 18 GB free disk before further builds.
SSH keys, host trust and network configuration were unchanged.

Each fixture contains pinned engine `a275ea812f34342cbaf7faf2e3558dcd6ca187d0`
and packed content from `92649430fcbc83538d8c4367ecb96cee1a67a944`. Runtime maps
are included under `runtime-content/maps`; the sole configuration adjustment is
`build.srcDir = runtime-content`. Game/HTTP/management ports are
`44594/1080/9898`, all bound to `127.0.0.1` by the pinned engine. This is a runtime
bundle, not a full content-rebuild checkout.

The complete corrected archive is
`.superpowers/platform-preparation/server-289-fixture-v2.tar.gz`, SHA-256
`6524c4835aa825dc3994d7ae1a6536c1141836367eacaeccd5a3d61a5a373171`.
Current manifest SHA-256 is
`7cea872135aa1af6e26512b9270b7718bb363a4991ffeb8e97630b0544577e0b`.
All 1,213 runtime files were verified. The initial archive omitted
`data/raw/wordenc`; its first builder start failed with ENOENT. The corrected
file and manifest were applied after checking every original runtime file,
with the initial manifest retained. Client cache files are unchanged.

Locked dependencies were installed using `npm ci --ignore-scripts --no-audit
--no-fund`. Each machine generated fresh local RSA keys and a fresh SQLite
schema from the pinned migration. No existing key, account, save or database
was copied. The builder's portable Node archive was verified against the
official version-pinned SHA-256 list. No compiler work ran on Concord.

Start in the fixture directory with the selected Node executable:
`node --import tsx src/app.ts`. Keep the foreground/SSH parent alive for the
proof lifetime. Windows' initial detached SSH launch disappeared before
readiness; the verified launch holds its parent through `WaitForExit`.
No system service was installed for these new fixtures.

Concord's existing 274 system service required privileged stop access. The
operator stopped `274bot-concord-test-server.service` before the 289 check;
the fixtures run sequentially to fit its 2 GB RAM. The 274 files/unit were not
changed. No desktop, display server or large build was launched on the VPS.

Raw setup, failure, initialization and readiness receipts are under
`evidence/platform-preparation/`; `platforms.json` collects the three readiness
observations. Their PIDs describe those observations, not a persistent service
contract. Host/client test candidates and live action evidence remain separate.

## Build preflight after the usage-limit interruption

The committed host `93e12ccf` with client `2be16970` was exported as 481 verified
source files, excluding the active snapshot/reset edits. Archive SHA-256:
`4bd8088ca0e68b3ad2f5f41fff8c71a6248fa91c0fec62136456b48e0e40cdcd`.
Both remote source copies sit under the fixture parent's
`candidates/source-93e12ccf-2be16970` directory. These are build preflight
results, not the final source or native/live acceptance:

- Hyper-V: release `tui-play` build passed in 30 seconds with Rust 1.98.0,
  reusing `/home/builder/274bot-campaign/target` and two build jobs.
- Windows: release `panel-play` and `tui-play` builds passed in 77 seconds
  with Rust 1.98.0, reusing `C:\Users\Austen\274bot-campaign\target-native`
  and two build jobs. The build emitted an unused RSS helper warning and
  linker LNK4098 runtime-library warnings; the raw log retains them.
- Concord: no compilation and no UI launch. Its later TUI binary comes from
  the builder after the relevant candidate is reviewed.

Logs, source identities, commands and exit statuses are in
`evidence/platform-preparation/build-preflight/`. Final platform proofs must
rebuild the later reviewed candidate and record its executable identity.

## Reviewed local bank fixture refresh

Engine commit `cc359656b4acd216ca452495874b6beba9a0ac75` adds only the local
givebank fixture branch, with an explicit non-production and staff >= 4 guard.
It passed Luna implementation checks and actual Grok 4.5 review on the same
card `t_b237feb4`. The one source file has SHA-256
`30466707ab67003aa4b54be9e451982ac8b0267bb24839b6eb15971e7c08db33`.

Root restarted its isolated Mac 289 engine and applied the same reviewed file
to the three isolated platform fixtures. Each updater verified all 1,213
original runtime files before replacing that single source file and retained
the prior manifest. Current shared runtime manifest SHA-256:
`a8490093c5af0dc947d10aaa121842b94bf8a01048fc673916b232b200cf027f`.
The v2 archive above remains the original archive; later setups must apply the
recorded guarded patch. No keys, accounts, databases or client cache changed.

All four restarted fixtures passed `/rs2.cgi` and `/crc` HTTP 200 checks with
the same 40-byte CRC response. All three remote runtime manifests and 1,213
files were rechecked after startup. Observed PIDs: Mac 10828, Windows 17968,
Hyper-V 3644 and Concord 135117. Remote parents use SSH keepalives. These are
readiness observations, not banking or client-gameplay qualification.

The prior Hyper-V SSH parent had ended with a jump-host timeout, leaving no
engine or game listeners. The new process was started after the verified
update. The Windows stdin-based updater stalled before mutation; root stopped
only that identified updater and used SFTP successfully. A later oversized
read-only command also ran successfully after transferring its checker file.
Failures, ownership checks, model identities and successful refresh receipts
are retained in `evidence/platform-preparation/bank-fixture/`. The shared Mac
274 engine and operator-stopped Concord 274 system unit were not changed.
