//! Read-only identity export for offline packaging and generated-data tooling.
use std::path::Path;

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: cache-content-id REVISION JAG_DIR SNAPSHOT_DIR".into());
    }
    let revision = args[0].parse::<u16>().map_err(|e| e.to_string())?;
    let snapshot = Path::new(&args[2]);
    let snapshot = if snapshot.join("manifest").is_file() {
        snapshot.to_path_buf()
    } else {
        let bytes = std::fs::read(Path::new(&args[1]).join("versionlist")).map_err(|e| e.to_string())?;
        snapshot.join(client::unpack::version_hash(&bytes))
    };
    let identity =
        client::content_identity::compute_decoded_content_identity(revision, &args[1], &snapshot)
            .map_err(|e| e.to_string())?;
    let transfer = nav::manifest::CacheManifest::capture(revision, Path::new(&args[1]))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "format": "274DCI01", "revision": revision,
            "content_id": identity.content_id_hex(),
            "cache_id": transfer.identity(),
            "transfer": transfer,
        }))
        .map_err(|e| e.to_string())?
    );
    Ok(())
}
