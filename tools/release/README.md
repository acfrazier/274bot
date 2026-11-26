# Release packaging

## End-to-end release entry point

`release.py` is the checked-in controller and native worker for all three
platforms:

```sh
TAG=0.2.9   # after `[workspace.package] version` is `0.2.9`; a numeric patch tag may begin with that version
COMMIT="$(git rev-parse HEAD)"
python3 tools/release/release.py build --commit "$COMMIT" --platform all --dry-run
python3 tools/release/release.py finalize --commit "$COMMIT" --platform all \
  --tag "$TAG" --dry-run
python3 tools/release/release.py verify --commit "$COMMIT" --platform all --dry-run
```

Dry runs only inspect the requested commit and print the complete plan. They do
not create a work directory, connect to a builder, build, sign, notarize, tag,
push, or publish.

Before packaging, sweep every tracked doc against the release commit:
`README`, `FIRST-START`, `CONTRIBUTING`, `CONTEXT`, `NOTICE`, `docs/**`
and the tools READMEs. Fix status and version words, removed or renamed
features, and new features, so the docs describe the candidate. Set
`[workspace.package] version` in `Cargo.toml` and `RELEASE` in
`crates/panel/src/build_info.rs` first; the package version and the app's
display name come from them. `package.py` stages only `LICENSE`,
`NOTICE.md` and `FIRST-START.md` into the package; the other docs are read
from the repository at the release commit.

For a real candidate, provide all four local generator roots and the actual
`RS2B0T` query source. Revision 289 also requires the decoded snapshot used
for its map and terrain bundle:

Keep `WORK` and `ARTIFACTS` under the request evidence directory, outside the
local home directory. Set `RELEASE_LINUX_ROOT` and `RELEASE_WINDOWS_ROOT` to
absolute, request-specific evidence directories outside each builder's home
directory (for example, `/evidence/request/linux` and
`D:/evidence/request/windows`). Remote operations use those roots verbatim.

```sh
python3 tools/release/release.py build \
  --commit "$COMMIT" --platform all --work-dir "$WORK" \
  --linux-remote-root "$RELEASE_LINUX_ROOT" \
  --windows-remote-root "$RELEASE_WINDOWS_ROOT" \
  --game-data-274-engine "$GAME_DATA_274_ENGINE" \
  --game-data-274-content "$GAME_DATA_274_CONTENT" \
  --game-data-289-engine "$GAME_DATA_289_ENGINE" \
  --game-data-289-content "$GAME_DATA_289_CONTENT" \
  --rs2b0t "$RS2B0T" \
  --snapshot-root "$SNAPSHOT_ROOT" --snapshot-version "$SNAPSHOT_VERSION" \
  --sign-identity "$RELEASE_SIGN_IDENTITY" \
  --rusty-v8-archive "$RUSTY_V8_ARCHIVE"
```

Each engine root contains its revision's `data/pack/client`; each content root
is that revision's local content tree. `RS2B0T` is the actual local bank/cook
query source, not a pinned checkout. The snapshot root contains the named
decoded snapshot directory. `--client-root` defaults to
`vendor/fr-client-rust`. The controller reads the client commit from the host
commit's gitlink and exports both exact commits into `source.tar.gz`. It
transports all four game-data roots, `RS2B0T`, and the optional snapshot in
`inputs.tar.gz` without game-input digests or pins. `.git` metadata is excluded
and symlinks are refused.

Real builds require the host and client checkouts at their requested commits
with no tracked or untracked changes. `release.py` and `windows-ssh.py` must
match that commit, so uncommitted controller code cannot become the transported
native worker. Workers verify the source archive receipt, then extract the
source and local inputs into a fresh per-commit workspace.

Before the product build, each worker runs the shared local bootstrap:

```text
GAME_DATA_274_ENGINE=.../game-data/274/engine
GAME_DATA_274_CONTENT=.../game-data/274/content
GAME_DATA_289_ENGINE=.../game-data/289/engine
GAME_DATA_289_CONTENT=.../game-data/289/content
RS2B0T=.../rs2b0t
python tools/local-data/build.py
```

The wrapper runs generator stage 1, builds and runs the isolated `nav-bake`
tool for both revisions, exports the real schema-3 crossings projection, and
runs generator stage 2. It stages local navigation under
`target/local-data/nav/<revision>/`; stage 2 adds the actual prerequisite facts
derived from the exported projection. Product building follows only after the
wrapper succeeds. Release builds set `GIT_DIRTY=0` and `BOT_NAV_BUILD=require`,
then require Cargo evidence that the API used `embedded-game-data` and was not
compiled with `no_local_game_data`.

The native worker resolves Bun before it restricts the macOS build environment.
The wrapper receives that executable path through `BUN_EXECUTABLE`.
Cargo retains the restricted `PATH` during both navigation and product builds.

The same-build receipt records the exact host/client commits, target,
`rustc -Vv`, default features, and each shipped binary/navigation file's SHA-256.
It does not carry game-input digests. Revision-289 packages also contain the
terrain and map catalogue baked by the staged TUI from the local 289 cache and
decoded snapshot.

Completed `prepared/` state is reusable: a one-platform retry verifies the
source archive receipt and release identity, then reuses the transported
`inputs.tar.gz`. If local generator or map inputs intentionally change, use
`--reset-prepared` to replace the shared source/input payload before staging.
Use `--force-platform` to discard only the selected native staging and rebuild
it. An interrupted initial prepare removes its partial directory automatically.

### Builders and parameters

The host and client pin Rust **1.98.1** with Clippy and rustfmt. Prepare that exact
toolchain in each builder's Rustup home before a release build. Do not use
unqualified `stable` to qualify a release.

- macOS arm64 builds locally. `--sign-identity` (or
  `RELEASE_SIGN_IDENTITY`) is required for a real release build.
  `--keychain PATH` or `RELEASE_KEYCHAIN` selects the keychain used for
  identity lookup and signing; omit it to use the system default keychain.
  The macOS worker never builds with the operator's login environment: the Mac's
  login `PATH` puts MacPorts and Homebrew ahead of `/usr/bin`, and in the
  0.2.1 build MacPorts' `pkg-config` answered the `bzip2-sys` probe with
  `-L/opt/local/lib`, which would have shipped binaries linked against
  `/opt/local` libraries users do not have. The worker's environment is
  therefore an allowlist built from empty, not a scrubbed copy of the
  login environment. It constructs the fixed `PATH`, `HOME`, and
  `MACOSX_DEPLOYMENT_TARGET=11.0` (pinned to the documented macOS 11.0
  floor, never inherited; `SDKROOT` stays unset so `xcrun` rediscovers
  the SDK). `PATH` is `$CARGO_HOME/bin:/usr/bin:/bin:/usr/sbin:/sbin` when
  the caller sets `CARGO_HOME`, otherwise
  `$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin`. Then the worker copies
  only `USER`/`LOGNAME`, `TMPDIR`, `LANG`/`LC_*`, the proxy variables,
  `CARGO_HOME` and `RUSTUP_HOME` when they are set, and a `DEVELOPER_DIR`
  that is an existing `/Library/Developer/CommandLineTools` or
  `/Applications/*.app/Contents/Developer` (`xcrun` finds the Apple
  toolchain through it; any other value is dropped), plus the keys the
  worker itself pins. Everything else -- `LIBRARY_PATH`, `PKG_CONFIG*`,
  `DYLD_*`, compiler/flag overrides, `RUSTFLAGS`, and `RUSTUP_TOOLCHAIN`
  (the toolchain comes from `rust-toolchain.toml`) -- is not inherited.
  Each worker constructs `CARGO_HOME` as `<worker_root>/cache/cargo`, so
  Cargo uses request-local config and cache files rather than the builder
  account's `~/.cargo`. None of the dropped
  variables was set in the environment that produced the known-good
  0.2.1 package. Linux and Windows retain their platform-specific inherited
  environment but also use this worker-local `CARGO_HOME`.
- Linux x64 uses the SSH target `274bot-builder` by default. Override it with
  `--linux-host` or `RELEASE_LINUX_HOST`.
- Windows x64 runs over the promoted `windows-ssh.py` transport.
  `--windows-host` or `RELEASE_WINDOWS_HOST` is required. The Rusty V8 archive
  is supplied as `--rusty-v8-archive` or `RUSTY_V8_ARCHIVE`; no builder path is
  stored in the repository.
- Identity and known-hosts files are optional
  `--linux-identity`/`--linux-known-hosts` and
  `--windows-identity`/`--windows-known-hosts` parameters (with matching
  `RELEASE_*` environment variables). Batch mode and strict host-key checking
  are always enabled.
- Remote roots are required for real Linux and Windows operations. Set
  `--linux-remote-root` / `--windows-remote-root` (or
  `RELEASE_LINUX_ROOT` / `RELEASE_WINDOWS_ROOT`) to absolute request-evidence
  directories: a POSIX path such as `/evidence/request/linux` on Linux and a
  drive-qualified path such as `D:/evidence/request/windows` on Windows. The
  controller uses these paths directly and never joins them to remote `$HOME`.
  Cargo output is cached below each platform's per-commit workspace, so
  artifacts from different commits cannot mix.
- `--jobs` defaults to 4, `--revision` to 289, and the macOS app profile to
  `rs2b2t`.

No username, key path, cache path, signing identity, or credential is
hard-coded; the Linux host is only the documented, overridable builder alias.
`windows-ssh.py --help` documents its standalone `run`, `put`, and `get`
operations.

### Finalize and verify

After all native package directories exist:

```sh
python3 tools/release/release.py finalize \
  --commit "$COMMIT" --platform all --work-dir "$WORK" \
  --artifact-dir "$ARTIFACTS" --tag "$TAG" \
  --release-notes "$RELEASE_NOTES"

python3 tools/release/release.py verify \
  --commit "$COMMIT" --platform all --artifact-dir "$ARTIFACTS"
```

Finalize appends the credits footer (`tools/release/notes-footer.md`) to the
release notes once and writes the result to `$WORK/$COMMIT/RELEASE-NOTES.md`.
That file is what ships in every package; use it as the GitHub release body too.

Finalization first refuses a tag that is neither the Cargo version nor a
numeric patch tag beginning with that version (for example, version `0.2.0`
may finalize as `0.2.0.1`). It also requires the staged manifest's version and
public release name to match the pinned source. The final package and archive
are named from the tag (`274bot-0.2.0.1-linux-x64.tar.gz`), so a patch release
never reuses the base release's archive names; the build stages under the Cargo
version and finalize renames the staged directory. It then copies the release
notes, records platform runtime requirements, rehashes every package file,
creates the native archive, downloads remote archives, and regenerates
`SHA256SUMS` from only the candidate's expected archive names.

On macOS it submits the signed zip, requires Apple's `Accepted` result, records
the submission id immediately, staples and validates `274bot.app`, then
rehashes and recreates the archive. A retry after stapling or Gatekeeper
failure reuses that accepted id rather than resubmitting. The keychain profile
defaults to `274bot` and is parameterized by `--notary-profile` or
`RELEASE_NOTARY_PROFILE`. Credentials remain in the local keychain.

Verification safely extracts every archive, rejects traversal, links, duplicate
members, extra files, missing files, byte-count differences, and SHA-256
differences, and verifies `SHA256SUMS` has exactly the candidate archives. It
checks the requested host commit, runs both executables' `--help` on their
native platforms, and validates the extracted macOS signatures, staple, and
Gatekeeper acceptance. On macOS every Mach-O in the package must first
pass the architecture rule: the macOS release builds a single target
(`aarch64-apple-darwin`, packaged as `macos-arm64`), so only a *thin*
64-bit arm64 Mach-O is shipped. Verification reads each file's Mach-O
header directly and refuses anything else -- a fat/universal header
whatever its slices, a thin x86_64 or arm64e binary, a 32-bit or
truncated header -- naming the binary and what it is, without running
`otool` on it, so no tool output can hide a slice. It then runs
`otool -arch arm64 -L`/`-l` on every Mach-O (both standalone
executables and the app bundle's) and refuses any library reference
that is not, after path normalisation, under `/usr/lib/` or
`/System/Library/` or an `@executable_path`/`@rpath` reference
resolving to a file inside the package, naming the offending binary
and library. Because dyld expands `@executable_path` to the main
executable's directory -- and the package ships main executables at
two depths -- such a reference must resolve inside the package from
*every* shipped main executable's directory, not just the referencing
binary's; `@loader_path` as an install name is refused, so in-package
dylibs use `@rpath` plus an `@loader_path` rpath. It also refuses any
Mach-O whose minimum macOS version is missing or above 11.0, naming
the binary and its value: the only floors that count are a
macOS-platform `LC_BUILD_VERSION` (`platform 1`) `minos` or an
`LC_VERSION_MIN_MACOSX`; a build version for another platform (iOS,
Mac Catalyst, ...) is ignored, so a binary with no macOS floor is
refused as missing one. The worker pins
`MACOSX_DEPLOYMENT_TARGET=11.0`, and this is how verification proves
the shipped floor. Those checks are what keep a MacPorts/Homebrew-linked,
wrong-architecture, or floor-raising build from ever verifying.
Linux and Windows have no equivalent check yet, deliberately: ELF
`DT_NEEDED` entries and PE imports record library names, not
build-host paths, both builders are dedicated hosts with no shadowing
package manager on `PATH`, and their runtime floors are already pinned
in the manifest (glibc 2.39 and the named system libraries on Linux,
the Visual C++ runtime DLLs on Windows). A `readelf`/`dumpbin`
dependency allowlist would be a different mechanism
for a defect class neither builder has shown.

Each package's navigation payloads are checked against its same-build receipt
and release manifest. Verification does not compare navigation bytes across
platforms. Verification does not publish anything.

### Python version

The `build`, `finalize`, `verify`, and `clean` steps run with the worker's
own `python3` (`python` on Windows). They require Python 3.9 or newer,
including on a macOS worker over SSH where `python3` may be Apple's 3.9.

### Known issues in release notes

Open known issues and deferred user-visible work live as GitHub issues in
[acfrazier/274bot](https://github.com/acfrazier/274bot/issues?q=label%3Aknown-issue)
with the `known-issue` label. At release cut, list each relevant issue in the
release notes, link its entry to the GitHub issue, and state the known
workaround or that none is known.

### Retry and cleanup

The retry controls are deliberately scoped to the release work root:

```sh
# Rebuild one native platform without changing prepared archive identities.
python3 tools/release/release.py build ... --platform linux --force-platform

# Remove selected platform staging explicitly, keeping prepared/.
python3 tools/release/release.py clean --commit "$COMMIT" \
  --work-dir "$WORK" --platform linux --mode reset-platform

# After release verification, retain only package directories/archives,
# build/finalize receipts, and the macOS notarization id on each builder.
python3 tools/release/release.py clean --commit "$COMMIT" \
  --work-dir "$WORK" --platform all --mode retain

# Deliberately discard shared preparation (local only).
python3 tools/release/release.py clean --commit "$COMMIT" \
  --work-dir "$WORK" --mode reset-prepared
```

Mac finalization itself is resumable as described above. Remote cleanup uses
the same parameterized SSH settings as build/finalize and never touches paths
outside the per-commit release root.

The controller intentionally has no tag, push, upload, or GitHub-release
operation. Those remain explicit operator actions after native verification.

The package version comes from `[workspace.package]` in `Cargo.toml`; the public
name comes from `crates/panel/src/build_info.rs` `RELEASE` (`beta 2`). Release targets:

- macOS ARM64: Developer ID signed `274bot.app`, `panel-play`, `tui-play`.
- Windows x64: `panel-play.exe`, `tui-play.exe`.
- Linux x64: `panel-play`, `tui-play` (see [Linux panel runtime](#linux-panel-runtime)).

`host-play` remains a developer tool. Build default features with
`cargo build --locked --release`; do not enable profiling features. Before the
product build, run `python tools/local-data/build.py` with local source roots
for both revisions and `RS2B0T`. This generates embedded game facts and stages
navigation payloads for revisions 274 and 289. `BOT_NAV_BUILD=skip` means no
staging and must not be used for a release build.
`package.py` takes a Cargo profile directory and a build receipt. The receipt
contains `host_commit`, `client_commit`, `target`, `rustc`, `features` (the
string `default`), `built_at`, and `files` mapping each binary and navigation
artifact's relative path to its SHA-256. It verifies hashes before staging.
It copies only the selected binaries, navigation artifacts, and public docs;
it excludes machine-local bake stamps, engine/cache data and credentials.

Revision 289 packages also ship the WalkTo map catalogue and terrain. After
staging the binaries, `package.py` runs the staged `tui-play --map-bundle`
against the local revision-289 cache from `GAME_DATA_289_ENGINE` and the
decoded snapshot: `--map-cache` is its `data/pack/client` directory and
`--map-unpack` is the snapshot root. Both are required, either explicitly or
through `GAME_DATA_289_ENGINE` plus `BOT_NAV_SNAPSHOT_ROOT`; there are no
operator-filesystem defaults.
The bake runs the production map-cache path (the same producer, writer and
publication as a local bake; the image demand derives its catalogue first, so
one bake yields both) in a scratch cache and ships the published directories
as `map/289/catalogues/<key>/` (catalogue `manifest.json` plus
`client-pois.json`) and `map/289/images/<key>/` (image `manifest.json` plus
terrain PNGs), with `map/289/274bot.mapcatalogue.json` and
`map/289/274bot.mapimages.json` recording each bundle's key, manifest receipt,
totals, and map identity. `package.py` checks the catalogue and terrain against
their own output receipts; it no longer compares catalogue identity to a
navigation manifest. The release manifest records the descriptions and every
shipped file, and the macOS app copies `map/` into
`Contents/Resources` beside `nav/`. The app installs each artifact only when
its identity matches the bound client cache exactly; otherwise the catalogue
is derived locally and the local terrain bake stays behind the operator's
consent.

Example, after recording the build receipt:

```sh
python3 tools/release/package.py --platform macos \
  --input target/release --output "$WORK/274bot-$VERSION-macos-arm64" \
  --build-receipt "$WORK/macos-build.json" \
  --app-profile rs2b2t --sign-identity "$RELEASE_SIGN_IDENTITY" \
  --map-cache "$GAME_DATA_289_ENGINE/data/pack/client" \
  --map-unpack "$SNAPSHOT_ROOT"
```

`--check` (no `--output`, no signing) stages the same package into a
temporary directory, verifies every staged file against its release manifest,
prints a JSON summary (file count, bytes, shipped map bundle) and removes it.

The app's Finder launch selects `rs2b2t` through its Info.plist environment.
The standalone executables retain their normal CLI profile selection. The
bundle has its own navigation and map resources under Contents/Resources;
standalone binaries use the adjacent nav and map directories. Keep each layout
intact. The app bundle is the macOS deliverable for another reason too: macOS
accessibility tooling, which drives the panel in headed checks, has not worked
on a bare binary launch and needs `274bot.app`.

macOS signing enables hardened runtime with only the JIT entitlement needed
by V8. Test script execution after signing. Verify signatures and native launch
from an extracted archive. Notarize the signed archive with `xcrun notarytool
submit ... --keychain-profile PROFILE --wait`, staple the accepted ticket to
the app, then regenerate the archive and hashes. Signing alone is not a
notarization result. Credentials belong in the keychain, never this repository.

Packaging does not publish, tag, notarize, or imply native qualification.
Retain host/client identities, per-file hashes, architecture/runtime dependency
checks and smoke results with each candidate. Release notes must disclose the
remaining catalog limitations; historical scoped script passes do not imply
all scripts/options passed with a new binary.

Distributed packages target `rs2b2t`. Build local game facts and navigation
before the product build with `python tools/local-data/build.py`, setting all
four `GAME_DATA_274_*` and `GAME_DATA_289_*` source roots and `RS2B0T` to the
intended local inputs. Use the same revision-289 engine cache and decoded
snapshot for the package's map and terrain bundle. `NAV_PACK` is a runtime
override only; the build does not authenticate navigation against a selected
cache identity.

## Linux panel runtime

The Linux package ships `panel-play` beside `tui-play` with the same adjacent
`nav/289/` and `map/289/` resources as the Windows package; the panel's fonts
are compiled in. Build both binaries in the release build (`cargo build
--locked --release`). They are built on Ubuntu 24.04 and need glibc 2.39 or
newer. The builder stays on 24.04 on purpose: its glibc is the package's
minimum. Newer Ubuntu is covered by test environments (the `ubuntu-26.04` CI
lane), never by the builder.

- **Display:** an X11 or Wayland session. winit loads the display libraries at
  runtime: libX11, libX11-xcb, libXcursor, libXi, libxcb, libxkbcommon and
  libxkbcommon-x11 on X11; libwayland-client, libwayland-cursor and
  libxkbcommon on Wayland.
- **Graphics:** the panel draws with wgpu's Vulkan backend, so it needs the
  Vulkan loader (`libvulkan.so.1`) and a Vulkan driver: the GPU vendor's, or
  Mesa lavapipe (`libvulkan_lvp.so`) for software rendering. There is no
  OpenGL fallback for the window. `BOT_CPU=1` draws the game view with the CPU
  rasterizer instead of the GPU renderer; the window still presents through
  Vulkan.
- **Linked libraries** (`ldd` on the builder build): `panel-play` links
  `libasound.so.2` (ALSA, game audio), `libssl.so.3` and `libcrypto.so.3`
  (OpenSSL 3), `libgcc_s.so.1`, `libm.so.6` and `libc.so.6`; `tui-play` links
  the same except ALSA and needs no display or Vulkan.
- **Ubuntu 24.04 packages:** `libasound2t64 libssl3t64 libx11-6 libx11-xcb1
  libxcursor1 libxi6 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0
  libwayland-cursor0 libvulkan1 mesa-vulkan-drivers`.

The Linux package is checked with `package.py --check --platform linux` on the
builder. A real launch of the staged `panel-play` needs a display and Vulkan
driver as listed above; `tui-play` needs neither.

## Saved-data upgrade gate

`upgrade_test.py control` tests historical startup without logging in to a game.
Use tmux and Python with `cryptography`. Supply the preserved **test** snapshot from the
0.2.8 build. Never supply an operator profile or vault. Run the command once
with the TUI at `0dd33c2ed` and once with the TUI at `eaff8cbc9`:

```sh
python3 upgrade_test.py control --binary "$TUI" --snapshot "$SNAPSHOT_028" \
  --pass-file "$TEST_PASSPHRASE_FILE" --account "$TEST_ACCOUNT" --cache "$CACHE" \
  --crc-file "$ARCHIVED_CRC" --work "$NEW_DIR"
```

Set `ENGINE_DIR` for the historical local profile. Supply the complete revision-289
client cache and its original 40-byte `/crc` response. The control serves only
that response on loopback. It does not use a running engine or edit the cache.
The startup probe checks admission and vault unlock. Startup alone does not
prove that loadouts or sparse UI preferences were parsed. On the positive run,
the tool reads the unassigned Bronze axe, Lobster quantity one and the manual
movement preference in the TUI. The unscoped hint is tested for startup
admission, not for bank contents. A separate encrypted vault contains sparse
profile settings, a schema-4 Gatherer entry and an empty compiled envelope.
The tool supplies existing profile names before an absent name. Only that
last lookup proves that startup and unlock completed. A timeout is an error.
The tool also presses Start on the real 0.2.8 Gatherer. It checks the schema-4
refusal or successful admission and restoration of `Mining`. The old binary
must return 1 for a format refusal. The fixed binary must return 0. An
infrastructure failure returns 2 and cannot satisfy the negative control.
Repeat `control` with the candidate executable and a new work directory. Require
exit 0. Keep this result and the two historical controls with the live verdict.
None of these offline results qualifies a live login or bank visit.

For the release gate, use the **full shipped 0.2.9 package**, not a source
build. Verify the package against its published checksum. Obtain both
executable checksums from that verified package. Start from an interactive
terminal outside tmux. The tool requires tmux, a display, and the release
test engine. Use two disposable test accounts and leave `RS2B0T` unset. Keep
the work directory private. It contains an encrypted test vault and session logs.

```sh
python3 upgrade_test.py live --shipped "$OLD_TUI" --shipped-panel "$OLD_PANEL" \
  --shipped-sha256 "$OLD_TUI_SHA" --panel-sha256 "$OLD_PANEL_SHA" \
  --candidate "$NEW_TUI" --account "$TEST_ACCOUNT" --shared-account "$SECOND_ACCOUNT" \
  --cache "$CACHE" --work "$NEW_DIR"
```

1. In the shipped panel, open Browse and choose **Not now** at the catalog
   prompt. Load the `UpgradeProbe.ts` file at the path printed by the tool.
   With no profile focused, set its `mode` to `shared`. Enable session-file
   logging and quit. Do not import the catalog yet.
2. The tool renames the shipped loopback entry in `servers.json` to `upgrade-289`.
   In 0.2.9 the user edits that file directly (see `docs/reference/server-profiles.md`).
   The rename preserves the local endpoints, revision and vault. It creates
   a name that the candidate cannot resolve from built-in defaults.
3. In the shipped TUI, create both test accounts. Enable automatic login.
   Do not edit or start a script on the second account. Focus the primary
   account and log in. Disable “Pause script on manual movement.” Create
   just one loadout, named `upgrade`, with carry item `Lobster` and quantity `7`.
4. Open a real bank with at least one item, then close it. Keep logging enabled.
5. On the primary account select `UpgradeProbe` from **File**, set `mode` to
   `seed`, start it, then stop it after its `UPGRADE_READ` log appears. Set
   `mode` to `profile`. This gives the runtime store and the per-profile
   setting distinct values. The probe makes no game actions.
6. Set Gatherer `skill` to `Mining`, Grinch `radius` to `7`, and Quester
   `Maximum deaths` to `3`. Leave unqualified guides off. Turn global teleport
   permission off and turn Sherlock's `Allow teleports` on. A global override
   is not evidence that the card kept its setting.
7. Close all dialogs. Leave the primary account focused and the script idle.
   Detach with the tmux prefix followed by `d`. Do not quit the TUI.
8. The tool tests the deferred catalog on a private copy before import. The
   candidate must show the deferred-import row and start the file card with
   the shared setting on the second account. When the shipped panel opens
   again, import the printed `upgrade-catalog` directory and quit. Import
   clears the defer marker, so the two checks require distinct writer states.

The final candidate must show the custom endpoint, restored loadout, movement
preference and catalog card. Its new session log must report the writer's bank
row count and observation time before another bank visit. The file script must
read both its profile setting and its scoped raw runtime value on Start. The tool
also checks the compiled settings and requires each compiled script to reach
working or waiting. The candidate receives no setting edits. A move-aside or a
missing observation fails the gate. No candidate database schema is assumed.
