//! `e2e-suite`: the tracked native suite entrypoint.
//!
//! Usage lives in `docs/e2e-suite.md`; `e2e-suite --help` prints the flag list.

fn main() {
    e2e::suite::child::install_signal_handler();
    std::process::exit(e2e::suite::cli::main(std::env::args().skip(1)));
}
