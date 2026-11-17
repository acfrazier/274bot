//! Rewrite `paths/path.schema.json` from the Rust Path document types.
//!
//! An explicit maintenance command, not a test: `tests/path_schema.rs`
//! compares the checked-in schema to the generator and names this command
//! when it is stale.
//!
//! Usage: cargo run -p script --example regen_path_schema --features path-schema

use script::quester::schema::{path_schema_path, render};

fn main() {
    let path = path_schema_path();
    if let Err(error) = std::fs::write(&path, render()) {
        eprintln!("regen_path_schema: write {}: {error}", path.display());
        std::process::exit(1);
    }
    println!("wrote {}", path.display());
}
