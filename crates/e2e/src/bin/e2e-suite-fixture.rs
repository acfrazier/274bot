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
//!   hang             never exits (budget/cleanup coverage); `--ignore-sigterm` to make
//!                    the suite escalate to a forced kill
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
//! Environment: `E2E_SUITE_FIXTURE_LOG` appends one line per launch (resume/no-duplicate
//! launch coverage), `274BOT_SMOKE_DIR` is the capture root the suite points at the run,
//! and `E2E_SUITE_FIXTURE_CORE` / `E2E_SUITE_FIXTURE_PAIR` name the witness identity the
//! fixture should print — the stand-in for the identity a real adapter derives from its
//! own configuration, since the suite hands a real child only
//! `--profile ... --catalog ... --live <name>`.

use std::io::Write;
use std::path::PathBuf;
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
    ignore_sigterm: bool,
    report_args: Option<PathBuf>,
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
        ignore_sigterm: false,
        report_args: None,
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
            "--ignore-sigterm" => args.ignore_sigterm = true,
            "--report-args" => args.report_args = take(&mut i).map(PathBuf::from),
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

fn main() {
    let args = parse();
    record_launch(&args);
    if let Some(path) = &args.report_args {
        let _ = std::fs::write(
            path,
            std::env::args().skip(1).collect::<Vec<_>>().join("\n"),
        );
    }
    if args.ignore_sigterm {
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
        }
    }
    if args.sleep_ms > 0 && args.mode != "hang" {
        std::thread::sleep(Duration::from_millis(args.sleep_ms));
    }
    let live = args.live_name();
    let scenario = args.scenario();
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
