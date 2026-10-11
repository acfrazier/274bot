use seat_model::{Bounds, explore};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let closure = args.first().is_some_and(|value| value == "--closure");
    let horizon = if closure {
        usize::MAX
    } else {
        match args.first() {
            None => 6,
            Some(value) => match value.parse::<usize>() {
                Ok(value) => value,
                Err(_) => {
                    eprintln!("usage: seat-model [transition-horizon | --closure]");
                    return ExitCode::FAILURE;
                }
            },
        }
    };
    let bounds = if closure {
        Bounds::closure()
    } else {
        Bounds::small()
    };
    let result = explore(bounds, horizon);
    println!("rules=chosen bounds={bounds:?} horizon={horizon}");
    println!(
        "states={} transitions={} max_frontier={} elapsed_seconds={:.6}",
        result.states,
        result.transitions,
        result.max_frontier,
        result.elapsed.as_secs_f64()
    );
    if let Some((failure, path)) = result.failure {
        eprintln!("FAIL {}: {}\n{path:#?}", failure.name, failure.detail);
        ExitCode::FAILURE
    } else {
        println!("PASS: every invariant checked after every enabled transition");
        ExitCode::SUCCESS
    }
}
