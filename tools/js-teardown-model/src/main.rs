use js_teardown_model::{
    DEFAULT_BYTE_CAP, DEFAULT_FRONTIER_CAP, DEFAULT_STATE_CAP, Exploration, Limits, explore_retire,
    explore_teardown, retire, retire_profile, retire_profiles, teardown, teardown_profile,
    teardown_profiles,
};
use std::fmt::Debug;
use std::process::ExitCode;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Which {
    Teardown,
    Retire,
}

struct Cli {
    which: Which,
    profile: String,
    mutation: String,
    horizon: Option<usize>,
    limits: Limits,
    all_violations: bool,
}

fn main() -> ExitCode {
    let cli = match parse_args() {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}\n{}", usage());
            return ExitCode::from(2);
        }
    };
    let mut worst = 0u8;
    match cli.which {
        Which::Teardown => {
            let Some(mutation) = teardown::Mutation::parse(&cli.mutation) else {
                eprintln!("unknown teardown mutation {}\n{}", cli.mutation, usage());
                return ExitCode::from(2);
            };
            let profiles = if cli.profile == "all-profiles" {
                teardown_profiles()
            } else {
                match teardown_profile(&cli.profile) {
                    Some(profile) => vec![profile],
                    None => {
                        eprintln!("unknown teardown profile {}\n{}", cli.profile, usage());
                        return ExitCode::from(2);
                    }
                }
            };
            let model_for = |profile: &js_teardown_model::TeardownProfile| {
                teardown::Teardown::new(profile.bounds, mutation)
            };
            for profile in &profiles {
                let result = explore_teardown(
                    profile,
                    mutation,
                    cli.horizon,
                    cli.limits,
                    cli.all_violations,
                );
                let model = model_for(profile);
                worst = worst.max(report(
                    "teardown",
                    profile.name,
                    mutation.name(),
                    &result,
                    |trace| teardown::describe_trace(&model, trace),
                ));
            }
        }
        Which::Retire => {
            let Some(mutation) = retire::Mutation::parse(&cli.mutation) else {
                eprintln!("unknown retire mutation {}\n{}", cli.mutation, usage());
                return ExitCode::from(2);
            };
            let profiles = if cli.profile == "all-profiles" {
                retire_profiles()
            } else {
                match retire_profile(&cli.profile) {
                    Some(profile) => vec![profile],
                    None => {
                        eprintln!("unknown retire profile {}\n{}", cli.profile, usage());
                        return ExitCode::from(2);
                    }
                }
            };
            for profile in &profiles {
                let result = explore_retire(
                    profile,
                    mutation,
                    cli.horizon,
                    cli.limits,
                    cli.all_violations,
                );
                let model = retire::Retire::new(profile.bounds.clone(), mutation);
                worst = worst.max(report(
                    "retire",
                    profile.name,
                    mutation.name(),
                    &result,
                    |trace| retire::describe_trace(&model, trace),
                ));
            }
        }
    }
    ExitCode::from(worst)
}

/// Print one run. Returns the exit code: 0 clean and complete, 1 violation,
/// 3 a cap stopped the run.
fn report<S, A: Clone + Debug>(
    model: &str,
    profile: &str,
    mutation: &str,
    result: &Exploration<S, A>,
    describe: impl Fn(&[A]) -> String,
) -> u8 {
    println!(
        "model={model} profile={profile} mutation={mutation} horizon={} states={} transitions={} deepest={} max_frontier={} est_bytes={} elapsed_ms={}",
        result.horizon,
        result.states,
        result.transitions,
        result.deepest,
        result.max_frontier,
        result.estimated_peak_bytes,
        result.elapsed.as_millis(),
    );
    let peak_rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmHWM:")
                    .and_then(|rest| rest.trim().strip_suffix("kB"))
                    .and_then(|kb| kb.trim().parse::<u64>().ok())
            })
        });
    match peak_rss {
        Some(kb) => println!("peak_rss_kb={kb}"),
        None => println!("peak_rss_kb=unavailable"),
    }
    let failures: Vec<_> = result.failure.iter().chain(&result.failures).collect();
    for (violation, trace) in &failures {
        println!(
            "VIOLATION {}: {}\nshortest trace ({} steps):\n{}",
            violation.name,
            violation.detail,
            trace.len(),
            describe(trace)
        );
    }
    if let Some((kind, message)) = &result.cap {
        println!("CAP {kind:?}: {message}; the run is incomplete");
        return if failures.is_empty() { 3 } else { 1 };
    }
    if !failures.is_empty() {
        println!("{} violation name(s) found", failures.len());
        return 1;
    }
    println!("complete: no violation within the horizon");
    0
}

fn parse_args() -> Result<Cli, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cli = Cli {
        which: Which::Teardown,
        profile: String::new(),
        mutation: "none".to_string(),
        horizon: None,
        all_violations: false,
        limits: Limits {
            state_cap: DEFAULT_STATE_CAP,
            frontier_cap: DEFAULT_FRONTIER_CAP,
            byte_cap: DEFAULT_BYTE_CAP,
        },
    };
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        match option {
            "--model" => {
                cli.which = match args.get(index + 1).map(String::as_str) {
                    Some("teardown") => Which::Teardown,
                    Some("retire") => Which::Retire,
                    _ => return Err("--model needs teardown or retire".to_string()),
                };
                index += 2;
            }
            "--profile" => {
                cli.profile
                    .clone_from(args.get(index + 1).ok_or("--profile needs a name")?);
                index += 2;
            }
            "--mutation" => {
                cli.mutation
                    .clone_from(args.get(index + 1).ok_or("--mutation needs a name")?);
                index += 2;
            }
            "--all-violations" => {
                cli.all_violations = true;
                index += 1;
            }
            "--horizon" => {
                cli.horizon = Some(parse_number(&args, index, option)?);
                index += 2;
            }
            "--state-cap" => {
                cli.limits.state_cap = parse_number(&args, index, option)?;
                index += 2;
            }
            "--frontier-cap" => {
                cli.limits.frontier_cap = parse_number(&args, index, option)?;
                index += 2;
            }
            "--byte-cap" => {
                cli.limits.byte_cap = parse_number(&args, index, option)?;
                index += 2;
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if cli.profile.is_empty() {
        return Err("choose --profile NAME or --profile all-profiles".to_string());
    }
    Ok(cli)
}

fn parse_number(args: &[String], index: usize, option: &str) -> Result<usize, String> {
    args.get(index + 1)
        .ok_or_else(|| format!("{option} needs a number"))?
        .parse()
        .map_err(|_| format!("{option} needs a number"))
}

fn usage() -> &'static str {
    "usage: js-teardown-model --model teardown|retire --profile NAME|all-profiles [--mutation NAME] [--horizon N] [--state-cap N] [--frontier-cap N] [--byte-cap N] [--all-violations]\nEvery run is bounded: a horizon, a state cap, a frontier cap and a byte cap."
}
