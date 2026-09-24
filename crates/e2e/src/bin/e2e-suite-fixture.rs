//! `e2e-suite-fixture`: a disposable child that speaks the native terminal-receipt and
//! capture contracts without a client, engine, login or gameplay.
//!
//! It exists so the suite's offline verification can exercise real process behaviour —
//! budgets, timeouts, process-group cleanup, receipt identity checks, capture validation,
//! bounded logs — against a real child process. It is never used by a real suite run:
//! `run` launches the panel executables from the manifest (or the direct executables an
//! operator names with `--exec-core`/`--exec-pair`).
//!
//! It mimics the panel's real line contract: the outer proof name is the `--live` name
//! exactly as given (`script_thiever`), the evidence JSON names the *scenario*
//! (`thiever`), and the witness carries the host enum's serde wire form (`thiever`). It
//! also writes the terminal shot the scenario declares, exactly as the panel's
//! `hold_terminal_shot` path does before it announces a PASS.
//!
//! Modes:
//!   pass             PASS (+ the declared witness + terminal shot), exit 0
//!   fail             FAIL with a case message, exit 1
//!   zero-no-receipt  progress output only, exit 0
//!   slow-pass        like `pass` after `--sleep-ms`
//!   hang             never exits (budget/cleanup coverage); `--ignore-stop` to make the
//!                    suite escalate to its forced stop
//!   descendant       spawn a copy of itself (`--mode hang`) with its own stdio, record its
//!                    pid in `E2E_SUITE_DESCENDANT_PID`, exit 0: a descendant that outlives
//!                    the direct child and that nothing can see through the pipes
//!   descendant-pipe  the same, but the descendant inherits this process's stdio, so the
//!                    suite's pipes stay open after the direct child exits
//!   descendant-ignore-stop  the same with a descendant that ignores the graceful stop, so
//!                    only the suite's forced stop can end it
//!   escape-pipe      (unix) leave the suite's process group while holding the pipes
//!   malformed        PASS with a non-JSON payload
//!   duplicate        two PASS receipts
//!   dual-terminal    PASS and FAIL in one run
//!   wrong-witness    PASS plus a witness naming another case
//!   missing-witness  PASS with no witness at all
//!   no-shot          PASS with the terminal shot left unwritten
//!   wrong-label      write a capture under an unrelated label, then PASS
//!   broken-shots     write a PNG with a broken signature and a sidecar off scene 2
//!   infra            an infrastructure line, exit 1
//!   panic            a panic line, exit 101
//!
//! External loader smoke modes (the dedicated `EXTERNAL_LOADER` contract, driven by the
//! producer's own `host_play::external_loader::ExternalWatch` state machine so the printed
//! receipt is the real one):
//!   external-pass        qualified receipt + `EXTERNAL_LOADER` + `PASS` + the terminal capture
//!                        under the producer's own label, sidecar naming the actor, exit 0
//!   external-fail        the producer's failed receipt + `EXTERNAL_LOADER` + `FAIL`, exit 1
//!   external-no-witness  `PASS` carrying the external record with no `EXTERNAL_LOADER` line
//!   external-inconsistent  witness and `PASS` payload disagree
//!   external-incomplete  both lines carry a record that does not confirm the capture request
//!   external-prereq-only  only the prerequisite capture is written (a different label)
//!   external-no-capture  no capture at all
//!   external-bad-capture  a torn PNG with an off-scene sidecar under the terminal label
//!
//! The external modes hash the raw source they were given (the suite passes `--external-ts`),
//! refuse anything that is not the producer's frozen fixture, and write the reload identities
//! as the real digest of those bytes plus the producer's trailing newline. The 25-bone
//! prerequisite, the distinct-burial/XP/inventory gate and the Stop record are the producer's
//! own transitions, not synthetic counters.
//!
//! `--ignore-stop` is the platform's "do not die on a polite request": unix ignores
//! `SIGTERM`, Windows ignores the console `CTRL_C`/`CTRL_BREAK` events. The suite's forced
//! stop (journalled `SIGKILL`, Windows job termination) is what ends such a child.
//!
//! Environment: `E2E_SUITE_FIXTURE_LOG` appends one line per launch (resume/no-duplicate
//! launch coverage), `274BOT_SMOKE_DIR` is the capture root the suite points at the run,
//! `E2E_SUITE_DESCENDANT_PID` names the file a `descendant*` mode records its child's pid
//! in, and `E2E_SUITE_FIXTURE_CORE` / `E2E_SUITE_FIXTURE_PAIR` name the witness identity the
//! fixture should print — the stand-in for the identity a real adapter derives from its
//! own configuration, since the suite hands a real child only
//! `--profile ... --catalog ... --live <name>`.

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

/// A real 1x1 RGBA PNG. The suite decodes the capture for real; a signature alone is not
/// an image.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

#[derive(Debug)]
struct Args {
    mode: String,
    live: Option<String>,
    core: Option<String>,
    pair: Option<String>,
    shot: Option<String>,
    sleep_ms: u64,
    ignore_stop: bool,
    report_args: Option<PathBuf>,
    /// The raw TypeScript source the suite bound (absolute). Absent, the fixture reads the same
    /// tracked default fixture the panel does.
    external_ts: Option<PathBuf>,
}

impl Args {
    /// The `--live` name exactly as the suite passed it (`script_thiever`): the panel's
    /// proof name is `live.name`, not the scenario.
    fn live_name(&self) -> String {
        self.live
            .clone()
            .unwrap_or_else(|| "script_fixture".to_string())
    }

    /// The scenario the live name maps to (`thiever`).
    fn scenario(&self) -> String {
        let live = self.live_name();
        live.strip_prefix("script_").unwrap_or(&live).to_string()
    }
}

fn parse() -> Args {
    let mut args = Args {
        mode: "pass".into(),
        live: None,
        core: None,
        pair: None,
        shot: None,
        sleep_ms: 0,
        ignore_stop: false,
        report_args: None,
        external_ts: None,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        let take = |i: &mut usize| -> Option<String> {
            *i += 1;
            argv.get(*i).cloned()
        };
        match arg {
            "--mode" => args.mode = take(&mut i).unwrap_or_default(),
            "--core" => args.core = take(&mut i),
            "--pair" => args.pair = take(&mut i),
            "--shot" => args.shot = take(&mut i),
            "--sleep-ms" => {
                args.sleep_ms = take(&mut i).and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            // Both spellings name the same platform-specific action: ignore the graceful
            // stop so only the suite's forced stop can end this process.
            "--ignore-stop" | "--ignore-sigterm" => args.ignore_stop = true,
            "--report-args" => args.report_args = take(&mut i).map(PathBuf::from),
            // The typed source override the suite hands a real `external_watch`; the fixture
            // resolves it with the producer's own function, exactly like the panel does.
            "--external-ts" => args.external_ts = take(&mut i).map(PathBuf::from),
            // The suite hands the child the shared native profile flags; the fixture
            // records the ones it cares about and ignores the rest, exactly like a real
            // executable would resolve them from its own configuration.
            "--live" => args.live = take(&mut i),
            // An unrecognized long flag: skip its value too when one follows, so the next
            // token is still read as a flag.
            other
                if other.starts_with("--")
                    && argv.get(i + 1).is_some_and(|next| !next.starts_with("--")) =>
            {
                i += 1;
            }
            other if other.starts_with("--") => {}
            _ => {}
        }
        i += 1;
    }
    args
}

fn record_launch(args: &Args) {
    if let Ok(path) = std::env::var("E2E_SUITE_FIXTURE_LOG") {
        let line = format!("{} {}\n", args.scenario(), args.mode);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

fn capture_root() -> Option<PathBuf> {
    std::env::var_os(scenario_shot_env()).map(PathBuf::from)
}

fn scenario_shot_env() -> &'static str {
    "274BOT_SMOKE_DIR"
}

/// The label the panel would ask for: the scenario's own declared terminal shot.
fn declared_shot(scenario: &str) -> Option<String> {
    let scenario = scenario::get(scenario)?;
    scenario.settings.terminal_shot.map(str::to_string)
}

fn write_capture(label: &str, valid: bool) -> Option<PathBuf> {
    let root = capture_root()?;
    let dir = root.join(format!("fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let stem = format!("2026-09-14T00-00-00_{}", safe_label(label));
    let png = dir.join(format!("{stem}.png"));
    let bytes: Vec<u8> = if valid {
        TINY_PNG.to_vec()
    } else {
        b"not a png at all".to_vec()
    };
    std::fs::write(&png, bytes).ok()?;
    let sidecar = if valid {
        "{\"ingame\":true,\"scene_state\":2}"
    } else {
        // A torn capture: real PNG bytes are missing and the snapshot was taken off the
        // world, so it can never stand in for terminal evidence.
        "{\"ingame\":false,\"scene_state\":1}"
    };
    std::fs::write(dir.join(format!("{stem}.json")), sidecar).ok()?;
    Some(png)
}

fn safe_label(label: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            if pending {
                out.push('_');
                pending = false;
            }
            out.push(ch);
        } else {
            pending = true;
        }
    }
    if pending {
        out.push('_');
    }
    out.truncate(80);
    out
}

/// The witness the fixture prints. A real adapter derives the identity from its own
/// configuration; the offline fixture reads it from the environment, defaulting to the
/// scenario's own wire form for the core cases the fixture manifest declares.
fn witness(args: &Args) -> Option<(&'static str, String)> {
    if let Some(core) = &args.core {
        return Some(("CATALOG_CORE", core.clone()));
    }
    if let Some(pair) = &args.pair {
        return Some(("PAIRED_CORE", pair.clone()));
    }
    if let Ok(core) = std::env::var("E2E_SUITE_FIXTURE_CORE") {
        if !core.is_empty() {
            return Some(("CATALOG_CORE", core));
        }
    }
    if let Ok(pair) = std::env::var("E2E_SUITE_FIXTURE_PAIR") {
        if !pair.is_empty() {
            return Some(("PAIRED_CORE", pair));
        }
    }
    let scenario = args.scenario();
    host_play::catalog_core::CoreCase::parse(&scenario)
        .ok()
        .and_then(|case| serde_json::to_value(case).ok())
        .and_then(|value| value.as_str().map(str::to_string))
        .map(|wire| ("CATALOG_CORE", wire))
}

/// Drive the producer's own external loader state machine to its qualified record.
///
/// The fixture stands in for `external_watch`: it resolves the same raw source with the same
/// producer function, refuses anything but the frozen fixture, verifies and copies those bytes to
/// its own temporary path (never the input), and walks the real watch through its own transitions
/// — 25-bone prerequisite, load/select without Start, the burials/XP/inventory gate, Stop,
/// unchanged reload, then the harmless whitespace reload. The receipt it prints is the producer's
/// serialization, and the reload identities are the real digests of the bound bytes.
fn external_qualify(args: &Args) -> Result<(serde_json::Value, String, PathBuf), String> {
    use host_play::external_loader::{
        source_sha256, ExternalWatch, BONES_COUNT, FROZEN_SHA256, NOTHING_CHANGED, SCRIPT_NAME,
    };
    let source = host_play::external_loader::resolve_source(args.external_ts.as_deref())?;
    let bytes = std::fs::read(&source)
        .map_err(|error| format!("external loader source {}: {error}", source.display()))?;
    let sha256 = source_sha256(&bytes);
    if sha256 != FROZEN_SHA256 {
        return Err(format!(
            "external loader source sha {sha256} does not match frozen {FROZEN_SHA256}"
        ));
    }
    let mut whitespace = bytes.clone();
    whitespace.push(b'\n');
    let after = source_sha256(&whitespace);
    let owned = std::env::temp_dir().join(format!(
        "274bot-fixture-external-{}-ExampleBot.ts",
        std::process::id()
    ));
    std::fs::write(&owned, &bytes)
        .map_err(|error| format!("external loader owned copy {}: {error}", owned.display()))?;
    let identity = format!("file:{}", owned.display());
    let account =
        std::env::var("E2E_SUITE_FIXTURE_ACCOUNT").unwrap_or_else(|_| "fixture".to_string());
    let watch = ExternalWatch::default();
    let now = std::time::Instant::now();
    watch.configure(account.clone(), owned.clone(), sha256.clone());
    watch.note_scene(true, 2);
    watch.note_inventory(now, &account, BONES_COUNT, 0);
    watch.note_prereq_passed();
    watch.note_load(1, SCRIPT_NAME, &owned, &identity, &sha256, true, false);
    watch.begin_start(now)?;
    let burials: Vec<String> = (1..=19)
        .map(|i| format!("buried bones (#{i}, +{i} prayer xp total)"))
        .collect();
    watch.note_logs(now, &account, &burials);
    watch.note_inventory(now, &account, 12, 20);
    watch.request_stop(now);
    watch.note_logs(
        now,
        &account,
        &["BoneBurier stopped — 19 buried, +20 prayer xp".into()],
    );
    watch.note_stop(now, true, false);
    watch.note_reload_unchanged(NOTHING_CHANGED);
    watch.note_reload_changed(
        1, true, false, &owned, &identity, &sha256, &after, &sha256, &after, true, false,
    );
    watch.note_capture_requested();
    let receipt = watch.qualify()?;
    Ok(((*receipt).clone(), account, owned))
}

/// The producer's failed record: the prerequisite never reached the scene gate.
fn external_failed(args: &Args) -> Result<serde_json::Value, String> {
    use host_play::external_loader::ExternalWatch;
    let source = host_play::external_loader::resolve_source(args.external_ts.as_deref())?;
    let watch = ExternalWatch::default();
    let account =
        std::env::var("E2E_SUITE_FIXTURE_ACCOUNT").unwrap_or_else(|_| "fixture".to_string());
    watch.configure(
        account,
        source,
        host_play::external_loader::FROZEN_SHA256.into(),
    );
    watch.note_prereq_failed("fixture did not reach the scene gate");
    Ok(watch.evidence())
}

/// Write a capture under `label` with the actor binding the panel records for the external
/// terminal shot. `valid` selects a real PNG and an in-game sidecar.
fn write_actor_capture(label: &str, actor: &str, seconds: u32, valid: bool) -> Option<PathBuf> {
    let root = capture_root()?;
    let dir = root.join(format!("fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let stem = format!("2026-09-14T00-00-{seconds:02}_{}", safe_label(label));
    let png = dir.join(format!("{stem}.png"));
    let bytes: Vec<u8> = if valid {
        TINY_PNG.to_vec()
    } else {
        b"not a png at all".to_vec()
    };
    std::fs::write(&png, bytes).ok()?;
    let sidecar = if valid {
        format!("{{\"ingame\":true,\"scene_state\":2,\"actor\":\"{actor}\"}}")
    } else {
        "{\"ingame\":false,\"scene_state\":1}".to_string()
    };
    std::fs::write(dir.join(format!("{stem}.json")), sidecar).ok()?;
    Some(png)
}

/// The external loader smoke's terminal lines: the witness the panel prints plus its terminal
/// decision, or a deliberately incomplete/inconsistent pair for the suite's refusal tests.
fn external_mode(args: &Args) -> i32 {
    use host_play::external_loader::TERMINAL_SHOT;
    let live = args.live_name();
    let mode = args.mode.as_str();
    if mode == "external-fail" {
        let receipt = match external_failed(args) {
            Ok(receipt) => receipt,
            Err(error) => {
                eprintln!("FAIL: external_watch: {error}");
                return 1;
            }
        };
        eprintln!("EXTERNAL_LOADER: {live} {receipt}");
        eprintln!("FAIL: live {live} {receipt}");
        return 1;
    }
    let (receipt, account, _owned) = match external_qualify(args) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("FAIL: external_watch: {error}");
            return 1;
        }
    };
    let mut witness = receipt.clone();
    let mut terminal = receipt.clone();
    match mode {
        "external-pass" => {
            if let Some(path) = write_actor_capture(TERMINAL_SHOT, &account, 2, true) {
                println!("[panel] shot {TERMINAL_SHOT} -> {}", path.display());
            }
        }
        "external-prereq-only" => {
            // Only the prerequisite capture is written: a different label, never the contract.
            if let Some(path) =
                write_actor_capture(host_play::external_loader::PREREQ_SHOT, &account, 1, true)
            {
                println!(
                    "[panel] shot {} -> {}",
                    host_play::external_loader::PREREQ_SHOT,
                    path.display()
                );
            }
        }
        "external-no-capture" => {}
        "external-bad-capture" => {
            if let Some(path) = write_actor_capture(TERMINAL_SHOT, &account, 2, false) {
                println!("[panel] shot {TERMINAL_SHOT} -> {}", path.display());
            }
        }
        "external-inconsistent" => {
            // The witness is the qualified record; the terminal line contradicts it.
            terminal["capture_requested"] = serde_json::json!(false);
        }
        "external-incomplete" => {
            // Both lines carry a record that does not confirm the capture request.
            witness["capture_requested"] = serde_json::json!(false);
            terminal["capture_requested"] = serde_json::json!(false);
        }
        other => {
            eprintln!("fixture: unhandled external mode {other:?}");
            return 2;
        }
    }
    if mode != "external-no-witness" {
        println!("EXTERNAL_LOADER: {live} {witness}");
    }
    println!("PASS: live {live} {terminal}");
    0
}

fn main() {
    let args = parse();
    record_launch(&args);
    if let Some(path) = &args.report_args {
        let cwd = std::env::current_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default();
        let argv0 = std::env::args().next().unwrap_or_default();
        let rest = std::env::args().skip(1).collect::<Vec<_>>().join("\n");
        let _ = std::fs::write(path, format!("cwd={cwd}\nargv0={argv0}\n{rest}"));
    }
    if args.ignore_stop {
        ignore_graceful_stop();
    }
    if args.sleep_ms > 0 && args.mode != "hang" {
        std::thread::sleep(Duration::from_millis(args.sleep_ms));
    }
    let live = args.live_name();
    let scenario = args.scenario();
    if args.mode.starts_with("external-") {
        // The loader smoke's own contract: the producer's real state machine and receipt.
        let _ = std::io::stdout().flush();
        std::process::exit(external_mode(&args));
    }
    let shot_label = args.shot.clone().or_else(|| declared_shot(&scenario));
    println!("live {live}: running step 1/2");
    let _ = std::io::stdout().flush();

    let write_shot = |wanted: bool| {
        if !wanted {
            return;
        }
        if let Some(label) = &shot_label {
            if let Some(path) = write_capture(label, args.mode != "broken-shots") {
                println!("[panel] shot {label} -> {}", path.display());
            }
        }
    };

    match args.mode.as_str() {
        #[cfg(unix)]
        "escape-pipe" => {
            use std::os::unix::process::CommandExt;
            let pid_file = std::env::var_os("E2E_SUITE_ESCAPED_PID").expect("test pid path");
            // Suite owns and reaps this hang child by recorded PID.
            #[allow(clippy::zombie_processes)] // suite owns hang child via recorded PID
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--mode", "hang"])
                .process_group(0)
                .spawn()
                .unwrap();
            // This deliberately leaves the suite's group to exercise refusal when an
            // output pipe remains open. The test owns and terminates this exact PID.
            if let Err(error) = std::fs::write(pid_file, child.id().to_string()) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("record escaped fixture pid: {error}");
            }
        }
        "descendant" | "descendant-pipe" | "descendant-ignore-stop" => {
            let pid_file = std::env::var_os("E2E_SUITE_DESCENDANT_PID")
                .expect("the suite names the file the descendant pid is recorded in");
            let exe = std::env::current_exe().expect("the fixture binary path");
            let mut command = std::process::Command::new(exe);
            command.args(["--mode", "hang"]);
            if args.mode == "descendant-ignore-stop" {
                command.arg("--ignore-stop");
            }
            if args.mode != "descendant-pipe" {
                // A descendant with its own stdio: nothing here can see it through the
                // pipes, which is exactly the case the suite has to catch by ownership.
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
            }
            // Suite owns and reaps this hang child by recorded PID.
            #[allow(clippy::zombie_processes)] // suite owns hang child via recorded PID
            let mut child = command.spawn().expect("spawn the descendant fixture");
            if let Err(error) = std::fs::write(&pid_file, child.id().to_string()) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("record descendant fixture pid: {error}");
            }
        }
        "hang" => loop {
            std::thread::sleep(Duration::from_millis(200));
        },
        "fail" => {
            println!(
                "FAIL: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"FAIL\",\"message\":\"assertion failed: no action observed\"}}"
            );
            std::process::exit(1);
        }
        "zero-no-receipt" => {
            println!("live {live}: proving proof predicate");
        }
        "malformed" => {
            write_shot(true);
            println!("PASS: live {live} {{not json");
        }
        "duplicate" => {
            write_shot(true);
            println!("PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\"}}");
            println!("PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\"}}");
        }
        "dual-terminal" => {
            write_shot(true);
            println!("PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\"}}");
            println!("FAIL: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"FAIL\"}}");
            if let Some((kind, identity)) = witness(&args) {
                println!(
                    "{kind}: {live} {{\"case\":\"{identity}\",\"post_start_observations\":2}}"
                );
            }
        }
        "wrong-witness" => {
            write_shot(true);
            println!("PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\"}}");
            println!(
                "CATALOG_CORE: {live} {{\"case\":\"some_other_case\",\"post_start_observations\":2}}"
            );
        }
        "missing-witness" => {
            write_shot(true);
            println!("PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\"}}");
        }
        "no-shot" => {
            // The panel holds a PASS until the terminal shot is written; a PASS with no
            // capture must not qualify.
            println!("PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\"}}");
            if let Some((kind, identity)) = witness(&args) {
                println!(
                    "{kind}: {live} {{\"case\":\"{identity}\",\"post_start_observations\":2}}"
                );
            }
        }
        "wrong-label" => {
            // A capture for another label is not this case's contracted evidence.
            if let Some(path) = write_capture("unrelated shot", true) {
                println!("[panel] shot unrelated shot -> {}", path.display());
            }
            println!("PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\"}}");
            if let Some((kind, identity)) = witness(&args) {
                println!(
                    "{kind}: {live} {{\"case\":\"{identity}\",\"post_start_observations\":2}}"
                );
            }
        }
        "infra" => {
            println!("FATAL: engine unavailable");
            std::process::exit(1);
        }
        "panic" => {
            println!("thread 'main' panicked at crates/panel/src/session.rs:1:1:");
            std::process::exit(101);
        }
        "slow-pass" | "pass" | "broken-shots" => {
            write_shot(true);
            println!("PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\"}}");
            if let Some((kind, identity)) = witness(&args) {
                println!(
                    "{kind}: {live} {{\"case\":\"{identity}\",\"post_start_observations\":2}}"
                );
            }
        }
        other => {
            eprintln!("fixture: unknown mode {other:?}");
            std::process::exit(2);
        }
    }
    let _ = std::io::stdout().flush();
}

/// Ignore the graceful stop the suite sends first, on either platform, so the suite's
/// escalation to its forced stop is what ends this process. Unix ignores `SIGTERM`; Windows
/// answers the console `CTRL_C`/`CTRL_BREAK` events with "handled" and keeps running.
fn ignore_graceful_stop() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{FALSE, TRUE};
        use windows_sys::Win32::System::Console::{
            SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT,
        };
        unsafe extern "system" fn ignore(ctrltype: u32) -> i32 {
            match ctrltype {
                CTRL_C_EVENT | CTRL_BREAK_EVENT => TRUE,
                _ => FALSE,
            }
        }
        unsafe {
            SetConsoleCtrlHandler(Some(ignore), TRUE);
        }
    }
}
