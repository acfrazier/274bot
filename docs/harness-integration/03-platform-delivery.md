# Three-OS harness diagnostics and delivery

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: bounded architecture for brief 46 section C.
Not implementation, not LIVE, not native startup acceptance, not a
release tag, not a remote-control product.

Read once: `AGENTS.md`, `docs/execution.md`, `docs/harness.md`,
`docs/compat/STATE.md`, `docs/compat/01-session-profile.md`,
`docs/compat/06b-panel-startup-trace.md`,
`docs/compat/06c-panel-startup-design.md`,
`docs/compat/06d-panel-startup-preparation.md`,
`docs/compat/platform-preparation.md`, and
`docs/compat/briefs/46-harness-architecture.md`. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except
this report. No product edits, LIVE, fixtures, STATE, sibling A/B/D/E
files, subagents, stash, reset, restore, checkout, merge, remotes, or
release actions. Concurrent uncommitted catalog/game-data WIP is not
this report's product.

Operator request: integrate useful external harness capabilities into
the application for macOS, Windows and Linux release binaries. Current
shipped code is authoritative. Memory-campaign reports are evidence of
earlier capabilities, not a campaign restart.

## Verdict

**Proceed with a small in-app diagnostics/identity surface, keep
SSH/build/server provisioning outside the binary, and stop before
installers or remote ops.** Design approval is not source acceptance
and not three-OS proof.

Do not add a second capture backend, a second scenario engine, or a
distributed campaign controller. Reuse `ProfileSelection` /
`ServerProfile` / `SharedClientTemplate` / `ValidatedTemplate`, the
committed panel prepare workers, existing `--smoke` / `--live`, and
the shared profile CLI. Keep hashing identity checks. Keep server-admin
preparation explicit and isolated.

The useful application increment is: every shipped binary can say what
it is, check the selected profile/resources without logging in, stay
responsive while it prepares, write a redacted support bundle, and
emit a smoke receipt whose PASS is scene-2 (or TUI scenario proof),
not process exit 0. Packaging, SSH, fixture tarballs, Node engines,
NASM, and remote cargo stay operator automation.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `6c6bb2d5c25d51f5b339d6bb5f73d68b3b7296cb` (bank-fletching catalog; not this source) |
| Startup preparation (committed product) | `814e5293fec426b722e450580c514b9cffca826f` |
| Startup design | `06c-panel-startup-design.md` SHA-256 `6560e21e24e35a0ce60f00efda7f75e7abbb5c222dbe418b0afaf0f6df470455` |
| Startup implementation report | `06d-panel-startup-preparation.md` SHA-256 `00979340a76b5582af38981bb38a1fd5086526d5f7b752ccf74643ecb92c4cc4` |
| Client submodule HEAD | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief SHA-256 | `4a5a7df764e4c25fc6f1eeafeaa1ce316df2422a5f69397a9714ab60ce7192a9` |
| Windows native diagnostic (pre-fix) | host `584d05bcfe9e70c67ca897dd451bfd8a75d611b7`, binary SHA-256 `744ae204e58fabc7f652a25d4d1b6e0522876a83634ed7b4e988377212d80ddc` |
| Historical ref (read-only `git show`) | `codex/memory-diagnostics` `docs/memory/architecture-synthesis.md`, `build_provenance.py`, `concord-linux-build-report.md` |
| Kanban card | `t_b24bca46` |

WIP not used as product: `crates/host-play/tests/catalog_boundary_live.rs`
(modified), untracked catalog-headed/platform evidence, sibling
`docs/compat/evidence/harness-architecture/`. Helper source under
`.superpowers/platform-preparation/` was read; frozen source exports
and raw log bodies were not.

## What current source already does

### Profile and resource identity

`ProfileOptions` / `ProfileEnvironment` resolve a named profile before
any vault mutation (`crates/host-play/src/profile.rs`). Bind freezes
eight JAG hashes, CRC reread, RSA public values, nav pack/flags, and
optional cache manifest. `ServerProfile::validate_resources` re-hashes
those bytes against the frozen identity. `hash_file` is still
whole-file `read` + SHA-256 (`crates/nav/src/manifest.rs`). Keep all
three passes; do not replace them with path/mtime.

`ProfileSelection::prepare_template` is bind then
`SharedClientTemplate::load`. `SharedClientTemplate::validate_for_play`
is the only constructor of one-use `ValidatedTemplate`.
`run_with_template` still hashes for TUI/CLI.
`run_prepared_template` consumes the ticket and must not become the
default checked entry.

### Responsive preparation

Panel, after first present, detaches prepare then last-validate workers
and keeps painting (`crates/panel/src/app.rs` `StartupPreparation`).
Unlock is queued. Generation drops stale results. CloseRequested drops
receivers and does not join hashers. TUI still binds/loads before its
event loop; 06c accepted that as a terminal pause, not this section's
expansion.

Native Mac/Windows responsiveness after `814e5293` remains root-owned.
This report does not claim the beachball is gone.

### Build identity

Panel only: `crates/panel/src/build_info.rs` plus `crates/panel/build.rs`.
Visible line is hand-bumped `alpha 1 · <short>` (crate version still
`0.1.0`). Hover has `CARGO_PKG_VERSION`, full `GIT_COMMIT`, dirty,
`BUILD_TIME`. TUI and `host-play` have no equivalent. `--version` does
not exist on any binary.

`build.rs` stamps git via `git rev-parse` / `status --porcelain`,
overridable with `GIT_COMMIT` / `GITHUB_SHA` / `GIT_DIRTY`. `BUILD_TIME`
runs `date -u +%Y-%m-%dT%H:%M:%SZ`, which is not a Windows build tool.
Client submodule SHA is not baked.

### Runtime paths

Shared flags: `--cache`, `--unpack`, `--nav-pack`, `--nav-flags`,
`--vault`, `--engine`, `--catalog`, `--cache-manifest`. Environment:
`ENGINE_DIR`, `CLIENT_UNPACK_DIR`, `NAV_PACK`, `NAV_FLAGS`, `HOME` /
Windows `USERPROFILE` via `client::operator_home` and
`script::bot_home`. Explicitly blank `HOME` stays explicit
(`docs/harness.md`).

Defaults are developer-shaped, not release-shaped:

| Profile | Default cache | Default unpack / nav / vault under `~/.274bot` |
|---|---|---|
| local-274 | `~/experiments/Server/engine/data/pack/client` | `unpack`, `274bot.navpack`, `vault` |
| local-289 | `~/experiments/lostcity-289/engine/data/pack/client` | `unpack-289`, `289/274bot.navpack`, `vault-289` |
| public-289 | `~/.274bot/unpack-289` | `unpack-289`, `289/274bot.navpack`, `vault-prod` |

`host_play::default_vault_path_for(Local)` still returns `vault`, not
`vault-289`. Bound `ServerProfile::vault_path` is the revisioned path.
Callers that use the target-only helper without a bound profile can
open the wrong file on 289. Recorded as a seam; not fixed here.

Relative paths anchor to the captured working directory. Windows
headless catalog comparison at `584d05bc` canonicalized paths in the
harness; the application still prints raw `Path` display.

### Smoke and live

`panel-play --smoke` waits for focused `ingame && scene_state == 2`,
settles 3 s, writes one whole-window shot, PASS/exit 0 or FAIL/exit 1
(`SMOKE_DEADLINE` 300 s). `--live script_<name>` uses ScenarioRunner.
TUI has `--live` without `--smoke` (raster off; PASS from the
scenario, not a PNG). Readiness/normal exit is not success
(`docs/harness.md`).

`crates/panel/examples/catalog_watch.rs` is a campaign wrapper: requires
`--live script_*`, enters `IsolatedEnv` without rewriting `HOME`.
Platform helpers rename that example to `catalog-watch.exe`. It is not
a user-facing product name.

### Game-data and packaging metadata

`tools/game-data/generate.ts` / `verify.ts` pin engine/content commits,
refuse dirty inputs, and emit serde JSON under `crates/api/data/game-data/`.
That is build-time tooling (section E owns schema). Release binaries
consume the committed JSON; they must not shell out to Node or git
against the operator's engine checkouts.

Workspace `Cargo.toml` has no release profile beyond client
`opt-level=3` in dev. No zip/msi/dmg/appimage. No code signing. README
still describes a 274-only public `43594` world; current code's public
profile is 289 at `w1.rs2b2t.com:443`. README is not this report's edit.

## External helpers: keep vs fold

Active `.superpowers/platform-preparation/` helpers (source only):

| Helper | What it is | Destination |
|---|---|---|
| `extract-fixture.py`, `initialize-fixture.mjs`, `apply-bank-fixture.py`, `check-bank-fixture.py` | Isolated 289 engine unpack, fresh RSA+SQLite, staff givebank patch | **Outside.** Server-admin. Never a user session. |
| `windows-build-preflight.py`, `build-catalog-source.py`, `fork-platform-source.py` | Export/verify source tarball, `cargo --locked --release` on a named cache | **Outside.** Operator/CI. Binary can *report* identity, not build itself. |
| `run-platform-catalog.py` | Launch `catalog_boundary_live` with `LIVE=1` and hashed binary | **Outside.** Campaign proof. App already has `--live` / host-play tests. |
| `windows-native-prepare.py`, `windows-native-watch.py` | Copy hashed inputs, launch renamed example, PID receipt | Fold the *receipt shape* (commit, binary SHA, command, public-key SHA). Drop the rename and the SSH copy. |
| `windows-capture-owned.ps1` | Win32 `CopyFromScreen` of another PID | **Outside.** Failed on BotTest: PowerShell execution policy. App already has GPU whole-window capture; do not add a second backend. |
| `windows-public-login.mjs`, `windows-nasm-build.py` | Public RSA extract; assembler bootstrap | **Outside.** |
| Memory `build_provenance.py` / Concord ELF receipts | Frozen binary SHA, rustc, glibc floor, feature graph | Fold *identity fields* into baked `--version` + a sidecar receipt written by the packager. Do not ship heaptrack, owner-capture, shade probes, or ConPTY helpers. |

`initialize-fixture.mjs` generates a local RSA pair and refuses to
overwrite keys. That boundary is correct: the app may consume
`LOGIN_RSAN` / `LOGIN_RSAE` (public) or read the engine public half. It
must never copy `private.pem`, mint server keys, or inherit givebank.

## Startup stalls (Mac and Windows)

Mac 06b: debug `catalog-watch-f2b04198`, local 289. Window existed;
UI thread hashed ~334 MB pack+flags three times inside `boot_execute`.
First slot log at +37.471 s. Diagnosis, not a release budget.

Windows `584d05bc` native BotTest: `catalog-watch.exe --live script_alcher`
with explicit `--cache/--nav-pack/--unpack/--vault`. Receipt elapsed
198.079 s, process exit 0, no `FAIL:` lines in the scanned log. First
log line is already `slot … thread up` — hashing is before that.
Timed window captures at 55/110/170 s all failed (`running scripts is
disabled`). Manual PNGs exist beside those failures; they are not GPU
readback and not 814e evidence.

`06d` already states: 584d05bc predates `814e5293` and is not evidence
for or against the fix. This section agrees. Do not treat a 198 s
Alcher receipt as a responsiveness proof, and do not treat PowerShell
policy as an in-app defect.

## Capability inventory

### Current built-in

- Immutable per-process profile bind and three-pass resource identity.
- Panel off-thread prepare/validate with generation and queued Unlock.
- Shared profile CLI and `operator_home` (`HOME`, Windows `USERPROFILE`).
- `panel-play --smoke` (headed scene-2 shot) and `--live`.
- `tui-play --live` (no PNG).
- Panel dim-line git stamp (`alpha 1`).
- Explicit profile/cache/nav mismatch errors.
- Opt-in `memory-profile` samples (not a support tool).

### Current external (useful ideas, not code to import)

- Per-file SHA manifests and binary SHA receipts.
- Frozen host/client commit pairing on every launch record.
- Isolated engine fixture with fresh keys (admin only).
- Linux TUI release build with ELF/glibc evidence (Concord).
- Headless catalog driver with canonical Windows paths.

### Missing in the application

- `--version` / printed identity on panel, TUI, and host-play, including
  client gitlink and target triple.
- Offline `--check`: resolve+bind+validate, print paths/hashes, no
  GPU, vault, or login.
- Redacted support bundle (JSON + optional last shot).
- Smoke/live receipt JSON next to the shot/log.
- Windows-safe build timestamp; client SHA bake.
- Release-shaped missing-resource errors (no silent Mac `experiments/`
  defaults on foreign OS).
- TUI identity chrome.
- Packaging artifacts (zip + sidecar receipt) — belongs to operator
  automation, not `run_panel`.

### Obsolete / do not reintroduce

- Memory-campaign owner-capture, heaptrack, GPU shade probe, ConPTY
  campaign helpers as shipped tools.
- `catalog-watch` as a public binary name.
- In-app SSH, remote cargo, fixture tarball extract, staff givebank.
- Mandatory debug allocators or `memory-profile` in release.
- Per-tick world copies; a second scenario engine; mtime identity.

## Ownership and seams

| Concern | Owner | Surface |
|---|---|---|
| Resolve/bind/validate/print | `host-play` `profile.rs` + thin CLI flags on all three bins | `--check`, `--print-profile` |
| Prepared Play ticket | `host-play` `ValidatedTemplate` (already) | Panel only calls `run_prepared_template` |
| Responsive hashing | `panel` `app.rs` / `session.rs` (already) | Keep; TUI stays sync |
| Baked identity | shared `build_info` (panel today; move or duplicate into host-play/tui) | `--version`, panel/TUI chrome |
| Support bundle | panel/TUI command; host-play `--support-bundle DIR` | JSON writer in host-play so all three share redaction |
| Headed smoke | existing `panel --smoke` | add receipt JSON beside PNG |
| Headless smoke | TUI `--live` or host-play `--check` then optional script live | Linux/Concord without a display |
| Game-data | `tools/game-data` | build-time only |
| Nav bake | `nav-pack` | build-time / operator |
| Capture PNG | existing panel GPU readback (section A) | bundle may *attach* last shot; no new backend |
| Fleet/matrix | section B | not this report |
| SSH, fixtures, remote build, NASM, firewall | operator scripts / root | never linked into the app |

Security: support bundle and `--version` must not write vault bytes,
`BOT_VAULT_PASS`, `--vault-pass`, account passwords, `private.pem`, or
IsolatedEnv scratch. `LOGIN_RSAN`/`LOGIN_RSAE` are public; a bundle may
hash them, not dump them. Live/smoke IsolatedEnv must keep not setting
`HOME`. Normal interactive sessions must not load givebank or test
stores.

## What a three-OS binary should expose

1. **Identity.** `panel-play --version` / `tui-play --version` /
   `host-play --version` prints: public label, crate version, host
   commit, client commit, dirty, target triple, build time. Panel keeps
   the dim line. Release builds set `GIT_COMMIT`/`GIT_DIRTY=0` and a
   new `CLIENT_COMMIT` env so a tarball without `.git` still stamps.
   Replace `date -u` with a Rust/chrono-free UTC format that works on
   Windows builders (e.g. `SystemTime` formatting in `build.rs`, or
   require `BUILD_TIME` env at release).

2. **Resource/profile validation.** `… --check` runs
   `resolve_with_env` + `bind` (and stops). Prints `ServerProfile::label`,
   absolute cache/unpack/nav/vault/catalog paths, `cache_id`, nav
   availability, and the existing mismatch strings on failure. Exit 1
   on missing pack, cache change, public-274, or host/port drift. No
   window, no passphrase prompt. Hashing on this CLI thread is fine.

3. **Responsive preparation.** Already designed and committed for
   panel. Do not parallelize the three passes (RSS). Do not stream
   `hash_file` in this increment (06c out of scope). Native proof stays
   root-owned on Mac and Windows after `814e5293`.

4. **Support bundle.** Operator action: "Copy diagnostics". Writes
   `274bot-support/<stamp>/bundle.json` plus optional last PNG from the
   existing shot dir. JSON: identity, OS, resolved paths, cache/nav
   hashes, last `session.error`, last FAIL line, profile name/revision
   /target. No secrets. Host-play can write the same JSON without a UI.

5. **Clean runtime paths.** If the resolved engine/cache/nav path does
   not exist, fail with the path and the flag to set (`--cache`,
   `--nav-pack`, `ENGINE_DIR`). Do not imply `~/experiments/...` is a
   Windows or Linux install layout. Keep `~/.274bot` as the default
   *user* tree unless the operator chooses AppData (decision below).
   Align `default_vault_path_for` with revisioned `vault-289` or delete
   the target-only helper from user paths.

6. **Repeatable smoke.** Headed: `panel-play --smoke --profile …`
   already FAIL-closed on scene 2. Write `smoke.json` with identity,
   profile hashes, elapsed (not a performance claim), shot SHA if
   written, exit. Headless: `tui-play --live script_<smoke>` or
   `--check` alone for resource-only machines. Process exit 0 without
   the receipt is not proof.

## Staged implementation cards

Stop after card 5 unless the operator asks for packaging. Source
approval does not imply live acceptance.

1. **Shared build identity.** Move or clone `build_info`/`build.rs` so
   panel, tui, and host-play bake host+client SHA, dirty, target,
   `BUILD_TIME` without `date(1)`. Add `--version`. Tests: stamp
   helpers and "unknown" fallback. Files: `crates/panel/build.rs`,
   `crates/panel/src/build_info.rs`, new tiny module used by tui/host-play
   bins. No UI behavior change besides the existing dim line remaining.

2. **`--check`.** Shared parser flag handled before `run_panel` /
   TUI event loop / vault open. Reuse bind errors. Focused tests with
   the existing small profile fixtures (not 1 GB packs): missing pack,
   public-274 refusal, path print. Files: `host-play` profile + three
   `main`/`bin` parsers.

3. **Support bundle writer.** `host_play::support_bundle::write(dir, facts)`
   with an explicit denylist test (passphrase, vault path contents not
   copied, env keys). Panel menu / TUI key calls it. No network.

4. **Path/vault hygiene.** Missing default engine → actionable error.
   Make `default_vault_path` revision-aware or stop using it when a
   `ProfileSelection` exists. Windows path display can stay `Path`
   debug; do not add a second canonicalizer inside bind.

5. **Smoke receipt.** Extend `--smoke` (and TUI live PASS/FAIL) to
   write JSON beside the shot/log using the same identity fields.
   Existing smoke table tests stay; add one receipt-shape unit test.
   No new LIVE harness.

Root-owned, not these cards: native 814e beachball/Not Responding
proof; changed-resource refusal on disposable full-size packs;
unsigned zip of the three OS binaries plus sidecar receipt; Linux
glibc/ELF check copied from Concord's method.

## What stays outside the app

- SSH to Windows, Hyper-V, Concord; jump-host keepalives; PID hunting.
- Engine fixture archives, `npm ci`, Node `src/app.ts`, port binds.
- Staff givebank / moderator runtime patches.
- Remote `cargo` caches, NASM, portable Node, firewall rules.
- Source-export tarballs and per-file copy verifiers.
- PowerShell execution-policy workarounds and `CopyFromScreen`.
- Renaming `panel-play` to `catalog-watch.exe`.
- Auto-update, crash-phone-home, remote shell.

A packager script (repo `tools/` or operator machine) may: `cargo
build --locked --release -p panel --bin panel-play -p tui --bin tui-play
-p host-play`, record SHA-256, rustc, features (must not include
`memory-profile` unless requested), host/client commits, and zip the
three binaries with LICENSE/NOTICE. That script is not linked into
`panel-play`.

## Release-artifact checklist

For each OS artifact, before anyone calls it a release candidate:

- [ ] Binaries: `panel-play` and `tui-play`; `host-play` only if the
      operator includes the CLI (decision below).
- [ ] `cargo build --locked --release`; features default; no
      `memory-profile` / `memory-profile-no-alloc`.
- [ ] `GIT_DIRTY=0`, explicit `GIT_COMMIT` and `CLIENT_COMMIT` matching
      the gitlink; `--version` matches the sidecar.
- [ ] Sidecar JSON: host commit, client commit, binary SHA-256, bytes,
      target triple, rustc version, feature list, build time.
- [ ] LICENSE + NOTICE.md. No engine, cache JAGs, nav packs, vaults,
      RSA private keys, catalog scripts, or fixture databases.
- [ ] `--check` against operator-provided cache/nav fails closed when
      files are absent; succeeds and prints hashes when present.
- [ ] Headed Mac/Windows: `--smoke` or interactive first present paints
      during prepare (814e native proof). Linux without display:
      `tui-play --version` and `--check` only, unless a panel build is
      in scope.
- [ ] Windows builder does not require `date(1)` or Git inside the
      extracted tarball if env stamps are set.
- [ ] No `catalog-watch.exe` name. No givebank. No `BOT_VAULT_PASS` in
      the zip.

This checklist is not campaign catalog acceptance.

## Risks

1. Shipping `run_prepared_template` as the CLI default skips last
   validate. Keep `run_with_template` for TUI/host-play.
2. `--check` on the UI thread if someone wires it through `run_panel`.
   Parse and exit in `main`.
3. Support bundle copying `~/.274bot/vault` "for completeness".
   Deny by test.
4. Treating 584d05bc or 06b 37 s as a release budget.
5. Silent Mac engine defaults on Windows/Linux → confusing missing
   cache errors.
6. Baking `memory-profile` into a "diagnostics" release binary.
7. Joining hash workers on shutdown (reintroduces the stall).
8. Putting SSH or fixture extract behind a panel button.
9. Parallel hash passes (peak RSS).
10. README/public-port drift confusing operators; fix later, do not
    silently bind the old 274 public world.

## Focused validation (no LIVE from this card)

- Unit: `--version` unknown fallback; `--check` missing nav/cache
  strings; support-bundle denylist; vault helper 274 vs 289 path.
- Existing host-play mismatch tests remain (`cache changed`, flags
  changed, bound session refuses revision change).
- `cargo test -p panel --lib` / `-p host-play --lib` / `-p tui --lib`
  for the touched modules.
- Root later: Mac/Windows native paint-during-prepare and
  changed-resource refusal (06c § bounded native proof). Not claimed
  here.

## Operator decisions still required

1. **Linux product shape for 0.1.7 / alpha 2:** TUI-only (matches
   Concord) vs also shipping `panel-play` (wgpu, display).
2. **Is `host-play` a user-facing release binary** or developer-only?
3. **Windows user data directory:** keep `USERPROFILE\.274bot` (current,
   smallest change) vs `AppData\Roaming\274bot`. Recommend keep
   `.274bot` unless the operator wants a Windows-ism.
4. **Artifact format:** unsigned zip-of-binaries (recommend for alpha)
   vs signed pkg/msi/appimage/notarization. Signing is operator/infra.
5. **Public label:** keep `alpha 1` / crate `0.1.0` or bump to the
   stated host alpha 2 / 0.1.7 when tagging. Hand-bump `RELEASE`.
6. **Support bundle screenshots:** attach last PNG (useful, already
   captured) vs JSON-only (more private). Recommend attach, with the
   existing empty-F12 snapshot honesty from section A.
7. **Client SHA in the visible dim line** vs hover-only. Recommend
   hover/tooltip and `--version`, not the dim line.

## Suggested stopping point

Ship cards 1–5 (identity, `--check`, support bundle, path/vault
hygiene, smoke receipt). That is the useful three-OS application
surface. Do not start installers, auto-update, remote diagnostics, or
in-app fixture management. Native 814e proof and zip packaging stay
with root. Section D should sequence these behind the capture (A) and
run-control (B) reports without merging SSH into the product.
)
