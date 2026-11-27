#![cfg(feature = "scenario-luau")]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use host_play::scenario_luau::{FileLaunch, FileOptions, FileSource, FILE_RECEIPT_NAME};
use host_play::{run_with_template, ProfileOptions};
use scenario::RunnerStatus;

const RETIREMENT_HEADROOM: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn env_port(name: &str) -> Result<Option<u16>, String> {
    env_text(name)
        .map(|raw| {
            raw.parse()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| format!("{name} must be a TCP port"))
        })
        .transpose()
}

pub fn evidence_path() -> PathBuf {
    if let Some(path) = env_path("SCENARIO_EVIDENCE") {
        return path;
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "274bot-scenario-file-{}-{nonce}",
        std::process::id()
    ))
}

pub fn registered_options(id: &str, case: &str) -> Result<FileOptions, String> {
    let source = FileSource::registered(id)
        .ok_or_else(|| format!("{id:?} is not a registered File scenario"))?;
    let mut options = FileOptions::new(source);
    options.case = Some(case.to_owned());
    options.evidence = Some(evidence_path());
    Ok(options)
}

pub fn run(options: FileOptions, expect_fail: bool) -> Result<(), String> {
    if !super::live() {
        return Ok(());
    }

    let selection = ProfileOptions {
        port: env_port("WORLD_GAME_PORT")?,
        http_port: env_port("WORLD_HTTP_PORT")?,
        engine_dir: env_path("WORLD_ENGINE_DIR"),
        content_dir: env_path("WORLD_CONTENT_DIR"),
        cache_dir: env_path("BOT_CACHE_DIR"),
        nav_pack: env_path("WORLD_NAV_PACK").or_else(|| env_path("BOT_NAV_PACK")),
        nav_flags: env_path("WORLD_NAV_FLAGS"),
        ..ProfileOptions::default()
    }
    .resolve_preset(None)?;
    let evidence = options
        .evidence
        .clone()
        .ok_or("File scenario E2E needs an evidence output path")?;
    let launch = FileLaunch::prepare(options, &selection, false)?;
    let deadline = launch.settings.deadline + RETIREMENT_HEADROOM;
    let entries = host_play::mint_live_entries(&launch.accounts);
    let template = selection.prepare_template()?;
    let mut play = run_with_template(template, false, Vec::new(), |_| (None, None), |_, _, _| {})?;
    let observed = launch.run.clone();
    play.link_file_scenario(launch.run, super::profiles(&entries))?;

    let terminal = observed.await_terminal(deadline, POLL_INTERVAL)?;
    let receipt = terminal.receipt;
    let path = evidence.join(FILE_RECEIPT_NAME);
    let file = std::fs::File::open(&path)
        .map_err(|error| format!("open File receipt {}: {error}", path.display()))?;
    let persisted: serde_json::Value = serde_json::from_reader(file)
        .map_err(|error| format!("read File receipt {}: {error}", path.display()))?;
    if persisted != receipt {
        return Err("durable File receipt does not match the runner's terminal receipt".into());
    }
    let text = serde_json::to_string(&receipt).map_err(|error| error.to_string())?;
    let retiring = terminal.retiring_observed;
    match (terminal.status, expect_fail) {
        (RunnerStatus::Passed, false) if !retiring => Err(format!(
            "scenario file passed without observing RunnerStatus::Retiring; evidence={}; receipt={text}",
            evidence.display()
        )),
        (RunnerStatus::Passed, false) => {
            println!(
                "PASS: scenario file; retiring_observed={retiring}; evidence={}; receipt={text}",
                evidence.display()
            );
            Ok(())
        }
        (RunnerStatus::Failed(error), true) => {
            let roles = receipt["roles"].as_array().ok_or("receipt has no roles")?;
            if receipt["outcome"] != "FAIL" || roles.iter().any(|role| role["resolved"] != true) {
                return Err(format!(
                    "expected FAIL with every role resolved: {error}; receipt={text}"
                ));
            }
            println!(
                "EXPECTED-FAIL: scenario file; failure={error}; evidence={}; receipt={text}",
                evidence.display()
            );
            Ok(())
        }
        (RunnerStatus::Failed(error), false) => Err(format!(
            "scenario file failed: {error}; evidence={}; receipt={text}",
            evidence.display()
        )),
        (status, _) => Err(format!(
            "unexpected terminal status {status:?}; receipt={text}"
        )),
    }
}
