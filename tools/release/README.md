# Release packaging

## End-to-end release entry point

`release.py` is the checked-in controller and native worker for all three
platforms:

```sh
TAG=0.2.7   # after `[workspace.package] version` is `0.2.7`; a numeric patch tag may begin with that version
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

For a real candidate, give the controller a local work directory and the three
canonical revision-289 input roots:

```sh
python3 tools/release/release.py build \
  --commit "$COMMIT" --platform all --work-dir "$WORK" \
  --engine-dir "$ENGINE" --content-dir "$CONTENT" \
  --snapshot-root "$SNAPSHOTS" --snapshot-version "$SNAPSHOT_VERSION" \
  --sign-identity "$RELEASE_SIGN_IDENTITY" \
  --rusty-v8-archive "$RUSTY_V8_ARCHIVE"
```

The engine argument is the engine root containing `data/pack/client`; the
content argument is the canonical content tree; the snapshot root contains the
named decoded snapshot directory. `--client-root` defaults to
`vendor/fr-client-rust`. The controller reads the client commit from the host
commit's gitlink and refuses malformed or mismatched identities. It exports
both exact commits into `source.tar.gz`, exports only the pinned build inputs
into `inputs.tar.gz`, and writes `expect.json` with archive SHA-256 records and
the SHA-256 tree digest plus file count for pack, content, and snapshot.
`.git` metadata is excluded and symlinks are refused.
Real builds require both host and client checkouts to be at the requested
commits with no tracked or untracked changes. `release.py` and
`windows-ssh.py` must match that commit, so uncommitted controller code cannot
become the transported native worker.

Every native worker verifies both archives, extracts them into a fresh
per-commit workspace, recomputes all three input digests, and then builds:

```text
GIT_DIRTY=0
BOT_NAV_REVISION=289
BOT_NAV_BUILD=require
cargo build --locked --release -p panel --bin panel-play -p tui --bin tui-play
```

It records the exact host/client commits, target, `rustc -Vv`, default features,
input digests, and every binary/nav digest in the build receipt before calling
`package.py`. Each platform therefore ships `panel-play`, `tui-play`,
revision-289 navigation, and the terrain baked by the staged TUI from the same
pinned cache and snapshot.

Completed `prepared/` state is reusable: a one-platform retry verifies the
existing archives, manifest, and current input tree digests instead of
regenerating shared payload hashes. Use `--force-platform` to discard only the
selected native staging and rebuild it. Use `--reset-prepared` only when the
pinned source or inputs intentionally changed; it deletes and recreates the
shared payload before staging the requested platform. An interrupted initial
prepare removes its partial directory automatically.

### Builders and parameters

- macOS arm64 builds locally. `--sign-identity` (or
  `RELEASE_SIGN_IDENTITY`) is required for a real release build. The macOS
  worker never builds with the operator's login environment: the Mac's
  login `PATH` puts MacPorts and Homebrew ahead of `/usr/bin`, and in the
  0.2.1 build MacPorts' `pkg-config` answered the `bzip2-sys` probe with
  `-L/opt/local/lib`, which would have shipped binaries linked against
  `/opt/local` libraries users do not have. The worker's environment is
  therefore an allowlist built from empty, not a scrubbed copy of the
  login environment. It constructs the fixed `PATH`
  `~/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin`, `HOME`, and
  `MACOSX_DEPLOYMENT_TARGET=11.0` (pinned to the documented macOS 11.0
  floor, never inherited; `SDKROOT` stays unset so `xcrun` rediscovers
  the SDK), then copies only `USER`/`LOGNAME`, `TMPDIR`, `LANG`/`LC_*`,
  the proxy variables, and a `DEVELOPER_DIR` that is an existing
  `/Library/Developer/CommandLineTools` or
  `/Applications/*.app/Contents/Developer` (`xcrun` finds the Apple
  toolchain through it; any other value is dropped), plus the keys the
  worker itself pins. Everything else -- `LIBRARY_PATH`, `PKG_CONFIG*`,
  `DYLD_*`, compiler/flag overrides, `RUSTFLAGS`, and in particular
  `CARGO_HOME` and `RUSTUP_TOOLCHAIN` (the toolchain comes from
  `rust-toolchain.toml`) -- is not inherited. None of the dropped
  variables was set in the environment that produced the known-good
  0.2.1 package. Linux and Windows environment construction is
  unchanged.
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
- Remote work roots default to the home-relative `274bot-release` and can be
  changed with `--linux-remote-root` / `--windows-remote-root` (or
  `RELEASE_LINUX_ROOT` / `RELEASE_WINDOWS_ROOT`). Cargo output is
  cached below the platform's per-commit workspace, so artifacts from different
  commits cannot mix. Remote roots must stay below the remote home directory.
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
for a defect class neither builder has shown. Navigation is reported
byte-identical only after all six files from all three platform archives
compare equal; a partial per-platform verification explicitly reports that navigation was not compared.
Verification does not publish anything.

### Python version

The known-issues ledger is TOML and is read with `tomllib`, which exists
only on Python 3.11 and newer. `tools/known-issues/known_issues.py` needs
3.11+ for every command, and `release.py preflight` (run on the
orchestrator host) needs 3.11+ because it loads that tool. On an older
interpreter both exit with a one-line error naming the running interpreter
instead of a traceback. Run those commands with an interpreter that meets
the floor. On macOS that is the Homebrew `python3`
(`/opt/homebrew/bin/python3`, 3.11 or newer); Apple's `/usr/bin/python3`
is 3.9 and fails the check.

The `build`, `finalize`, `verify`, and `clean` steps run with the worker's
own `python3` (`python` on Windows) and need only what `release.py` already
needs (3.9 today), including on a macOS worker over SSH where `python3` is
Apple's 3.9.

### Known-issues ledger

`docs/known-issues.toml` is the single source of truth for the
`### Known issues` and `### Not in this release` blocks in `CHANGELOG.md`.
Edit the ledger, never the CHANGELOG blocks. Item lifecycle: add the item
with `first` (the first affected release), and `listed` (the first release
whose notes list it) when it was found after `first` had shipped. The commit
that fixes it sets `fixed` to the release carrying the fix; never delete an
item. An item is open at release X when `listed` (default `first`) is not
newer than X and `fixed` (when set) is newer than X, so a fix recorded for
0.2.5 leaves the published 0.2.4 section untouched and its check passing.
`fixed` must name a known release or the next one and must be later than
`first`; `listed` must name a known release or the next one, not be earlier
than `first`, and be earlier than `fixed`. Unknown fields are rejected, so a
typo cannot silently keep an item open.

The `## Unreleased` section stages no ledger blocks during development.
When a release section is cut, render the blocks for the next release,
paste them over the old ones, then check:

```sh
python3 tools/known-issues/known_issues.py render --release 0.2.7
python3 tools/known-issues/known_issues.py check --release 0.2.7
python3 tools/known-issues/known_issues.py carried --release 0.2.7
```

`check` fails when the CHANGELOG blocks differ from the render. `carried`
lists every item open at the release with its carry count and flags
`needs ruling` when the release reaches the item's target or the item has
been carried through two or more releases; it informs the release approval
and never blocks it. There is no `check --unreleased`: it used to fail
whenever the blocks were not staged yet, which is the normal state all
through development. `preflight` runs the same check against a commit's own
tree before packaging:

```sh
python3 tools/release/release.py preflight --commit "$COMMIT" --tag 0.2.7
```

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
`cargo build --locked --release`; do not enable profiling features. Build from
an exact source export or clean checkout with the recorded client gitlink.
Navigation generation requires the canonical engine/content inputs described
in `docs/api/nav.md`; release builds must not use `BOT_NAV_BUILD=skip`.

`package.py` takes a Cargo profile directory and a build receipt. The receipt
contains `host_commit`, `client_commit`, `target`, `rustc`, `features` (the
string `default`), `built_at`, and `files` mapping each binary and navigation
artifact's relative path to its SHA-256. It verifies hashes before staging.
It copies only the selected binaries, navigation artifacts, and public docs;
it excludes machine-local bake stamps, engine/cache data and credentials.

Revision 289 packages also ship the WalkTo map catalogue and terrain. After
staging the binaries, `package.py` runs the staged `tui-play --map-bundle`
against the pinned client cache the nav bundle was built from: `--map-cache`
(its jag directory) and `--map-unpack` (the snapshot root holding its decoded
snapshot). Both are required, either explicitly or through
`BOT_NAV_ENGINE_DIR` (or `ENGINE_DIR`) plus `BOT_NAV_SNAPSHOT_ROOT`; there are
no operator-filesystem defaults.
The bake runs the production map-cache path (the same producer, writer and
publication as a local bake; the image demand derives its catalogue first, so
one bake yields both) in a scratch cache and ships the published directories
as `map/289/catalogues/<key>/` (catalogue `manifest.json` plus
`client-pois.json`) and `map/289/images/<key>/` (image `manifest.json` plus
terrain PNGs), with `map/289/274bot.mapcatalogue.json` recording the catalogue
identity (revision, decoded client content, derive policy), key, the
manifest's size and SHA-256, the record count and the payload size, and
`map/289/274bot.mapimages.json` recording the same for the terrain.
`package.py` refuses a bundle whose content identity differs from
`274bot.navpack.json`'s `content_id`, re-verifies every shipped file against
those receipts, records the descriptions as `map_catalogue` and `map_images`
in `release-manifest.json` (whose `files` list, and so the archive
`SHA256SUMS`, covers every shipped file), and copies `map/` into the macOS
bundle's `Contents/Resources` beside `nav/`. The app installs each artifact
into `~/.274bot/map-cache` only when its identity matches the bound client
cache exactly; otherwise the catalogue is derived locally and the local
terrain bake stays behind the operator's consent.

Example, after recording the build receipt:

```sh
python3 tools/release/package.py --platform macos \
  --input target/release --output "$WORK/274bot-$VERSION-macos-arm64" \
  --build-receipt "$WORK/macos-build.json" \
  --app-profile rs2b2t --sign-identity "$RELEASE_SIGN_IDENTITY" \
  --map-cache "$ENGINE/data/pack/client" --map-unpack "$SNAPSHOTS"
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

Distributed packages target `rs2b2t`. Build their navigation with a
separate engine-input directory containing the public endpoint's CRC-verified
cache under data/pack/client, and `BOT_NAV_CONTENT_DIR` pointing at canonical
289 content. Do not overwrite the local engine cache. Both the public and the
local 289 cache identities are recognized. Local servers with another cache
need their own matching navigation build or explicit external navigation
resources.

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
