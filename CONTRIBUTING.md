# Contributing to 274bot

> **Audience:** anyone who wants to fix a bug, add a feature or improve a doc.
> When you finish this page you can build 274bot, run the checks CI runs, and
> send a pull request.

274bot is a Rust bot host. It is in beta, and script compatibility is partial:
see the [CHANGELOG](CHANGELOG.md) for what works. The repository ships no game
assets. You bring your own local Lost City engine to run it.

## What you can contribute

- Bug reports, with the steps that show the bug and the version you ran.
- Fixes and small features.
- Tests, including live tests against a local engine.
- Documentation. Read [Writing 274bot docs](docs/dev/writing-docs.md) first.

Discuss a change in a [GitHub issue](https://github.com/acfrazier/274bot/issues)
before you write a large one. Open pull requests against `dev`. Read and run
your change before you send it: a pull request that looks like unchecked
generated output is closed. Do not describe script compatibility as complete in
a pull request or a doc.

## Build and run

1. Install [Rust through rustup](https://rustup.rs). The repository pins Rust
   **1.98.1** in `rust-toolchain.toml`, and rustup installs it for you.
2. On Linux, install the audio and window headers CI uses:

   ```bash
   sudo apt-get install pkg-config libasound2-dev libwayland-dev \
     libxkbcommon-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev
   ```

3. Clone with the client submodule:

   ```bash
   git clone --recurse-submodules https://github.com/acfrazier/274bot.git
   cd 274bot
   ```

4. Point `ENGINE_DIR` at your local engine and run the panel:

   ```bash
   export ENGINE_DIR=/absolute/path/to/engine
   cargo run --release -p panel --bin panel-play -- --profile local-289
   ```

Always use `--release`: a debug build looks frozen. Engine setup, navigation
data, the other programs and the live-test commands are in
[Build from source](docs/dev/build-from-source.md).

## Run the checks

Run these before you push. CI runs the same commands on every pull request. The
client submodule has its own manifest, so the format, lint and test commands
run twice.

| Check | What it catches | Command |
| --- | --- | --- |
| Format | Code that `rustfmt` would change | `cargo fmt --all -- --check` |
| Format (client) | The same, in the client | `cargo fmt --all --manifest-path vendor/fr-client-rust/Cargo.toml -- --check` |
| Lint suppressions | More `#[allow]` or `#[expect]` than the saved count | `python3 -B tools/lint-ratchet/lint_ratchet.py` |
| Clippy | Warnings; any warning fails | `cargo clippy --workspace --all-targets --no-deps -- -D warnings` |
| Clippy (client) | The same, in the client | `cargo clippy --manifest-path vendor/fr-client-rust/Cargo.toml --workspace --all-targets --no-deps -- -D warnings` |
| Tests | Failing tests | `cargo test --workspace` |
| Tests (client) | The same, in the client | `cargo test --manifest-path vendor/fr-client-rust/Cargo.toml --workspace` |
| Architecture | A crate dependency the rules do not allow | `tools/architecture/run.sh` (on `dev`; for a release branch see below) |

CI runs with `BOT_NAV_BUILD=skip` and `SKIP_GPU=1`: no navigation pack is
bundled into the build, and tests skip the graphics adapter. Set the same on a
machine that has no engine or GPU.

`tools/architecture/run.sh` uses the same checker rules as CI on `dev`, where
pull requests go. A `release/<version>` branch is checked against its own
`release-<version>` rules instead; to reproduce that,
[Architecture check in detail](docs/dev/architecture-check.md) shows how to run
`run.sh` against a local copy of the rules at that branch.

Some tests need a local file, such as a navigation pack. On GitHub they skip and
print the reason. On your machine they fail and name the environment variable
to set.

### What else CI runs

- **Clippy and test lanes for features.** CI also lints every feature build the
  Clippy job in [`.github/workflows/ci.yml`](.github/workflows/ci.yml) lists, and
  tests these. Run the lane for the crate you changed:

  ```bash
  cargo test -p script --features load --tests
  cargo test -p script --no-default-features
  cargo test -p host-play --features live-harness,test-support --test grinch_live
  cargo test -p host-play --features memory-profile
  cargo test -p host-play --features memory-profile-no-alloc --lib
  cargo test -p host --features performance-profile
  cargo test -p panel --features memory-profile
  cargo test -p panel --features render-diagnostics --lib
  cargo test -p tui --features memory-profile
  ```

  On Linux the panel also has a test that needs a virtual display (install
  `xvfb`):
  `xvfb-run -a cargo test -p panel --features memory-profile --test ime_frame`.
- **FlatBuffer bindings.** If you change `crates/script/schema/isolate.fbs`,
  regenerate the bindings with the `flatc` version CI pins (see `ci.yml`):
  `flatc --rust -o crates/script/schema/generated crates/script/schema/isolate.fbs`.
- **Architecture check.** `tools/architecture/run.sh` needs Python 3.11 or newer
  and a pinned `rust-analyzer` on your `PATH`. A new crate dependency, feature
  link or checker change needs a change in the separate `274bot-arch` repository,
  which a maintainer makes. Details are in
  [Architecture check in detail](docs/dev/architecture-check.md); the crate
  layout is in [Architecture](docs/architecture.md).

CI never sets `LIVE=1`. A green CI run does not show that a bot works in the
game: run the live tests for any change that affects behavior in the game.

## Live-test rules

Live tests run real clients against a local engine. They are marked `#[ignore]`
and run only with `LIVE=1`:

```bash
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
HOME="$(mktemp -d)" LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
HOME="$(mktemp -d)" LIVE=1 cargo test -p host-play -- --ignored --test-threads=1
```

The rules for writing one:

1. **Use real game content.** Never spawn the thing the code under test acts
   on: the tree, rock, fishing spot, NPC or door. Move the account next to a
   real one with `::tele` and let the bot do the real work.
2. **Seed only the account's own state:** inventory, bank, stats, position and
   quest progress. Seed a drop you need; never wait for a chance drop or event.
3. **Set a fixed deadline** for every test. A test that runs past it fails.
4. **Use disposable accounts on a local engine,** never a public account.
5. **Use a throwaway `HOME`** (as above), so a test never touches your own vault
   or profiles.

A failing live test prints `FAIL:` and exits with status 1. The harness, the
suite runner and the other live commands are in
[Build from source](docs/dev/build-from-source.md#live-tests-and-suite-runner)
and the [live harness guide](docs/harness.md).

## The client submodule

`vendor/fr-client-rust` is the game client. It is the
[FR-client-bothost](https://github.com/acfrazier/FR-client-bothost) repository,
branch `multirev-bh-modular`, checked out as a Git submodule. If it is empty,
run `git submodule update --init`.

- To change the client, change that repository, then commit the new submodule
  pointer in a 274bot pull request.
- Do not push to `Fairy-Ring/FR-client-rust`.
- Do not add a bot action API inside the client, and do not put 274bot crates in
  the client repository. Packet timing and `doAction` stay as the Java client has
  them.
- The client has its own `README.md` and `NOTICE.md`.

The Rust version is pinned in `rust-toolchain.toml` here and in the client, and
in the CI workflows. Change them together in one pull request; do not run
`rustup update` partway through a release.

## Code style

- **Typed errors.** A function that other crates call returns a typed error
  (a `thiserror` enum), not a `String`.
- **No `unwrap` in library code.** Tests may use it. Use `.expect("why")` only
  for a broken invariant.
- **`#[expect]`, not `#[allow]`.** Write `#[expect(lint, reason = "why")]`. The
  lint ratchet check fails if the number of suppressions grows.
- **`host_log!`, not `println!` or `eprintln!`,** in product code. It adds the
  category, level and bot, and hides passwords. Command-line output meant for
  a person may print.
- **One wire between JavaScript and Rust:** FlatBuffers. Do not add JSON.
- **`unsafe`** is only for the V8 boundary and operating-system calls. Give each
  block a `// SAFETY:` comment.

## Write the CHANGELOG

Add an entry under `## Unreleased` in [CHANGELOG.md](CHANGELOG.md), in the
section for the area you changed. Write what changed for the person who runs
the app: what they see and what they do differently. Do not name crates, types,
tests or commits. [Writing 274bot docs](docs/dev/writing-docs.md#changelog-entries)
has the rules.

The "Known issues" and "Not in this release" lists are generated. Add or change
an item in [`docs/known-issues.toml`](docs/known-issues.toml), then preview the
lists with `python3 tools/known-issues/known_issues.py render --release <next>`,
where `<next>` is the release being developed. The `check` command compares the
lists with a release section of the CHANGELOG, so it only works once that
section exists, when a release is cut.
Never edit those lists by hand.

## More developer pages

- [Build from source](docs/dev/build-from-source.md)
- [Generate the selected game data](docs/dev/selected-data-generation.md)
- [Architecture check in detail](docs/dev/architecture-check.md)
- [Architecture](docs/architecture.md) and [CONTEXT.md](CONTEXT.md), the terms
  the code uses
- [Docs index](docs/README.md)

## License

MIT ([LICENSE](LICENSE)). Attribution is in [NOTICE.md](NOTICE.md).
