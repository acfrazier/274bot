use hostlog_model::{Limits, Mutation, explore_profile, profile, profiles};
use std::process::ExitCode;

/// Defaults of one run; the profiles stay far below them.
const STATE_CAP: usize = 5_000_000;
const FRONTIER_CAP: usize = 2_000_000;
const BYTE_CAP: usize = 2 * 1024 * 1024 * 1024;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut profile_name = String::new();
    let mut mutation = Mutation::None;
    let mut horizon = None;
    let mut limits = Limits {
        state_cap: STATE_CAP,
        frontier_cap: FRONTIER_CAP,
        byte_cap: BYTE_CAP,
    };
    let mut all_violations = false;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        let value = args.get(index + 1);
        let number = || {
            value
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or_else(|| format!("{option} needs a number"))
        };
        let step = match option {
            "--profile" => {
                profile_name = value.cloned().unwrap_or_default();
                2
            }
            "--mutation" => {
                let name = value.map_or("", String::as_str);
                match Mutation::parse(name) {
                    Some(parsed) => mutation = parsed,
                    None => return usage(&format!("unknown mutation {name}")),
                }
                2
            }
            "--all-violations" => {
                all_violations = true;
                1
            }
            "--horizon" | "--state-cap" | "--frontier-cap" | "--byte-cap" => {
                let n = match number() {
                    Ok(n) => n,
                    Err(message) => return usage(&message),
                };
                match option {
                    "--horizon" => horizon = Some(n),
                    "--state-cap" => limits.state_cap = n,
                    "--frontier-cap" => limits.frontier_cap = n,
                    _ => limits.byte_cap = n,
                }
                2
            }
            other => return usage(&format!("unknown option {other}")),
        };
        index += step;
    }
    let chosen = if profile_name == "all-profiles" {
        profiles()
    } else {
        match profile(&profile_name) {
            Some(p) => vec![p],
            None => return usage("choose --profile NAME or --profile all-profiles"),
        }
    };
    let mut worst = 0u8;
    for p in &chosen {
        let result = explore_profile(p, mutation, horizon, limits, all_violations);
        println!(
            "model=hostlog profile={} mutation={} horizon={} states={} transitions={} deepest={} elapsed_ms={}",
            p.name,
            mutation.name(),
            result.horizon,
            result.states,
            result.transitions,
            result.deepest,
            result.elapsed.as_millis()
        );
        let failures: Vec<_> = result.failure.iter().chain(&result.failures).collect();
        for (violation, trace) in &failures {
            println!(
                "VIOLATION {}: {}\nshortest trace ({} steps):",
                violation.name,
                violation.detail,
                trace.len()
            );
            for (n, action) in trace.iter().enumerate() {
                println!("  {:>2}. {action:?}", n + 1);
            }
        }
        if let Some((kind, message)) = &result.cap {
            println!("CAP {kind:?}: {message}; the run is incomplete");
            worst = worst.max(if failures.is_empty() { 3 } else { 1 });
        } else if failures.is_empty() {
            println!("complete: no violation within the horizon");
        } else {
            worst = worst.max(1);
        }
    }
    ExitCode::from(worst)
}

fn usage(message: &str) -> ExitCode {
    eprintln!(
        "{message}\nusage: hostlog-model --profile NAME|all-profiles [--mutation NAME] [--horizon N] [--state-cap N] [--frontier-cap N] [--byte-cap N] [--all-violations]\nEvery run is bounded: a horizon, a state cap, a frontier cap and a byte cap."
    );
    ExitCode::from(2)
}
