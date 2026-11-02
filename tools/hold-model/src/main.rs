use hold_model::{
    DEFAULT_BYTE_CAP, DEFAULT_FRONTIER_CAP, DEFAULT_STATE_CAP, Exploration, Limits, Model,
    Mutation, Profile, describe_trace, explore_profile, profile, profiles,
};
use std::process::ExitCode;

struct Cli {
    profiles: Vec<Profile>,
    mutation: Mutation,
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
    for profile in &cli.profiles {
        let result = explore_profile(
            profile,
            cli.mutation,
            cli.horizon,
            cli.limits,
            cli.all_violations,
        );
        worst = worst.max(report(profile, cli.mutation, &result));
    }
    ExitCode::from(worst)
}

/// Print one run. Returns the exit code: 0 clean and complete, 1 violation,
/// 3 a cap stopped the run.
fn report(
    profile: &Profile,
    mutation: Mutation,
    result: &Exploration<hold_model::State, hold_model::Action>,
) -> u8 {
    println!(
        "profile={} mutation={} horizon={} states={} transitions={} deepest={} max_frontier={} est_bytes={} elapsed_ms={}",
        profile.name,
        mutation.name(),
        result.horizon,
        result.states,
        result.transitions,
        result.deepest,
        result.max_frontier,
        result.estimated_peak_bytes,
        result.elapsed.as_millis()
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
    let model = Model::new(profile.bounds, mutation);
    let failures: Vec<_> = result.failure.iter().chain(&result.failures).collect();
    for (violation, trace) in &failures {
        println!(
            "VIOLATION {}: {}\nshortest trace ({} steps):\n{}",
            violation.name,
            violation.detail,
            trace.len(),
            describe_trace(&model, trace)
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
        profiles: Vec::new(),
        mutation: Mutation::None,
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
            "--profile" => {
                let name = args.get(index + 1).ok_or("--profile needs a name")?;
                if name == "all-profiles" {
                    cli.profiles = profiles();
                } else {
                    cli.profiles
                        .push(profile(name).ok_or_else(|| format!("unknown profile {name}"))?);
                }
                index += 2;
            }
            "--mutation" => {
                let name = args.get(index + 1).ok_or("--mutation needs a name")?;
                cli.mutation =
                    Mutation::parse(name).ok_or_else(|| format!("unknown mutation {name}"))?;
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
    if cli.profiles.is_empty() {
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
    "usage: hold-model --profile NAME|all-profiles [--mutation NAME] [--horizon N] [--state-cap N] [--frontier-cap N] [--byte-cap N]\nEvery run is bounded: a horizon, a state cap, a frontier cap and a byte cap. Run it on chardonnay inside a memory scope."
}
