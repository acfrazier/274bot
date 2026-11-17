//! Rewrite checked-in script-js outputs from their generators.
//!
//! These are explicit maintenance commands, not tests: the freshness tests in
//! `tests/declared_abi.rs` and `tests/host_js.rs` compare the checked-in files
//! to the generators and name the matching command below when they fail.
//!
//! Usage: cargo run -p script-js --example regen_checked_in -- <command>
//!
//! Commands:
//! - `declared-surface`: rewrite `src/shim/declared_surface.js` from the
//!   checked-in fixture `tests/fixtures/js_declared_abi.json`.
//! - `host-js`: rewrite `host-js/index.d.ts` from the host verb tables.
//! - `compat-dts-tree <dir>`: write the authored compat declaration tree
//!   under `<dir>`, as frozen recounts consume it.

use std::path::PathBuf;

use script_js::tooling::compat_dts::write_compat_dts_tree;
use script_js::tooling::declared_abi::{
    declared_surface_path, load_fixture, write_declared_surface,
};
use script_js::tooling::host_js::{host_js_path, write_host_js_dts};

const USAGE: &str = "usage: regen_checked_in declared-surface | host-js | compat-dts-tree <dir>";

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("regen_checked_in: {error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["declared-surface"] => {
            let exports = load_fixture()?;
            write_declared_surface(&exports)?;
            println!("wrote {}", declared_surface_path().display());
            Ok(())
        }
        ["host-js"] => {
            write_host_js_dts()?;
            println!("wrote {}", host_js_path().display());
            Ok(())
        }
        ["compat-dts-tree", dir] => {
            let dir = PathBuf::from(dir);
            write_compat_dts_tree(&dir)?;
            println!("wrote {}", dir.display());
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}
