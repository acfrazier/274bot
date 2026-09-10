//! Print an identity manifest for a cache paired with a known server revision.
//! cargo run -p host-play --example cache_manifest -- 289 /path/to/client-cache
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = (|| {
        if args.len() != 2 {
            return Err("usage: cache_manifest 274|289 CACHE_DIR; assert the prepared server revision explicitly".to_string());
        }
        let revision = host_play::profile::parse_revision(&args[0])?.as_i32() as u16;
        let manifest = host_play::profile::CacheManifest::capture(revision, Path::new(&args[1]))?;
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())
    })();
    match result {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("cache_manifest: {error}");
            std::process::exit(2);
        }
    }
}
