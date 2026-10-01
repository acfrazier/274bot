use std::env;
use std::fs::File;
use std::io;
use std::path::PathBuf;

use flate2::write::GzEncoder;
use flate2::Compression;

fn main() -> io::Result<()> {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    for revision in [274, 289] {
        let source = format!("data/game-data/{revision}.json");
        println!("cargo:rerun-if-changed={source}");
        let mut input = File::open(source)?;
        let output = File::create(out.join(format!("game-data-{revision}.json.gz")))?;
        let mut encoder = GzEncoder::new(output, Compression::best());
        io::copy(&mut input, &mut encoder)?;
        encoder.finish()?;
    }
    Ok(())
}
