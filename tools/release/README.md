# Release packaging

Alpha 3 uses host workspace version 0.1.8. The public name comes from
`crates/panel/src/build_info.rs` `RELEASE`. Initial targets:

- macOS ARM64: Developer ID signed `274bot.app`, `panel-play`, `tui-play`.
- Windows x64: `panel-play.exe`, `tui-play.exe`.
- Linux x64: `tui-play`.

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

Example, after recording the build receipt:

```sh
python3 tools/release/package.py --platform macos \
  --input target/release --output .superpowers/release/274bot-0.1.7-macos-arm64 \
  --build-receipt .superpowers/release/macos-build.json \
  --app-profile public-289 --sign-identity YOUR_DEVELOPER_ID_IDENTITY_SHA1
```

The app's Finder launch selects public-289 through its Info.plist environment.
The standalone executables retain their normal CLI profile selection. The
bundle has its own navigation resources under Contents/Resources; standalone
binaries use the adjacent nav directory. Keep each layout intact.

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
