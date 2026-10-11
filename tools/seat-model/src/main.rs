use seat_model::{
    Bounds, Exploration, ExploreLimits, Profile, explore_profile_with_limits, explore_with_limits,
    profile as find_profile, profiles,
};
use std::process::ExitCode;

enum Selection {
    Profiles,
    Named(Profile),
    Legacy {
        name: &'static str,
        bounds: Bounds,
        horizon: usize,
    },
}

struct Cli {
    selection: Selection,
    limits: ExploreLimits,
}

fn main() -> ExitCode {
    let cli = match parse_args() {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{error}\n{}", usage());
            return ExitCode::FAILURE;
        }
    };

    match cli.selection {
        Selection::Profiles => {
            let mut passed = true;
            for profile in profiles() {
                let result = explore_profile_with_limits(
                    profile,
                    seat_model::Rules::Chosen,
                    seat_model::Mutation::None,
                    cli.limits,
                );
                passed &= report(profile.name(), &result);
            }
            if passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Selection::Named(profile) => {
            let result = explore_profile_with_limits(
                &profile,
                seat_model::Rules::Chosen,
                seat_model::Mutation::None,
                cli.limits,
            );
            if report(profile.name(), &result) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Selection::Legacy {
            name,
            bounds,
            horizon,
        } => {
            let result = explore_with_limits(
                bounds,
                horizon,
                seat_model::Rules::Chosen,
                seat_model::Mutation::None,
                cli.limits,
            );
            if report(name, &result) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn report(name: &str, result: &Exploration) -> bool {
    println!(
        "profile={name} rules=chosen bounds={:?} horizon={} prefix_transitions={} state_cap={} frontier_cap={} byte_cap={}",
        result.bounds,
        result.horizon,
        result.prefix_transitions,
        result.state_cap,
        result.frontier_cap,
        result.byte_cap
    );
    println!(
        "states={} transitions={} max_frontier={} estimated_peak_bytes={} elapsed_seconds={:.6}",
        result.states,
        result.transitions,
        result.max_frontier,
        result.estimated_peak_bytes,
        result.elapsed.as_secs_f64()
    );
    println!("coverage={:?}", result.coverage);
    let peak_rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmHWM:")?
                    .split_whitespace()
                    .next()?
                    .parse::<usize>()
                    .ok()
            })
        });
    match peak_rss {
        Some(kib) => println!("peak_rss_kib={kib}"),
        None => println!("peak_rss_kib=unavailable"),
    }

    if let Some(cap) = result.cap_reached {
        if let Some((failure, _)) = &result.failure {
            eprintln!("CAP-REACHED {cap:?}: {}", failure.detail);
        } else {
            eprintln!("CAP-REACHED {cap:?}");
        }
        false
    } else if let Some((failure, path)) = &result.failure {
        eprintln!("FAIL {}: {}", failure.name, failure.detail);
        if !path.is_empty() {
            eprintln!("{path:#?}");
        }
        false
    } else if result.complete {
        println!("PASS: bounded exhaustive exploration completed");
        true
    } else {
        eprintln!("FAIL: exploration did not complete");
        false
    }
}

fn parse_args() -> Result<Cli, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut all_profiles = false;
    let mut selected_profile = None;
    let mut closure = false;
    let mut positional_horizon = None;
    let mut limits = ExploreLimits::default();
    let mut explicit_state_cap = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--profiles" => all_profiles = true,
            "--profile" => {
                index += 1;
                selected_profile = Some(
                    args.get(index)
                        .ok_or_else(|| "--profile requires a name".to_owned())?
                        .clone(),
                );
            }
            "--closure" => closure = true,
            "--state-cap" => {
                index += 1;
                limits.state_cap = parse_limit(&args, index, "--state-cap")?;
                explicit_state_cap = true;
            }
            "--frontier-cap" => {
                index += 1;
                limits.frontier_cap = parse_limit(&args, index, "--frontier-cap")?;
            }
            "--byte-cap" => {
                index += 1;
                limits.byte_cap = parse_limit(&args, index, "--byte-cap")?;
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option {value}"));
            }
            value => {
                if positional_horizon.is_some() {
                    return Err("only one transition horizon is accepted".to_owned());
                }
                positional_horizon = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid transition horizon {value:?}"))?,
                );
            }
        }
        index += 1;
    }

    let selector_count = (if all_profiles { 1 } else { 0 })
        + (if selected_profile.is_some() { 1 } else { 0 })
        + (if closure { 1 } else { 0 })
        + (if positional_horizon.is_some() { 1 } else { 0 });
    if selector_count > 1 {
        return Err("choose only one of --profiles, --profile, --closure, or a horizon".to_owned());
    }

    let selection = if all_profiles {
        Selection::Profiles
    } else if let Some(name) = selected_profile {
        let profile = find_profile(&name).ok_or_else(|| format!("unknown profile {name:?}"))?;
        Selection::Named(profile)
    } else if closure {
        if !explicit_state_cap {
            return Err("--closure requires an explicit --state-cap".to_owned());
        }
        Selection::Legacy {
            name: "legacy-closure",
            bounds: Bounds::closure(),
            horizon: usize::MAX,
        }
    } else if positional_horizon.is_none() {
        Selection::Profiles
    } else {
        let horizon = positional_horizon.unwrap_or(6);
        if horizon > 6 && !explicit_state_cap {
            return Err(format!(
                "horizon {horizon} exceeds the legacy small horizon 6; pass --state-cap N explicitly"
            ));
        }
        Selection::Legacy {
            name: "legacy-small",
            bounds: Bounds::small(),
            horizon,
        }
    };

    Ok(Cli { selection, limits })
}

fn parse_limit(args: &[String], index: usize, option: &str) -> Result<usize, String> {
    args.get(index)
        .ok_or_else(|| format!("{option} requires a value"))?
        .parse::<usize>()
        .map_err(|_| format!("invalid value for {option}"))
}

fn usage() -> &'static str {
    "usage: seat-model [--profiles | --profile NAME | [transition-horizon] | --closure] \
     [--state-cap N] [--frontier-cap N] [--byte-cap N]"
}
