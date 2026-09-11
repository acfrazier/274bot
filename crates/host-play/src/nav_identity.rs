//! Compile-time bundled navigation identities and install-path selection.
//!
//! The checked-in table is empty until a packager fills it. An empty table,
//! `--nav-pack` / `NAV_PACK`, or a user-authored sidecar never selects the
//! bundled path. `--release` is not a product class.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// One shipped navpack identity, analogue of `known-cache-identities.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundledNavIdentity {
    pub revision: u16,
    pub cache_id: String,
    pub format: String,
    pub nav_sha256: String,
    pub flags_sha256: Option<String>,
    pub relative_path: String,
}

/// Origin of the loaded navigation pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavOrigin {
    Bundled {
        identity: BundledNavIdentity,
        path: PathBuf,
    },
    External {
        path: PathBuf,
    },
}

impl NavOrigin {
    pub fn path(&self) -> &Path {
        match self {
            Self::Bundled { path, .. } | Self::External { path } => path,
        }
    }

    pub fn is_bundled(&self) -> bool {
        matches!(self, Self::Bundled { .. })
    }

    pub fn bundled_identity(&self) -> Option<&BundledNavIdentity> {
        match self {
            Self::Bundled { identity, .. } => Some(identity),
            Self::External { .. } => None,
        }
    }
}

/// Read/hash/decode counts for one origin-aware load. Tests use these instead
/// of restating progress labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NavLoadCounters {
    pub pack_reads: u32,
    pub pack_hashes: u32,
    pub pack_decodes: u32,
}

/// Compile-time table. Empty in git until release assets are assembled.
pub fn bundled_nav_identities() -> &'static [BundledNavIdentity] {
    static TABLE: OnceLock<Vec<BundledNavIdentity>> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("bundled-nav-identities.json"))
            .expect("bundled nav identities")
    })
}

/// Resource root for install-relative navpacks: macOS app
/// `Contents/Resources`, otherwise the directory containing the binary.
pub fn install_resource_root(exe: &Path) -> PathBuf {
    let Some(parent) = exe.parent() else {
        return PathBuf::from(".");
    };
    if parent.file_name().is_some_and(|name| name == "MacOS") {
        if let Some(contents) = parent.parent() {
            if contents.file_name().is_some_and(|name| name == "Contents") {
                return contents.join("Resources");
            }
        }
    }
    parent.to_path_buf()
}

/// Select bundled vs external origin. A matching compiled row plus no pack
/// override is required for bundle trust; otherwise the resolved path is
/// external and hashed at load.
pub fn select_nav_origin(
    table: &[BundledNavIdentity],
    resource_root: Option<&Path>,
    revision: u16,
    cache_id: &str,
    pack_overridden: bool,
    external_path: &Path,
) -> Result<NavOrigin, String> {
    if pack_overridden || table.is_empty() {
        return Ok(NavOrigin::External {
            path: external_path.to_path_buf(),
        });
    }
    let mut matched = None;
    for identity in table {
        if identity.revision == revision && identity.cache_id == cache_id {
            if matched.is_some() {
                return Err(format!(
                    "duplicate bundled navigation identity for revision {revision}"
                ));
            }
            matched = Some(identity);
        }
    }
    let Some(identity) = matched else {
        return Ok(NavOrigin::External {
            path: external_path.to_path_buf(),
        });
    };
    let root = resource_root.ok_or_else(|| {
        "bundled navigation identity table requires an install resource root".to_string()
    })?;
    let path = contained_pack_path(root, &identity.relative_path)?;
    validate_identity_shape(identity)?;
    Ok(NavOrigin::Bundled {
        identity: identity.clone(),
        path,
    })
}

fn validate_identity_shape(identity: &BundledNavIdentity) -> Result<(), String> {
    if identity.format != "274V8" {
        return Err(format!(
            "bundled navigation format {} is unsupported; expected 274V8",
            identity.format
        ));
    }
    if identity.nav_sha256.len() != 64
        || !identity.nav_sha256.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err("bundled navigation identity nav_sha256 is not a SHA-256 hex digest".into());
    }
    if let Some(flags) = &identity.flags_sha256 {
        if flags.len() != 64 || !flags.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(
                "bundled navigation identity flags_sha256 is not a SHA-256 hex digest".into(),
            );
        }
    }
    match identity.revision {
        274 | 289 => Ok(()),
        revision => Err(format!("unsupported revision {revision}; use 274 or 289")),
    }
}

fn contained_pack_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() {
        return Err("bundled navigation relative_path is empty".into());
    }
    let rel = Path::new(relative);
    if rel.is_absolute() {
        return Err("bundled navigation relative_path must be install-relative".into());
    }
    for component in rel.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(
                    "bundled navigation relative_path must stay under the install resource root"
                        .into(),
                );
            }
        }
    }
    Ok(root.join(rel))
}

#[cfg(test)]
mod tests {
    use super::{
        bundled_nav_identities, contained_pack_path, install_resource_root, select_nav_origin,
        BundledNavIdentity, NavOrigin,
    };
    use std::path::{Path, PathBuf};

    fn identity(relative_path: &str) -> BundledNavIdentity {
        BundledNavIdentity {
            revision: 289,
            cache_id: "cache".into(),
            format: "274V8".into(),
            nav_sha256: "ab".repeat(32),
            flags_sha256: None,
            relative_path: relative_path.into(),
        }
    }

    #[test]
    fn compiled_table_is_empty_until_packaging() {
        assert!(bundled_nav_identities().is_empty());
    }

    #[test]
    fn mac_app_and_ordinary_binary_roots() {
        assert_eq!(
            install_resource_root(Path::new("/Applications/274bot.app/Contents/MacOS/274bot")),
            PathBuf::from("/Applications/274bot.app/Contents/Resources")
        );
        assert_eq!(
            install_resource_root(Path::new("/opt/274bot/274bot")),
            PathBuf::from("/opt/274bot")
        );
    }

    #[test]
    fn empty_table_or_override_stays_external() {
        let path = Path::new("/tmp/custom.navpack");
        let table = [identity("274bot.navpack")];
        assert_eq!(
            select_nav_origin(&[], Some(Path::new("/app")), 289, "cache", false, path).unwrap(),
            NavOrigin::External {
                path: path.to_path_buf()
            }
        );
        assert_eq!(
            select_nav_origin(&table, Some(Path::new("/app")), 289, "cache", true, path).unwrap(),
            NavOrigin::External {
                path: path.to_path_buf()
            }
        );
    }

    #[test]
    fn matching_row_selects_contained_install_path() {
        let table = [identity("nav/274bot.navpack")];
        let origin = select_nav_origin(
            &table,
            Some(Path::new("/app/Resources")),
            289,
            "cache",
            false,
            Path::new("/home/.274bot/274bot.navpack"),
        )
        .unwrap();
        match origin {
            NavOrigin::Bundled { path, identity } => {
                assert_eq!(path, PathBuf::from("/app/Resources/nav/274bot.navpack"));
                assert_eq!(identity.relative_path, "nav/274bot.navpack");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parent_dir_and_absolute_paths_are_rejected() {
        let err = contained_pack_path(Path::new("/app"), "../escape.navpack").unwrap_err();
        assert!(err.contains("stay under the install resource root"));
        let err = contained_pack_path(Path::new("/app"), "/etc/passwd").unwrap_err();
        assert!(err.contains("install-relative"));
        let table = [identity("../escape.navpack")];
        assert!(select_nav_origin(
            &table,
            Some(Path::new("/app")),
            289,
            "cache",
            false,
            Path::new("/tmp/x")
        )
        .is_err());
    }

    #[test]
    fn unknown_format_or_duplicate_row_is_rejected() {
        let mut bad = identity("274bot.navpack");
        bad.format = "274V7".into();
        assert!(select_nav_origin(
            &[bad],
            Some(Path::new("/app")),
            289,
            "cache",
            false,
            Path::new("/tmp/x")
        )
        .unwrap_err()
        .contains("274V8"));
        let table = [identity("a.navpack"), identity("b.navpack")];
        assert!(select_nav_origin(
            &table,
            Some(Path::new("/app")),
            289,
            "cache",
            false,
            Path::new("/tmp/x")
        )
        .unwrap_err()
        .contains("duplicate"));
    }
}
