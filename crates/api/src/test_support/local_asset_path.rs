use std::env;
use std::ffi::OsStr;
use std::path::PathBuf;

/// Resolve an optional local-only test asset. Missing paths are skippable only
/// in hosted GitHub Actions; local builds fail with actionable guidance.
pub fn local_asset_path(env_key: &str, default_path: &str, description: &str) -> Option<PathBuf> {
    let configured = (!env_key.is_empty())
        .then(|| env::var_os(env_key))
        .flatten()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let default = (!default_path.is_empty()).then(|| PathBuf::from(default_path));
    let path = configured.or(default);

    if path.as_ref().is_some_and(|path| path.exists()) {
        return path;
    }

    let missing = match (&path, env_key.is_empty()) {
        (Some(path), _) => format!("{} at {}", description, path.display()),
        (None, false) => format!("{} (set {env_key})", description),
        (None, true) => description.to_owned(),
    };
    if env::var_os("GITHUB_ACTIONS").as_deref() == Some(OsStr::new("true")) {
        eprintln!("Skipping {missing}: local asset is unavailable (GITHUB_ACTIONS=true)");
        None
    } else {
        panic!("missing {missing}; local assets must be generated before building or testing")
    }
}
