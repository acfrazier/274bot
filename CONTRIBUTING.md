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

### Before you open a PR

Run the full CI check set:

```sh
cargo run --locked -p xtask -- ci --all
```

This command runs the formatting checks, lint checks, architecture check,
FlatBuffer check, all Clippy commands, and every test lane from GitHub CI.
It writes one log file per step and a `summary.txt` file to
`target/xtask-ci/`.

Use `cargo run --locked -p xtask -- ci --list` to print the step names. Run selected steps with
`cargo run --locked -p xtask -- ci <step>...`. Use `--log-dir <path>` to choose another log
directory.
You may add a personal Cargo alias in your user-level Cargo config; keep it
outside the repository.

The `xtask` package is a workspace member. The workspace `default-members`
list excludes it, so plain `cargo test` and `cargo clippy` do not build it.
Commands that use `--workspace`, including CI, include `xtask`.

### Requirements

- Install Rust 1.98.1 through rustup. Include the `rustfmt` and `clippy`
  components. Clone the repository with its client submodule.
- Run the full checks on Linux x86_64. The current pinned `rust-analyzer` and
  `flatc` builds support that platform. `xtask` downloads them with `curl` and
  checks their hashes in `~/.cache/274bot-ci`.
- Install Python 3, `git`, `curl`, `sha256sum`, `gzip`, `unzip` and `diff`.
  Linux test lanes also need the ALSA and window headers used by CI. The panel
  memory lane needs `xvfb`.
- Local test lanes need the same 289 navigation pack, decoded snapshots and
  other local test assets that the tests name. Set `NAV_PACK`,
  `WORLD_NAV_PACK`, `NAV_ARRIVAL_PACK`, `BOT_NAV_SNAPSHOT_ROOT`, `ENGINE_DIR`
  or `T5W_INSPECT_CAPTURE_BASELINE` when the test error names them.
- The architecture check uses the `main` branch of `274bot-arch`. Set
  `ARCH_REF` to check another branch or commit. Set `ARCH_REPO` to use a local
  checkout. `ARCH_RUST_ANALYZER_URL` can point to a mirror of the pinned tool.

On GitHub, tests skip a missing local asset only when
`GITHUB_ACTIONS=true`. A local run fails and names the missing asset.

The full check runs every CI lane in sequence. Allow several hours for a
cold build. Warm Cargo and pinned-tool caches reduce the time.

CI never sets `LIVE=1`. A green CI run does not show that a bot works in the
game. Run the live tests for any change that affects behavior in the game.

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
and the [live harness guide](docs/dev/harness.md).

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
- [Architecture](docs/dev/architecture.md) and [CONTEXT.md](CONTEXT.md), the terms
  the code uses
- [Docs index](docs/README.md)

## License

MIT ([LICENSE](LICENSE)). Attribution is in [NOTICE.md](NOTICE.md).
