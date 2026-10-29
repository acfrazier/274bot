use std::io::{self, BufRead, Write};

use thiserror::Error;

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Calculate(#[from] seed_calc::Error),
    #[error("usage: seed-calc [QUERIES.jsonl] (or provide JSON lines on standard input)")]
    Usage,
}
fn run() -> Result<(), CliError> {
    let mut args = std::env::args_os().skip(1);
    let input: Box<dyn BufRead> = match args.next() {
        Some(path) => Box::new(io::BufReader::new(std::fs::File::open(path)?)),
        None => Box::new(io::stdin().lock()),
    };
    if args.next().is_some() {
        return Err(CliError::Usage);
    }
    let mut output = io::BufWriter::new(io::stdout().lock());
    let mut summaries = io::stderr().lock();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let query = serde_json::from_str(&line)?;
        match seed_calc::calculate(&query) {
            Ok(answer) => {
                serde_json::to_writer(&mut output, &answer)?;
                writeln!(output)?;
                writeln!(summaries, "{}", answer.summary())?;
            }
            Err(error) => {
                serde_json::to_writer(
                    &mut output,
                    &serde_json::json!({"status":"error", "error":error.to_string()}),
                )?;
                writeln!(output)?;
                writeln!(summaries, "query refused: {error}")?;
            }
        }
    }
    output.flush()?;
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
