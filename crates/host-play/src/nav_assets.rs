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

/// Install resource root for an executable: macOS app `Contents/Resources`,
/// otherwise the executable's containing directory.
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

#[cfg(test)]
mod tests {
    use super::{artifact_layout, install_resource_root, local_asset_root, ArtifactLayout};
    use std::path::{Path, PathBuf};

    #[test]
    fn install_root_handles_mac_app_and_plain_binary() {
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
