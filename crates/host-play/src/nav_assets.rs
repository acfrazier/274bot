//! Local navigation asset paths shared by the build script and runtime.

use std::path::{Component, Path, PathBuf};

/// Install-relative layout of generated navigation payloads for one revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactLayout {
    pub dir: PathBuf,
    pub relative_pack: PathBuf,
    pub relative_flags: PathBuf,
    pub relative_reach: PathBuf,
    pub relative_canlight: PathBuf,
    pub relative_pois: PathBuf,
}

/// `nav/<revision>/…` keeps local navigation assets for each revision separate.
pub fn artifact_layout(revision: u16) -> ArtifactLayout {
    let dir = PathBuf::from("nav").join(revision.to_string());
    ArtifactLayout {
        relative_pack: dir.join("274bot.navpack"),
        relative_flags: dir.join("274bot.navflags"),
        relative_reach: dir.join("274bot.navreach"),
        relative_canlight: dir.join("274bot.navcanlight"),
        relative_pois: dir.join("274bot.navpois"),
        dir,
    }
}

/// Default local source root used by the navigation generator and host build.
pub fn local_asset_root(workspace_root: &Path) -> PathBuf {
    workspace_root.join("target/local-data")
}

/// Stage a payload without changing a source that is already at its destination.
pub fn stage_payload(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source_path = source.canonicalize()?;
    match destination.canonicalize() {
        Ok(destination_path) if destination_path == source_path => return Ok(()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    std::fs::copy(source, destination)?;
    let mut permissions = std::fs::metadata(destination)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    std::fs::set_permissions(destination, permissions)
}

/// Resource root for staged navigation payloads: `BOT_NAV_RESOURCE_DIR` when
/// set, otherwise the Cargo profile directory containing ordinary binaries.
pub fn resource_root_for_build(
    out_dir: &Path,
    override_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(dir) = override_dir {
        if dir.as_os_str().is_empty() {
            return Err("BOT_NAV_RESOURCE_DIR is empty".into());
        }
        if dir.is_absolute() {
            return Ok(dir.to_path_buf());
        }
        if dir
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(format!(
                "resource directory {} must not escape its root",
                dir.display()
            ));
        }
        return std::env::current_dir()
            .map(|cwd| cwd.join(dir))
            .map_err(|error| format!("working directory: {error}"));
    }
    let build_dir = out_dir
        .parent()
        .and_then(Path::parent)
        .filter(|build| build.file_name().is_some_and(|name| name == "build"))
        .ok_or_else(|| {
            format!(
                "unexpected build script output layout {}",
                out_dir.display()
            )
        })?;
    build_dir.parent().map(Path::to_path_buf).ok_or_else(|| {
        format!(
            "unexpected build script output layout {}",
            out_dir.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{artifact_layout, local_asset_root, stage_payload, ArtifactLayout};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "274bot-nav-stage-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("temporary staging fixture root");
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn staging_copies_new_and_existing_destinations() {
        let root = TempRoot::new();
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"navigation payload").unwrap();
        for _ in 0..2 {
            stage_payload(&source, &destination).unwrap();
            assert_eq!(std::fs::read(&source).unwrap(), b"navigation payload");
            assert_eq!(std::fs::read(&destination).unwrap(), b"navigation payload");
            std::fs::write(&destination, b"old").unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn staging_read_only_source_twice_keeps_destination_user_writable() {
        use std::os::unix::fs::PermissionsExt;

        let root = TempRoot::new();
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"navigation payload").unwrap();
        let mut permissions = std::fs::metadata(&source).unwrap().permissions();
        permissions.set_mode(0o444);
        std::fs::set_permissions(&source, permissions).unwrap();

        for _ in 0..2 {
            stage_payload(&source, &destination).unwrap();
            let source_mode = std::fs::metadata(&source).unwrap().permissions().mode();
            let destination_mode = std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(source_mode & 0o777, 0o444, "source stays read-only");
            assert_ne!(
                destination_mode & 0o200,
                0,
                "staged destination stays user-writable"
            );
            assert_eq!(std::fs::read(&destination).unwrap(), b"navigation payload");
        }
    }

    #[test]
    fn staging_same_path_preserves_payload() {
        let root = TempRoot::new();
        let source = root.0.join("payload");
        std::fs::write(&source, b"navigation payload").unwrap();
        stage_payload(&source, &source).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), b"navigation payload");
    }

    #[cfg(unix)]
    #[test]
    fn staging_aliased_roots_preserves_all_five_payloads() {
        let root = TempRoot::new();
        let source_root = root.0.join("source");
        let resource_root = root.0.join("resources");
        let payloads = root.0.join("payloads");
        let relative = Path::new("nav/289");
        std::fs::create_dir_all(&payloads).unwrap();
        for staging_root in [&source_root, &resource_root] {
            std::fs::create_dir_all(staging_root.join("nav")).unwrap();
            std::os::unix::fs::symlink(&payloads, staging_root.join(relative)).unwrap();
        }
        for extension in ["navpack", "navflags", "navreach", "navcanlight", "navpois"] {
            let name = format!("274bot.{extension}");
            let source = source_root.join(relative).join(&name);
            let destination = resource_root.join(relative).join(&name);
            std::fs::write(&source, name.as_bytes()).unwrap();
            stage_payload(&source, &destination).unwrap();
            assert_eq!(std::fs::read(source).unwrap(), name.as_bytes());
        }
    }

    #[test]
    fn layout_and_local_source_root_are_revision_keyed() {
        let layout = artifact_layout(289);
        assert_eq!(
            layout,
            ArtifactLayout {
                dir: PathBuf::from("nav/289"),
                relative_pack: PathBuf::from("nav/289/274bot.navpack"),
                relative_flags: PathBuf::from("nav/289/274bot.navflags"),
                relative_reach: PathBuf::from("nav/289/274bot.navreach"),
                relative_canlight: PathBuf::from("nav/289/274bot.navcanlight"),
                relative_pois: PathBuf::from("nav/289/274bot.navpois"),
            }
        );
        assert_eq!(
            local_asset_root(Path::new("/repo")),
            PathBuf::from("/repo/target/local-data")
        );
    }
}
