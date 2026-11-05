use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

const RA_URL: &str = "https://github.com/rust-lang/rust-analyzer/releases/download/2026-10-12/rust-analyzer-x86_64-unknown-linux-gnu.gz";
const RA_ARCHIVE_SHA256: &str = "040cab4e2abc490fadd95c3f6cbc60b941a78761dbc3acf0ec66fd7a97f31db3";
const RA_BINARY_SHA256: &str = "fd89304c85f6c1996b036ea8f2c9ad9d3e25a197dbd713661c7fe51ec9dc8e49";
const RA_VERSION: &str = "rust-analyzer 0.3.3081-standalone";
const FLATC_URL: &str = "https://github.com/google/flatbuffers/releases/download/v25.12.19/Linux.flatc.binary.g%2B%2B-13.zip";
const FLATC_ARCHIVE_SHA256: &str =
    "9f87066dc5dfa7fe02090b55bab5f3e55df03e32c9b0cdf229004ade7d091039";
const ARCH_BRANCH: &str = "main";

const CLIPPY_COMMANDS: &[(&str, &str)] = &[
    (
        "clippy-1",
        "cargo clippy --workspace --all-targets --no-deps -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-2",
        "cargo clippy --manifest-path vendor/fr-client-rust/Cargo.toml --workspace --all-targets --no-deps -- -D warnings",
    ),
    (
        "clippy-3",
        "cargo clippy -p host --all-targets --no-deps --features performance-profile -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-4",
        "cargo clippy -p host --all-targets --no-deps --features render-diagnostics -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-5",
        "cargo clippy -p panel --all-targets --no-deps --features memory-profile -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-6",
        "cargo clippy -p panel --all-targets --no-deps --features render-diagnostics -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-7",
        "cargo clippy -p host-play --all-targets --no-deps --features memory-profile -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-8",
        "cargo clippy -p host-play --all-targets --no-deps --features memory-profile-no-alloc -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-9",
        "cargo clippy -p tui --all-targets --no-deps --features memory-profile -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-10",
        "cargo clippy -p script --no-default-features --all-targets --no-deps -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-11",
        "cargo clippy -p host-play --no-default-features --all-targets --no-deps -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
    (
        "clippy-12",
        "cargo clippy --manifest-path vendor/fr-client-rust/Cargo.toml --workspace --all-targets --no-deps --features render-test-hooks -- -D warnings",
    ),
    (
        "clippy-13",
        "cargo clippy -p host --all-targets --no-deps --features render-test-hooks -- -D warnings -D clippy::todo -D clippy::unimplemented",
    ),
];

const TEST_LANES: &[(&str, &str)] = &[
    ("workspace", "cargo test --workspace"),
    (
        "vendor",
        "cargo test --manifest-path vendor/fr-client-rust/Cargo.toml --workspace",
    ),
    ("script-load", "cargo test -p script-js --tests"),
    (
        "script-no-default",
        "cargo test -p script --no-default-features",
    ),
    (
        "native-only",
        "cargo build -p panel --no-default-features && cargo build -p tui --no-default-features && cargo build -p host-play --no-default-features && cargo test -p host-play --no-default-features --lib && python3 -B tools/architecture/native_dependencies.py --self-test && python3 -B tools/architecture/native_dependencies.py",
    ),
    (
        "grinch-oracles",
        "cargo test -p host-play --features live-harness,test-support --test grinch_live",
    ),
    (
        "host-play-memory",
        "cargo test -p host-play --features memory-profile",
    ),
    (
        "host-play-memory-no-alloc",
        "cargo test -p host-play --features memory-profile-no-alloc --lib",
    ),
    (
        "host-performance",
        "cargo test -p host --features performance-profile",
    ),
    (
        "panel-memory",
        "cargo test -p panel --features memory-profile && xvfb-run -a cargo test -p panel --features memory-profile --test ime_frame",
    ),
    (
        "panel-render-diagnostics",
        "cargo test -p panel --features render-diagnostics --lib",
    ),
    ("tui-memory", "cargo test -p tui --features memory-profile"),
    (
        "host-play-memory-26.04",
        "cargo test -p host-play --features memory-profile",
    ),
    (
        "vendor-render-test-hooks",
        "cargo test --manifest-path vendor/fr-client-rust/Cargo.toml --workspace --features render-test-hooks",
    ),
    (
        "host-render-test-hooks",
        "cargo test -p host --features render-test-hooks",
    ),
];

#[derive(Clone, Copy)]
enum StepKind {
    Shell(&'static str),
    ArchitectureSelfTest,
    ArchitectureCheck,
    FlatBuffers,
    SelfCheck,
}

struct Step {
    name: String,
    kind: StepKind,
}

struct Options {
    log_dir: PathBuf,
    cache_dir: PathBuf,
    arch_ref: Option<String>,
    arch_repo: Option<PathBuf>,
    list: bool,
    all: bool,
    selected: Vec<String>,
}

struct RunStep {
    name: String,
    commands: Vec<String>,
}

pub fn run(args: Vec<String>) -> Result<u8, String> {
    let root = workspace_root();
    let mut options = parse_options(args, &root)?;
    let steps = steps();

    if options.list {
        for step in &steps {
            println!("{}", step.name);
        }
        return Ok(0);
    }

    if options.all {
        options.selected = steps.iter().map(|step| step.name.clone()).collect();
    }

    let mut selected_steps = Vec::with_capacity(options.selected.len());
    let mut seen = HashSet::new();
    for name in &options.selected {
        if !seen.insert(name.as_str()) {
            return Err(format!("step `{name}` was selected more than once"));
        }
        let step = steps
            .iter()
            .find(|step| step.name == *name)
            .ok_or_else(|| format!("unknown CI step `{name}`; use `--list` to list valid steps"))?;
        selected_steps.push(step);
    }

    fs::create_dir_all(&options.log_dir).map_err(|error| {
        format!(
            "cannot create log directory {}: {error}",
            options.log_dir.display()
        )
    })?;
    let summary_path = options.log_dir.join("summary.txt");
    let mut summary = File::create(&summary_path)
        .map_err(|error| format!("cannot create {}: {error}", summary_path.display()))?;
    let mut failed = false;

    for step in selected_steps {
        let log_path = options.log_dir.join(format!("{}.log", step.name));
        let code = match File::create(&log_path) {
            Ok(mut log) => {
                let code = match execute_step(step, &root, &options, &mut log) {
                    Ok(code) => code,
                    Err(error) => {
                        let _ = writeln!(log, "xtask error: {error}");
                        1
                    }
                };
                let _ = writeln!(log, "exit={code}");
                code
            }
            Err(error) => {
                eprintln!("xtask: cannot create {}: {error}", log_path.display());
                1
            }
        };
        failed |= code != 0;
        let line = format!("{} exit={code}", step.name);
        writeln!(summary, "{line}")
            .map_err(|error| format!("cannot write {}: {error}", summary_path.display()))?;
        println!("{line}");
    }

    summary
        .flush()
        .map_err(|error| format!("cannot flush {}: {error}", summary_path.display()))?;
    Ok(u8::from(failed))
}

fn parse_options(args: Vec<String>, root: &Path) -> Result<Options, String> {
    let mut list = false;
    let mut all = false;
    let mut selected = Vec::new();
    let mut log_dir = None;
    let mut cache_dir = None;
    let mut arch_ref = env::var("ARCH_REF")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let mut arch_repo = env::var_os("ARCH_REPO")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--list" => list = true,
            "--all" => all = true,
            "--log-dir" | "--cache-dir" | "--arch-ref" | "--arch-repo" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                match arg.as_str() {
                    "--log-dir" => log_dir = Some(PathBuf::from(value)),
                    "--cache-dir" => cache_dir = Some(PathBuf::from(value)),
                    "--arch-ref" => {
                        arch_ref = (!value.trim().is_empty()).then(|| value.clone());
                    }
                    "--arch-repo" => {
                        arch_repo = (!value.is_empty()).then(|| PathBuf::from(value));
                    }
                    _ => return Err(format!("unexpected option `{arg}`")),
                }
            }
            _ if arg.starts_with("--log-dir=") => {
                log_dir = Some(PathBuf::from(&arg["--log-dir=".len()..]));
            }
            _ if arg.starts_with("--cache-dir=") => {
                cache_dir = Some(PathBuf::from(&arg["--cache-dir=".len()..]));
            }
            _ if arg.starts_with("--arch-ref=") => {
                let value = &arg["--arch-ref=".len()..];
                arch_ref = (!value.trim().is_empty()).then(|| value.to_owned());
            }
            _ if arg.starts_with("--arch-repo=") => {
                let value = &arg["--arch-repo=".len()..];
                arch_repo = (!value.is_empty()).then(|| PathBuf::from(value));
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option `{arg}`")),
            _ => selected.push(arg.clone()),
        }
        index += 1;
    }

    if list && (all || !selected.is_empty()) {
        return Err("--list cannot be combined with --all or step names".to_owned());
    }
    if all && !selected.is_empty() {
        return Err("--all cannot be combined with step names".to_owned());
    }
    if !list && !all && selected.is_empty() {
        return Err("select --all or one or more step names".to_owned());
    }

    let log_dir = resolve_path(
        root,
        log_dir.unwrap_or_else(|| PathBuf::from("target/xtask-ci")),
    );
    let cache_dir = match cache_dir {
        Some(path) => resolve_path(root, path),
        None => env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".cache/274bot-ci"))
            .unwrap_or_else(|| root.join("target/xtask-ci/cache")),
    };

    Ok(Options {
        log_dir,
        cache_dir,
        arch_ref,
        arch_repo: arch_repo.map(|path| resolve_path(root, path)),
        list,
        all,
        selected,
    })
}

fn resolve_path(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn steps() -> Vec<Step> {
    let mut result = vec![
        shell("fmt", "cargo fmt --all -- --check"),
        shell(
            "fmt-client",
            "cargo fmt --all --manifest-path vendor/fr-client-rust/Cargo.toml -- --check",
        ),
        shell(
            "ratchet-selftest",
            "python3 -B tools/lint-ratchet/lint_ratchet.py --self-test",
        ),
        shell("ratchet", "python3 -B tools/lint-ratchet/lint_ratchet.py"),
        shell(
            "nav-clippy",
            "python3 -B tools/architecture/nav_clippy.py --check",
        ),
        shell(
            "docs-check-selftest",
            "python3 -B tools/docs-check/test_docs_check.py",
        ),
        shell("docs-check", "python3 -B tools/docs-check/docs_check.py"),
        Step {
            name: "arch-selftest".to_owned(),
            kind: StepKind::ArchitectureSelfTest,
        },
        Step {
            name: "arch".to_owned(),
            kind: StepKind::ArchitectureCheck,
        },
        Step {
            name: "flatbuffers".to_owned(),
            kind: StepKind::FlatBuffers,
        },
    ];

    result.extend(
        CLIPPY_COMMANDS
            .iter()
            .map(|(name, command)| shell(name, command)),
    );
    result.extend(
        TEST_LANES
            .iter()
            .map(|(lane, command)| shell(&format!("test-{lane}"), command)),
    );
    result.push(Step {
        name: "ci-self-check".to_owned(),
        kind: StepKind::SelfCheck,
    });
    result
}

fn shell(name: &str, command: &'static str) -> Step {
    Step {
        name: name.to_owned(),
        kind: StepKind::Shell(command),
    }
}

fn execute_step(
    step: &Step,
    root: &Path,
    options: &Options,
    log: &mut File,
) -> Result<i32, String> {
    match step.kind {
        StepKind::Shell(command) => run_shell_step(step, command, root, log),
        StepKind::ArchitectureSelfTest => architecture_step(step, root, options, log, true),
        StepKind::ArchitectureCheck => architecture_step(step, root, options, log, false),
        StepKind::FlatBuffers => flatbuffers_step(step, root, options, log),
        StepKind::SelfCheck => self_check_step(root, log),
    }
}

fn run_shell_step(step: &Step, command: &str, root: &Path, log: &mut File) -> Result<i32, String> {
    if step.name.starts_with("test-") {
        return run_test_step(step, command, root, log);
    }
    writeln!(log, "== {}: {command}", step.name).map_err(io_error)?;
    let mut process = Command::new("bash");
    process.arg("-c").arg(command).current_dir(root);
    configure_common_environment(&mut process, root);
    run_logged(process, log)
}

fn run_test_step(step: &Step, command: &str, root: &Path, log: &mut File) -> Result<i32, String> {
    writeln!(log, "== {}: {command}", step.name).map_err(io_error)?;
    let mut process = Command::new("bash");
    process
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET");
    configure_common_environment(&mut process, root);

    let temp_home = if env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        None
    } else {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set for the local test lane".to_owned())?;
        let snapshot_root = env::var_os("BOT_NAV_SNAPSHOT_ROOT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".274bot/unpack-289"));
        let temp_home = root.join("target/xtask-ci/homes").join(format!(
            "{}-{}",
            std::process::id(),
            step_nonce()
        ));
        if let Err(error) = seed_test_home(&temp_home, &snapshot_root) {
            let _ = fs::remove_dir_all(&temp_home);
            return Err(error);
        }
        let cargo_home = env::var_os("CARGO_HOME")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| home.join(".cargo").into_os_string());
        let rustup_home = env::var_os("RUSTUP_HOME")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| home.join(".rustup").into_os_string());
        process.env("CARGO_HOME", cargo_home);
        process.env("RUSTUP_HOME", rustup_home);
        process.env("BOT_NAV_SNAPSHOT_ROOT", snapshot_root);
        process.env("HOME", &temp_home);
        Some(temp_home)
    };
    let result = run_logged(process, log);
    if let Some(home) = temp_home
        && let Err(error) = fs::remove_dir_all(&home)
    {
        return Err(format!(
            "cannot remove test HOME {}: {error}",
            home.display()
        ));
    }
    result
}

fn is_cargo_run_package_variable(name: &OsStr) -> bool {
    let name = name.as_encoded_bytes();
    name == b"CARGO_MANIFEST_DIR"
        || name == b"CARGO_MANIFEST_PATH"
        || name.starts_with(b"CARGO_PKG_")
        || name.starts_with(b"CARGO_CRATE_")
        || name.starts_with(b"CARGO_BIN_")
}

fn clear_cargo_run_package_environment(
    process: &mut Command,
    names: impl IntoIterator<Item = OsString>,
) {
    for name in names {
        if is_cargo_run_package_variable(&name) {
            process.env_remove(name);
        }
    }
}

fn configure_common_environment(process: &mut Command, root: &Path) {
    clear_cargo_run_package_environment(process, env::vars_os().map(|(name, _)| name));
    process
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .env("SKIP_GPU", "1")
        .env("BOT_NAV_BUILD", "skip");
    if env::var_os("CARGO_TERM_COLOR").is_none() {
        process.env("CARGO_TERM_COLOR", "never");
    }
    if env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        return;
    }

    let pack = env::var("NAV_PACK")
        .or_else(|_| env::var("WORLD_NAV_PACK"))
        .or_else(|_| env::var("NAV_ARRIVAL_PACK"))
        .ok()
        .or_else(|| {
            [
                root.join("target/local-data/nav/289/274bot.navpack"),
                PathBuf::from(
                    "/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v18/274bot-289-v18.navpack",
                ),
                PathBuf::from(
                    "/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack",
                ),
            ]
            .into_iter()
            .find(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned())
        });
    if let Some(pack) = pack {
        for name in ["NAV_PACK", "WORLD_NAV_PACK", "NAV_ARRIVAL_PACK"] {
            if env::var_os(name).is_none() {
                process.env(name, &pack);
            }
        }
    }
}

fn seed_test_home(home: &Path, snapshot_root: &Path) -> Result<(), String> {
    let destination = home.join(".274bot/unpack");
    fs::create_dir_all(&destination)
        .map_err(|error| format!("cannot create test HOME {}: {error}", home.display()))?;
    if !snapshot_root.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(snapshot_root).map_err(|error| {
        format!(
            "cannot read nav snapshot root {}: {error}",
            snapshot_root.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if is_snapshot_hash(&name_text) && entry.path().is_dir() {
            copy_directory(&entry.path(), &destination.join(name_text.as_ref()))?;
        }
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    let mut command = Command::new("cp");
    command.arg("-a");
    if env::consts::OS == "linux" {
        command.arg("--reflink=auto");
    }
    let output = command
        .arg(source)
        .arg(destination)
        .output()
        .map_err(|error| format!("cannot start cp for snapshot {}: {error}", source.display()))?;
    if !output.status.success() {
        return Err(format!(
            "cannot copy snapshot {}: {}",
            source.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn step_nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn is_snapshot_hash(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn run_logged(mut process: Command, log: &mut File) -> Result<i32, String> {
    let stdout = log.try_clone().map_err(io_error)?;
    let stderr = log.try_clone().map_err(io_error)?;
    process
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let status = process
        .status()
        .map_err(|error| format!("cannot start command: {error}"))?;
    Ok(exit_code(status))
}

fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn architecture_step(
    step: &Step,
    root: &Path,
    options: &Options,
    log: &mut File,
    self_test: bool,
) -> Result<i32, String> {
    writeln!(
        log,
        "== {}: python3 -B 274bot-arch/check.py {}",
        step.name,
        if self_test {
            "--self-test"
        } else {
            "--root <workspace>"
        }
    )
    .map_err(io_error)?;
    let ra = ensure_rust_analyzer(&options.cache_dir, log)?;
    let version = command_output(&ra, &["--version"], root, log)?;
    if version.trim() != RA_VERSION {
        return Err(format!(
            "pinned rust-analyzer version mismatch: expected `{RA_VERSION}`, found `{}`",
            version.trim()
        ));
    }
    writeln!(log, "rust-analyzer: {}", version.trim()).map_err(io_error)?;
    let checker = if let Some(path) = &options.arch_repo {
        path.clone()
    } else {
        clone_arch_checker(root, options, step, log)?
    };
    let check_path = checker.join("check.py");
    let mut command = Command::new("python3");
    command
        .arg("-B")
        .arg(check_path)
        .current_dir(root)
        .env("PATH", prepend_path(&options.cache_dir.join("bin"))?);
    if self_test {
        command.arg("--self-test");
    } else {
        command.arg("--root").arg(root);
    }
    let result = run_logged(command, log);
    if options.arch_repo.is_none() {
        let _ = fs::remove_dir_all(&checker);
    }
    result
}

fn clone_arch_checker(
    root: &Path,
    options: &Options,
    step: &Step,
    log: &mut File,
) -> Result<PathBuf, String> {
    let branch = options.arch_ref.as_deref().unwrap_or(ARCH_BRANCH);
    let check_root =
        root.join("target/xtask-ci/checkers")
            .join(format!("{}-{}", std::process::id(), step.name));
    fs::create_dir_all(
        check_root
            .parent()
            .ok_or_else(|| "invalid checker path".to_owned())?,
    )
    .map_err(io_error)?;
    let _ = fs::remove_dir_all(&check_root);
    writeln!(
        log,
        "git clone --depth 1 --branch {branch} https://github.com/acfrazier/274bot-arch.git {}",
        check_root.display()
    )
    .map_err(io_error)?;
    let shallow = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--branch")
        .arg(branch)
        .arg("https://github.com/acfrazier/274bot-arch.git")
        .arg(&check_root)
        .current_dir(root)
        .stdout(log.try_clone().map_err(io_error)?)
        .stderr(log.try_clone().map_err(io_error)?)
        .status()
        .map_err(|error| format!("cannot start git clone for 274bot-arch: {error}"))?;
    if !shallow.success() {
        let _ = fs::remove_dir_all(&check_root);
        let full = Command::new("git")
            .arg("clone")
            .arg("https://github.com/acfrazier/274bot-arch.git")
            .arg(&check_root)
            .current_dir(root)
            .stdout(log.try_clone().map_err(io_error)?)
            .stderr(log.try_clone().map_err(io_error)?)
            .status()
            .map_err(|error| format!("cannot start full git clone for 274bot-arch: {error}"))?;
        if !full.success() {
            return Err(format!("git clone of 274bot-arch branch `{branch}` failed"));
        }
        let checkout = Command::new("git")
            .args(["checkout", "--quiet", "--detach", branch])
            .current_dir(&check_root)
            .stdout(log.try_clone().map_err(io_error)?)
            .stderr(log.try_clone().map_err(io_error)?)
            .status()
            .map_err(|error| format!("cannot check out 274bot-arch ref `{branch}`: {error}"))?;
        if !checkout.success() {
            return Err(format!("git checkout of 274bot-arch ref `{branch}` failed"));
        }
    }
    writeln!(log, "arch checker ref: {branch}").map_err(io_error)?;
    Ok(check_root)
}

fn flatbuffers_step(
    step: &Step,
    root: &Path,
    options: &Options,
    log: &mut File,
) -> Result<i32, String> {
    writeln!(
        log,
        "== {}: flatc --rust and diff generated bindings",
        step.name
    )
    .map_err(io_error)?;
    let expected_version = flatc_expected_version(root)?;
    let flatc = ensure_flatc(&options.cache_dir, &expected_version, log)?;
    let schema_root = if root.join("crates/script-js/schema/isolate.fbs").is_file() {
        root.join("crates/script-js/schema")
    } else {
        root.join("crates/script/schema")
    };
    let schema = schema_root.join("isolate.fbs");
    if !schema.is_file() {
        return Err(format!(
            "FlatBuffer schema is missing: {}",
            schema.display()
        ));
    }
    let output = root.join("target/xtask-ci/generated").join(format!(
        "{}-{}",
        std::process::id(),
        step.name
    ));
    let _ = fs::remove_dir_all(&output);
    fs::create_dir_all(&output).map_err(io_error)?;
    let generation = Command::new(&flatc)
        .args(["--rust", "-o"])
        .arg(&output)
        .arg(&schema)
        .current_dir(root)
        .stdout(log.try_clone().map_err(io_error)?)
        .stderr(log.try_clone().map_err(io_error)?)
        .status()
        .map_err(|error| format!("cannot start pinned flatc: {error}"))?;
    let code = if generation.success() {
        let comparison = Command::new("diff")
            .arg("-ru")
            .arg(schema_root.join("generated"))
            .arg(&output)
            .current_dir(root)
            .stdout(log.try_clone().map_err(io_error)?)
            .stderr(log.try_clone().map_err(io_error)?)
            .status()
            .map_err(|error| format!("cannot start diff: {error}"))?;
        exit_code(comparison)
    } else {
        exit_code(generation)
    };
    let _ = fs::remove_dir_all(output);
    Ok(code)
}

fn self_check_step(root: &Path, log: &mut File) -> Result<i32, String> {
    writeln!(log, "== ci-self-check: validate .github/workflows/ci.yml").map_err(io_error)?;
    let path = root.join(".github/workflows/ci.yml");
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let problems = inspect_workflow(&contents);
    if problems.is_empty() {
        writeln!(
            log,
            "CI run steps call the locked xtask runner or use an approved hosted setup command."
        )
        .map_err(io_error)?;
        return Ok(0);
    }
    for problem in problems {
        writeln!(log, "{problem}").map_err(io_error)?;
    }
    Ok(1)
}

fn inspect_workflow(contents: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let matrix_lanes = parse_matrix_lanes(contents);
    let expected_lanes: HashSet<&str> = TEST_LANES.iter().map(|(lane, _)| *lane).collect();
    let actual_lanes: HashSet<&str> = matrix_lanes.iter().map(String::as_str).collect();
    if matrix_lanes.len() != actual_lanes.len() {
        problems.push("CI test matrix has a duplicate lane name".to_owned());
    }
    for lane in expected_lanes.difference(&actual_lanes) {
        problems.push(format!("CI test matrix is missing lane `{lane}`"));
    }
    for lane in actual_lanes.difference(&expected_lanes) {
        problems.push(format!("CI test matrix has unknown lane `{lane}`"));
    }

    let run_steps = parse_run_steps(contents);
    let step_specs = steps();
    let known: HashSet<&str> = step_specs.iter().map(|step| step.name.as_str()).collect();
    let test_template = "cargo run --locked -p xtask -- ci test-${{ matrix.lane }}";
    let mut called = HashSet::new();
    let mut has_test_template = false;

    for step in run_steps {
        if is_setup_only(&step.name) {
            continue;
        }
        if step.name == "Test ${{ matrix.lane }}" {
            if step.commands.len() == 1 && step.commands[0] == test_template {
                has_test_template = true;
            } else {
                problems.push(format!(
                    "{}: matrix test step must run `{test_template}`",
                    step.name
                ));
            }
            continue;
        }
        if step.commands.len() != 1 {
            problems.push(format!(
                "{}: run block must contain one locked xtask command",
                step.name
            ));
            continue;
        }
        let Some(name) = parse_xtask_call(&step.commands[0]) else {
            problems.push(format!(
                "{}: run command bypasses the xtask runner: {}",
                step.name,
                step.commands.join(" ; ")
            ));
            continue;
        };
        if !known.contains(name) {
            problems.push(format!(
                "{}: references unknown xtask CI step `{name}`",
                step.name
            ));
            continue;
        }
        if !called.insert(name.to_owned()) {
            problems.push(format!("CI calls step `{name}` more than once"));
        }
    }

    if !has_test_template {
        problems.push("CI has no xtask test-lane matrix call".to_owned());
    }
    for step in step_specs
        .iter()
        .filter(|step| !step.name.starts_with("test-"))
    {
        if !called.contains(step.name.as_str()) {
            problems.push(format!("CI does not call xtask CI step `{}`", step.name));
        }
    }
    problems
}

fn parse_run_steps(contents: &str) -> Vec<RunStep> {
    let lines: Vec<&str> = contents.lines().collect();
    let mut results = Vec::new();
    let mut current_name = None::<String>;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let indent = line.chars().take_while(|ch| *ch == ' ').count();
        let trimmed = line.trim();
        let run = if indent == 6 && trimmed.starts_with("- run:") {
            current_name = None;
            Some(("<unnamed run step>".to_owned(), trimmed[6..].trim()))
        } else if indent == 8 && trimmed.starts_with("run:") {
            Some((
                current_name
                    .take()
                    .unwrap_or_else(|| "<unnamed run step>".to_owned()),
                trimmed[4..].trim(),
            ))
        } else {
            None
        };

        if let Some((name, value)) = run {
            let mut commands = Vec::new();
            if matches!(value, "|" | ">" | "|-" | ">-") {
                index += 1;
                while index < lines.len() {
                    let body = lines[index];
                    let body_indent = body.chars().take_while(|ch| *ch == ' ').count();
                    if !body.trim().is_empty() && body_indent <= indent {
                        break;
                    }
                    let command = body.trim();
                    if !command.is_empty() && !command.starts_with('#') {
                        commands.push(command.to_owned());
                    }
                    index += 1;
                }
                results.push(RunStep { name, commands });
                continue;
            }
            commands.push(value.to_owned());
            results.push(RunStep { name, commands });
        } else if indent == 6 && trimmed.starts_with("- ") {
            current_name = trimmed
                .strip_prefix("- name: ")
                .map(|name| name.trim().to_owned());
        }
        index += 1;
    }
    results
}

fn parse_matrix_lanes(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter_map(|line| {
            let indent = line.chars().take_while(|ch| *ch == ' ').count();
            let trimmed = line.trim();
            (indent == 10 && trimmed.starts_with("- lane: "))
                .then(|| trimmed[8..].trim().to_owned())
        })
        .collect()
}

fn parse_xtask_call(command: &str) -> Option<&str> {
    let command = command.strip_prefix("cargo run --locked -p xtask -- ci ")?;
    if command.is_empty() || command.split_whitespace().count() != 1 {
        return None;
    }
    Some(command)
}

fn is_setup_only(name: &str) -> bool {
    matches!(
        name,
        "Native libs (ALSA + window)"
            | "Fetch pinned client submodule as data"
            | "Free runner disk"
            | "Fetch the 289 nav pack"
            | "Use the 289 nav pack"
            | "Require every test lane"
    )
}

fn ensure_rust_analyzer(cache: &Path, log: &mut File) -> Result<PathBuf, String> {
    ensure_linux_x86_64("rust-analyzer")?;
    let bin_dir = cache.join("bin");
    let download_dir = cache.join("downloads");
    fs::create_dir_all(&bin_dir).map_err(io_error)?;
    fs::create_dir_all(&download_dir).map_err(io_error)?;
    let binary = bin_dir.join("rust-analyzer");
    if binary.is_file() && file_sha256(&binary, log)? == RA_BINARY_SHA256 {
        return Ok(binary);
    }
    if binary.exists() {
        writeln!(
            log,
            "cached rust-analyzer digest did not match; replacing it"
        )
        .map_err(io_error)?;
        fs::remove_file(&binary).map_err(io_error)?;
    }

    let archive = download_dir.join("rust-analyzer.gz");
    if !archive.is_file() || file_sha256(&archive, log)? != RA_ARCHIVE_SHA256 {
        let url = env::var("ARCH_RUST_ANALYZER_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| RA_URL.to_owned());
        download(&url, &archive, log)?;
        require_digest(&archive, RA_ARCHIVE_SHA256, "rust-analyzer archive", log)?;
    }
    let decoded = bin_dir.join(format!("rust-analyzer-{}.tmp", std::process::id()));
    let status = Command::new("gzip")
        .arg("-dc")
        .arg(&archive)
        .stdout(File::create(&decoded).map_err(io_error)?)
        .stderr(log.try_clone().map_err(io_error)?)
        .status()
        .map_err(|error| format!("cannot start gzip to unpack pinned rust-analyzer: {error}"))?;
    if !status.success() {
        let _ = fs::remove_file(&decoded);
        return Err("gzip could not unpack the pinned rust-analyzer archive".to_owned());
    }
    require_digest(&decoded, RA_BINARY_SHA256, "rust-analyzer binary", log)?;
    set_executable(&decoded)?;
    fs::rename(&decoded, &binary).map_err(io_error)?;
    Ok(binary)
}

fn flatc_expected_version(root: &Path) -> Result<String, String> {
    let manifest_path = root.join("crates/script-js/Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    flatc_version_from_manifest(&manifest).map_err(|error| {
        format!(
            "cannot read flatc version from {}: {error}",
            manifest_path.display()
        )
    })
}

fn flatc_version_from_manifest(manifest: &str) -> Result<String, String> {
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_dependencies = line == "[dependencies]";
            continue;
        }
        if !in_dependencies {
            continue;
        }
        let Some((name, dependency)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != "flatbuffers" {
            continue;
        }
        let dependency = dependency.trim();
        let requirement = dependency
            .split_once("version")
            .and_then(|(_, field)| field.split_once('=').map(|(_, value)| value.trim()))
            .or_else(|| dependency.strip_prefix('"').map(|_| dependency))
            .and_then(|value| value.strip_prefix('"'))
            .and_then(|value| value.split_once('"').map(|(version, _)| version))
            .ok_or_else(|| "flatbuffers dependency has no readable version pin".to_owned())?;
        let version = requirement
            .strip_prefix('=')
            .filter(|version| {
                !version.is_empty()
                    && version
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte == b'.')
            })
            .ok_or_else(|| "flatbuffers dependency does not use an exact version pin".to_owned())?;
        return Ok(format!("flatc version {version}"));
    }
    Err("flatbuffers dependency is missing from [dependencies]".to_owned())
}

fn ensure_flatc(cache: &Path, expected_version: &str, log: &mut File) -> Result<PathBuf, String> {
    ensure_linux_x86_64("flatc")?;
    let bin_dir = cache.join("bin");
    let download_dir = cache.join("downloads");
    fs::create_dir_all(&bin_dir).map_err(io_error)?;
    fs::create_dir_all(&download_dir).map_err(io_error)?;
    let binary = bin_dir.join("flatc");
    let archive_pin = bin_dir.join("flatc.sha256");
    let binary_pin = bin_dir.join("flatc.binary.sha256");
    if binary.is_file()
        && fs::read_to_string(&archive_pin).is_ok_and(|value| value.trim() == FLATC_ARCHIVE_SHA256)
        && fs::read_to_string(&binary_pin)
            .ok()
            .map(|expected| file_sha256(&binary, log).map(|actual| actual == expected.trim()))
            .transpose()?
            .is_some_and(|matches| matches)
    {
        let version = command_output(&binary, &["--version"], Path::new("."), log)?;
        if version.trim() == expected_version {
            return Ok(binary);
        }
    }
    if binary.exists() {
        writeln!(
            log,
            "cached flatc did not match its pinned archive or version; replacing it"
        )
        .map_err(io_error)?;
        fs::remove_file(&binary).map_err(io_error)?;
    }
    let _ = fs::remove_file(&archive_pin);
    let _ = fs::remove_file(&binary_pin);

    let archive = download_dir.join("flatc-linux.zip");
    if !archive.is_file() || file_sha256(&archive, log)? != FLATC_ARCHIVE_SHA256 {
        download(FLATC_URL, &archive, log)?;
        require_digest(&archive, FLATC_ARCHIVE_SHA256, "flatc archive", log)?;
    }
    let extract = download_dir.join(format!("flatc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&extract);
    fs::create_dir_all(&extract).map_err(io_error)?;
    let status = Command::new("unzip")
        .arg("-q")
        .arg(&archive)
        .arg("-d")
        .arg(&extract)
        .stdout(log.try_clone().map_err(io_error)?)
        .stderr(log.try_clone().map_err(io_error)?)
        .status()
        .map_err(|error| format!("cannot start unzip for pinned flatc: {error}"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&extract);
        return Err("unzip could not unpack the pinned flatc archive".to_owned());
    }
    let unpacked = extract.join("flatc");
    let copied = bin_dir.join(format!("flatc-{}.tmp", std::process::id()));
    fs::copy(&unpacked, &copied)
        .map_err(|error| format!("flatc archive has no usable flatc binary: {error}"))?;
    set_executable(&copied)?;
    let version = command_output(&copied, &["--version"], Path::new("."), log)?;
    if version.trim() != expected_version {
        let _ = fs::remove_file(&copied);
        let _ = fs::remove_dir_all(&extract);
        return Err(format!(
            "pinned flatc version mismatch: expected `{expected_version}`, found `{}`",
            version.trim()
        ));
    }
    let binary_digest = file_sha256(&copied, log)?;
    fs::rename(&copied, &binary).map_err(io_error)?;
    fs::write(&archive_pin, format!("{FLATC_ARCHIVE_SHA256}\n")).map_err(io_error)?;
    fs::write(&binary_pin, format!("{binary_digest}\n")).map_err(io_error)?;
    let _ = fs::remove_dir_all(&extract);
    Ok(binary)
}

fn ensure_linux_x86_64(tool: &str) -> Result<(), String> {
    if env::consts::OS != "linux" || env::consts::ARCH != "x86_64" {
        return Err(format!(
            "the pinned {tool} build is available only for Linux x86_64; this host is {} {}",
            env::consts::OS,
            env::consts::ARCH
        ));
    }
    Ok(())
}

fn download(url: &str, destination: &Path, log: &mut File) -> Result<(), String> {
    let temporary = destination.with_extension(format!("download-{}", std::process::id()));
    let _ = fs::remove_file(&temporary);
    writeln!(
        log,
        "curl --fail --location --retry 3 --output {} {url}",
        temporary.display()
    )
    .map_err(io_error)?;
    let status = Command::new("curl")
        .args(["--fail", "--location", "--retry", "3", "--output"])
        .arg(&temporary)
        .arg(url)
        .stdout(log.try_clone().map_err(io_error)?)
        .stderr(log.try_clone().map_err(io_error)?)
        .status()
        .map_err(|error| format!("cannot start curl for pinned tool {url}: {error}"))?;
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(format!("curl could not download pinned tool from {url}"));
    }
    fs::rename(&temporary, destination).map_err(io_error)
}

fn file_sha256(path: &Path, log: &mut File) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(log.try_clone().map_err(io_error)?)
        .output()
        .map_err(|error| format!("cannot run sha256sum for {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!("sha256sum failed for {}", path.display()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| format!("sha256sum returned no digest for {}", path.display()))
}

fn require_digest(path: &Path, expected: &str, label: &str, log: &mut File) -> Result<(), String> {
    let actual = file_sha256(path, log)?;
    writeln!(log, "sha256 {label}: {actual}").map_err(io_error)?;
    if actual != expected {
        return Err(format!(
            "{label} SHA-256 mismatch: expected {expected}, found {actual}"
        ));
    }
    Ok(())
}

fn command_output(
    executable: &Path,
    args: &[&str],
    cwd: &Path,
    log: &mut File,
) -> Result<String, String> {
    let output = Command::new(executable)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(log.try_clone().map_err(io_error)?)
        .output()
        .map_err(|error| format!("cannot run {}: {error}", executable.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} exited with {}",
            executable.display(),
            exit_code(output.status)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).map_err(io_error)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).map_err(io_error)?;
    }
    Ok(())
}

fn prepend_path(bin_dir: &Path) -> Result<String, String> {
    let current = env::var_os("PATH").ok_or_else(|| "PATH is not set".to_owned())?;
    let mut entries = vec![bin_dir.to_path_buf()];
    entries.extend(env::split_paths(&current));
    let joined =
        env::join_paths(entries).map_err(|error| format!("cannot build tool PATH: {error}"))?;
    Ok(joined.to_string_lossy().into_owned())
}

fn io_error(error: io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci_step_list_contains_every_lane_and_clippy_command() {
        let names: Vec<String> = steps().into_iter().map(|step| step.name).collect();
        assert_eq!(CLIPPY_COMMANDS.len(), 13);
        assert_eq!(TEST_LANES.len(), 15);
        assert!(names.iter().any(|name| name == "clippy-13"));
        assert!(
            names
                .iter()
                .any(|name| name == "test-host-play-memory-26.04")
        );
        assert!(names.iter().any(|name| name == "ci-self-check"));
    }

    #[test]
    fn step_environment_clears_cargo_package_variables() {
        let names = [
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_PKG_VERSION",
            "CARGO_PKG_NAME",
            "CARGO_CRATE_NAME",
            "CARGO_BIN_NAME",
            "CARGO_BIN_EXE_xtask",
        ];
        let check = format!(
            "for name in {}; do if printenv \"$name\" >/dev/null; then exit 1; fi; done; test \"$CARGO_HOME\" = preserved",
            names.join(" ")
        );
        let mut process = Command::new("bash");
        process.arg("-c").arg(check).env("CARGO_HOME", "preserved");
        for name in names {
            process.env(name, "cargo-injected");
        }
        clear_cargo_run_package_environment(
            &mut process,
            names.iter().map(|name| OsString::from(*name)),
        );
        let output = process.output().expect("step environment probe starts");
        assert!(
            output.status.success(),
            "step inherited cargo package variables: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn flatc_version_comes_from_the_flatbuffers_manifest_pin() {
        let manifest = "[dependencies]\nflatbuffers = { version = \"=26.1.2\" }\n";
        assert_eq!(
            flatc_version_from_manifest(manifest).as_deref(),
            Ok("flatc version 26.1.2")
        );
    }

    #[test]
    fn flatc_version_parser_rejects_non_exact_pins() {
        let manifest = "[dependencies]\nflatbuffers = { version = \"25.12.19\" }\n";
        assert!(flatc_version_from_manifest(manifest).is_err());
    }

    #[test]
    fn checked_in_workflow_runs_every_registered_step() {
        let problems = inspect_workflow(include_str!("../../../.github/workflows/ci.yml"));
        assert!(problems.is_empty(), "workflow problems: {problems:?}");
    }

    #[test]
    fn workflow_parser_rejects_commands_that_bypass_xtask() {
        let yaml = "jobs:\n  fmt:\n    steps:\n      - name: Check formatting\n        run: cargo fmt --all -- --check\n";
        let problems = inspect_workflow(yaml);
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains("bypasses the xtask runner"))
        );
    }

    #[test]
    fn workflow_parser_accepts_a_named_setup_command() {
        let yaml = "jobs:\n  fmt:\n    steps:\n      - name: Native libs (ALSA + window)\n        run: sudo apt-get update\n";
        let runs = parse_run_steps(yaml);
        assert_eq!(runs.len(), 1);
        assert!(is_setup_only(&runs[0].name));
    }

    #[test]
    fn unnamed_run_step_does_not_inherit_a_setup_name() {
        let yaml = "jobs:\n  test:\n    steps:\n      - name: Fetch the 289 nav pack\n        uses: actions/cache@v4\n      - run: cargo test\n";
        let runs = parse_run_steps(yaml);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].name, "<unnamed run step>");
    }

    #[test]
    fn workflow_parser_reads_multiline_run_blocks() {
        let yaml = "jobs:\n  arch:\n    steps:\n      - name: Install deps\n        run: |\n          set -euo pipefail\n          sudo apt-get update\n";
        let runs = parse_run_steps(yaml);
        assert_eq!(
            runs[0].commands,
            ["set -euo pipefail", "sudo apt-get update"]
        );
    }
}
