# Release packaging

Alpha 3 uses host workspace version 0.1.8. The public name comes from
`crates/panel/src/build_info.rs` `RELEASE`. Initial targets:

- macOS ARM64: Developer ID signed `274bot.app`, `panel-play`, `tui-play`.
- Windows x64: `panel-play.exe`, `tui-play.exe`.
- Linux x64: `panel-play`, `tui-play` (the panel from 0.1.9; see
  [Linux panel runtime](#linux-panel-runtime)).

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

Revision 289 packages also ship the WalkTo map terrain (operator decision
2026-09-26). After staging the binaries, `package.py` runs the staged
`tui-play --map-bundle` against the pinned client cache the nav bundle was
built from: `--map-cache` (its jag directory) and `--map-unpack` (the snapshot
root holding its decoded snapshot), by default the nav build's own resolution
(`BOT_NAV_ENGINE_DIR`, else `ENGINE_DIR`, else the canonical engine, plus
`/data/pack/client`; `BOT_NAV_SNAPSHOT_ROOT`, else `~/.274bot/unpack-289`).
The bake runs the production map-cache path (the same producer, writer and
publication as a local bake) in a scratch cache and ships the published
directory as `map/289/images/<key>/` (image `manifest.json` plus terrain
PNGs), with `map/289/274bot.mapimages.json` recording the image identity
(revision, decoded client content, bake policy), key, the manifest's size and
SHA-256 and the tile totals. `package.py` refuses terrain whose content
identity differs from `274bot.navpack.json`'s `content_id`, re-verifies every
shipped file against those receipts, records the description as
`map_images` in `release-manifest.json` (whose `files` list, and so the
archive `SHA256SUMS`, covers every tile), and copies `map/` into the macOS
bundle's `Contents/Resources` beside `nav/`. The 289 terrain is 1,702 tiles,
about 19 MB on disk, and the bake step takes about 25 s. The app installs it into
`~/.274bot/map-cache` only when the identity matches the bound client cache
exactly; otherwise the local bake stays behind the operator's consent.

Example, after recording the build receipt:

```sh
python3 tools/release/package.py --platform macos \
  --input target/release --output .superpowers/release/274bot-0.1.8-macos-arm64 \
  --build-receipt .superpowers/release/macos-build.json \
  --app-profile public-289 --sign-identity YOUR_DEVELOPER_ID_IDENTITY_SHA1 \
  --map-cache "$ENGINE/data/pack/client" --map-unpack "$SNAPSHOTS"
```

`--check` (no `--output`, no signing) stages the same package into a
temporary directory, verifies every staged file against its release manifest,
prints a JSON summary (file count, bytes, shipped map terrain) and removes it.

The app's Finder launch selects public-289 through its Info.plist environment.
The standalone executables retain their normal CLI profile selection. The
bundle has its own navigation and map resources under Contents/Resources;
standalone binaries use the adjacent nav and map directories. Keep each layout
intact.

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

Initial distributed packages target public-289. Build their navigation with a
separate engine-input directory containing the public endpoint's CRC-verified
cache under data/pack/client, and BOT_NAV_CONTENT_DIR pointing at canonical
289 content. Do not overwrite the local engine cache. The public 289 archive
identity was checked on 2026-09-16 via HTTPS /crc and all eight archive CRCs;
only versionlist differs from the existing local 289 set. Both exact cache
identities remain recognized. Local servers with another cache require their
own matching navigation build or explicit external navigation resources.

## Linux panel runtime

The Linux package ships `panel-play` beside `tui-play` with the same adjacent
`nav/289/` and `map/289/` resources as the Windows package; the panel's fonts
are compiled in. Build both binaries in the release build (`cargo build
--locked --release`). They are built on Ubuntu 24.04 and need glibc 2.39 or
newer.

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

Checked 2026-09-27 on the Ubuntu 24.04 builder: `package.py --check
--platform linux` staged 1,715 files (both binaries, `nav/289`, `map/289`,
docs), and the staged `panel-play` ran on Xvfb with Mesa lavapipe
(`adapter name=llvmpipe … backend=Vulkan`), logged a throwaway local account
in, and opened WalkTo on the shipped terrain (installed into the scratch
`~/.274bot/map-cache` byte for byte) without a bake prompt.
