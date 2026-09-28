//! Owner-only persistence helpers for the vault and the state files beside it.
//!
//! [`write_private_file`] is the one writer: an exclusive same-directory temp
//! file created `0o600`, flushed, then renamed over the target, so a reader
//! never sees a torn file, a leftover temp can never redirect the write, and
//! the umask cannot widen the mode. [`read_private_file`] is the matching
//! reader for state that later decides what the host loads or runs.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

/// Writes `data` to `path` atomically. On Unix the parent directory is created
/// `0o700` when missing and the file is `0o600` from the moment it exists.
///
/// The temp file is `path` with the extension `tmp`. It is created with
/// `create_new`, which fails on any existing name instead of following it, so a
/// planted symlink cannot make the write land elsewhere; a leftover plain file
/// or symlink from a writer that died is replaced, a directory is never
/// touched. On failure the temp file is removed and `path` is unchanged.
pub fn write_private_file(path: &Path, data: &[u8]) -> Result<(), io::Error> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        ensure_private_dir(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = create_temp(&tmp)?;
    let written = file.write_all(data).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written.and_then(|()| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    Ok(())
}

fn create_temp(tmp: &Path) -> io::Result<File> {
    match open_exclusive(tmp) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            match fs::symlink_metadata(tmp) {
                Ok(meta) if meta.is_dir() => return Err(error),
                Ok(_) => fs::remove_file(tmp)?,
                Err(gone) if gone.kind() == io::ErrorKind::NotFound => {}
                Err(other) => return Err(other),
            }
            open_exclusive(tmp)
        }
        other => other,
    }
}

fn open_exclusive(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

#[cfg(unix)]
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// Reads a persisted state file whose contents later decide what the host
/// loads or runs (the script library, the catalog root), refusing one another
/// user could have written.
///
/// Refused with a clear error: anything that is not a regular file (a FIFO or
/// device would hang or never end), a file larger than `max_bytes`, and, on
/// Unix, a file owned by neither the current user nor root or writable by its
/// group or by others. A file that others can merely read (state written before
/// the `0o600` writer) is tightened to `0o600` in place and read. Checks run on
/// the opened file, not the path, so the file cannot be swapped after the
/// check. A missing file is `NotFound`, which callers treat as a first run.
///
/// Windows has no Unix mode bits here, so only the regular-file and size checks
/// apply; the same holds on filesystems that report every file as `0o777`.
pub fn read_private_file(path: &Path, max_bytes: u64) -> io::Result<String> {
    let file = open_for_check(path)?;
    let meta = file.metadata()?;
    #[cfg(unix)]
    match store_file_verdict(
        &StoreFacts {
            is_file: meta.is_file(),
            mode: meta.mode(),
            owner: meta.uid(),
            len: meta.len(),
        },
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() },
        max_bytes,
    ) {
        Verdict::Accept => {}
        // Only group/other read bits: the state is not secret, so a failed
        // tighten (read-only mount) does not stop the read.
        Verdict::Tighten => {
            let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
        }
        Verdict::Refuse(reason) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{}: {reason}", path.display()),
            ));
        }
    }
    #[cfg(not(unix))]
    if !meta.is_file() || meta.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{}: not a regular file of at most {max_bytes} bytes",
                path.display()
            ),
        ));
    }
    let mut text = String::new();
    // `take` also bounds a file that grows after the size check.
    file.take(max_bytes.saturating_add(1))
        .read_to_string(&mut text)?;
    if text.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: larger than {max_bytes} bytes", path.display()),
        ));
    }
    Ok(text)
}

/// Opens without blocking on a FIFO (the type check follows) or acquiring a
/// controlling terminal, following symlinks on purpose: the checks below apply
/// to whatever the link resolves to.
fn open_for_check(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    options.open(path)
}

#[cfg(unix)]
struct StoreFacts {
    is_file: bool,
    mode: u32,
    owner: u32,
    len: u64,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Accept,
    Tighten,
    Refuse(String),
}

#[cfg(unix)]
fn store_file_verdict(facts: &StoreFacts, euid: u32, max_bytes: u64) -> Verdict {
    if !facts.is_file {
        return Verdict::Refuse("is not a regular file".into());
    }
    if facts.owner != euid && facts.owner != 0 {
        return Verdict::Refuse(format!(
            "is owned by another user (uid {}); refusing state it may have written",
            facts.owner
        ));
    }
    if facts.mode & 0o022 != 0 {
        return Verdict::Refuse(format!(
            "is writable by other users (mode {:04o}); refusing state they may have \
             written. If you wrote it, run `chmod 600` on it",
            facts.mode & 0o7777
        ));
    }
    if facts.len > max_bytes {
        return Verdict::Refuse(format!("is larger than {max_bytes} bytes"));
    }
    if facts.mode & 0o044 != 0 {
        return Verdict::Tighten;
    }
    Verdict::Accept
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::PathBuf;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("274bot-private-file-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn a_planted_temp_symlink_is_replaced_not_followed() {
        let dir = scratch("symlink");
        let victim = dir.join("victim");
        fs::write(&victim, "precious").unwrap();
        let target = dir.join("state.json");
        symlink(&victim, target.with_extension("tmp")).unwrap();

        write_private_file(&target, b"new state").unwrap();

        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        assert_eq!(fs::read_to_string(&target).unwrap(), "new state");
        assert!(
            !fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the target must be a regular file, not a link to the victim"
        );
    }

    #[test]
    fn a_wide_leftover_temp_or_target_never_widens_the_result() {
        let dir = scratch("wide");
        let target = dir.join("state.json");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o666)).unwrap();
        let tmp = target.with_extension("tmp");
        fs::write(&tmp, "stale").unwrap();
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o666)).unwrap();

        write_private_file(&target, b"fresh").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "fresh");
        assert_eq!(mode_of(&target), 0o600);
        assert!(!tmp.exists(), "no temp file survives a successful write");
    }

    #[test]
    fn a_failed_write_leaves_the_target_and_no_temp_behind() {
        let dir = scratch("fail");
        // Renaming a file over a directory fails on every Unix.
        let target = dir.join("state.json");
        fs::create_dir(&target).unwrap();

        assert!(write_private_file(&target, b"data").is_err());

        assert!(target.is_dir(), "the target is untouched");
        assert!(!target.with_extension("tmp").exists());
    }

    #[test]
    fn a_directory_at_the_temp_name_fails_the_write_and_survives() {
        let dir = scratch("blocker");
        let target = dir.join("state.json");
        fs::write(&target, "old").unwrap();
        let blocker = target.with_extension("tmp");
        fs::create_dir(&blocker).unwrap();

        assert!(write_private_file(&target, b"new").is_err());

        assert!(blocker.is_dir(), "a directory is never removed");
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
    }

    #[test]
    fn read_refuses_state_others_can_write_and_names_the_fix() {
        let dir = scratch("writable");
        for mode in [0o666, 0o664, 0o622] {
            let path = dir.join(format!("state-{mode:o}"));
            fs::write(&path, "[]").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();

            let error = read_private_file(&path, 1024).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{mode:o}");
            assert!(error.to_string().contains("chmod 600"), "{error}");
            assert_eq!(mode_of(&path), mode, "a refused file is left untouched");
        }
    }

    #[test]
    fn read_tightens_state_written_before_the_owner_only_writer() {
        let dir = scratch("legacy");
        let path = dir.join("state.json");
        fs::write(&path, "[1]").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(read_private_file(&path, 1024).unwrap(), "[1]");

        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn read_passes_a_missing_file_through_as_not_found() {
        let dir = scratch("missing");
        let error = read_private_file(&dir.join("absent"), 1024).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn read_refuses_a_fifo_without_blocking_and_a_directory() {
        let dir = scratch("special");
        let fifo = dir.join("pipe");
        let c_path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: c_path is a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);

        let error = read_private_file(&fifo, 1024).unwrap_err();
        assert!(error.to_string().contains("not a regular file"), "{error}");
        let error = read_private_file(&dir, 1024).unwrap_err();
        assert!(error.to_string().contains("not a regular file"), "{error}");
    }

    #[test]
    fn read_bounds_the_size_at_the_boundary() {
        let dir = scratch("size");
        let path = dir.join("state.json");
        fs::write(&path, "12345").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(read_private_file(&path, 5).unwrap(), "12345");
        let error = read_private_file(&path, 4).unwrap_err();
        assert!(error.to_string().contains("larger than 4"), "{error}");
    }

    #[test]
    fn owner_rule_accepts_the_current_user_and_root_and_refuses_anyone_else() {
        let facts = |owner| StoreFacts {
            is_file: true,
            mode: 0o600,
            owner,
            len: 1,
        };
        assert_eq!(store_file_verdict(&facts(501), 501, 10), Verdict::Accept);
        assert_eq!(store_file_verdict(&facts(0), 501, 10), Verdict::Accept);
        assert!(matches!(
            store_file_verdict(&facts(502), 501, 10),
            Verdict::Refuse(reason) if reason.contains("another user")
        ));
    }
}
