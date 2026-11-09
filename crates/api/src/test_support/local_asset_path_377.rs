use std::env;
use std::path::PathBuf;

/// Resolve a revision-377 asset, which the 274/289 local-data builder does not produce.
///
/// An absent default skips on every host. An explicit environment path must name
/// a file, including on CI. Callers must still decode and validate its contents.
pub fn local_asset_path_377(
    env_key: &str,
    default_path: &str,
    description: &str,
) -> Option<PathBuf> {
    if let Some(configured) = (!env_key.is_empty())
        .then(|| env::var_os(env_key))
        .flatten()
    {
        let path = PathBuf::from(configured);
        assert!(
            path.is_file(),
            "invalid {description} at {}: {env_key} must name an existing file",
            path.display()
        );
        return Some(path);
    }

    let path = PathBuf::from(default_path);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!(
            "Skipping {description} at {}: optional revision-377 asset is unavailable",
            path.display()
        );
        None
    }
}
