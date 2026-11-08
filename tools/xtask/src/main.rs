mod ci;

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("ci") => match ci::run(args.collect()) {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                eprintln!("xtask: {error}");
                ExitCode::from(2)
            }
        },
        Some("--help" | "-h") | None => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some(command) => {
            eprintln!("xtask: unknown command `{command}`");
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn print_usage() {
    println!(
        "Usage: cargo run --locked -p xtask -- ci --list | --all | <step>... [--log-dir <path>]"
    );
}
