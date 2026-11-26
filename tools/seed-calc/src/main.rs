use std::ffi::OsString;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use seed_calc::Revision;
use thiserror::Error;

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Calculate(#[from] seed_calc::Error),
    #[error("{0}")]
    InvalidRevision(String),
    #[error(
        "usage: seed-calc [--revision 289|377] [QUERIES.jsonl] (or provide JSON lines on standard input)"
    )]
    Usage,
}

#[derive(Debug, PartialEq, Eq)]
struct CliArgs {
    revision: Revision,
    input_path: Option<PathBuf>,
}

fn parse_revision_os(raw: OsString) -> Result<Revision, CliError> {
    let text = raw
        .into_string()
        .map_err(|_| CliError::InvalidRevision("invalid --revision value".into()))?;
    Revision::parse(&text).map_err(CliError::InvalidRevision)
}

fn parse_args<I>(args: I) -> Result<CliArgs, CliError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut iter = args.into_iter();
    let mut revision = Revision::R289;
    let mut revision_set = false;
    let mut input_path: Option<PathBuf> = None;
    let mut stop_flags = false;

    while let Some(arg) = iter.next() {
        if !stop_flags {
            if arg == "--" {
                stop_flags = true;
                continue;
            }
            if arg == "--revision" {
                if revision_set {
                    return Err(CliError::InvalidRevision(
                        "duplicate --revision option".into(),
                    ));
                }
                let value = iter.next().ok_or_else(|| {
                    CliError::InvalidRevision("missing value for --revision".into())
                })?;
                revision = parse_revision_os(value)?;
                revision_set = true;
                continue;
            }
            if let Some(raw) = arg.to_str().and_then(|s| s.strip_prefix("--revision=")) {
                if revision_set {
                    return Err(CliError::InvalidRevision(
                        "duplicate --revision option".into(),
                    ));
                }
                revision = Revision::parse(raw).map_err(CliError::InvalidRevision)?;
                revision_set = true;
                continue;
            }
            if arg.to_str().is_some_and(|s| s.starts_with('-')) {
                return Err(CliError::Usage);
            }
        }
        if input_path.is_some() {
            return Err(CliError::Usage);
        }
        input_path = Some(PathBuf::from(arg));
    }

    Ok(CliArgs {
        revision,
        input_path,
    })
}

fn run() -> Result<(), CliError> {
    let cli = parse_args(std::env::args_os().skip(1))?;
    let input: Box<dyn BufRead> = match cli.input_path {
        Some(path) => Box::new(io::BufReader::new(std::fs::File::open(path)?)),
        None => Box::new(io::stdin().lock()),
    };
    let mut output = io::BufWriter::new(io::stdout().lock());
    let mut summaries = io::stderr().lock();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let query = serde_json::from_str(&line)?;
        match seed_calc::calculate_for_revision(&query, cli.revision) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn os(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn cli_defaults_to_289_and_accepts_377_flag_forms() {
        assert_eq!(
            parse_args(os(&[])).unwrap(),
            CliArgs {
                revision: Revision::R289,
                input_path: None,
            }
        );
        assert_eq!(
            parse_args(os(&["queries.jsonl"])).unwrap(),
            CliArgs {
                revision: Revision::R289,
                input_path: Some(PathBuf::from("queries.jsonl")),
            }
        );
        assert_eq!(
            parse_args(os(&["--revision", "377", "queries.jsonl"])).unwrap(),
            CliArgs {
                revision: Revision::R377,
                input_path: Some(PathBuf::from("queries.jsonl")),
            }
        );
        assert_eq!(
            parse_args(os(&["queries.jsonl", "--revision=377"])).unwrap(),
            CliArgs {
                revision: Revision::R377,
                input_path: Some(PathBuf::from("queries.jsonl")),
            }
        );
        assert_eq!(
            parse_args(os(&["--revision=289"])).unwrap(),
            CliArgs {
                revision: Revision::R289,
                input_path: None,
            }
        );
    }

    #[test]
    fn cli_refuses_invalid_duplicate_or_unknown_revision_options() {
        assert!(parse_args(os(&["--revision"])).is_err());
        assert!(parse_args(os(&["--revision", "274"])).is_err());
        assert!(parse_args(os(&["--revision=abc"])).is_err());
        assert!(parse_args(os(&["--revision", "289", "--revision", "377"])).is_err());
        assert!(parse_args(os(&["--unknown"])).is_err());
        assert!(parse_args(os(&["a.jsonl", "b.jsonl"])).is_err());
    }
}
