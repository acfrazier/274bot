//! Owner-only persistence helpers for the vault and the state files beside it.
//!
//! [`write_private_file`] replaces a file and [`create_private_file`] creates
//! one that must not exist yet. Both stage the bytes in a **unique**,
//! exclusively created temp file `0o600` in the target's directory, flush it,
//! and publish it by an atomic name operation, so a reader never sees a torn
//! file, no other writer's temp can be confused with this one, and the umask
//! cannot widen the mode. [`read_private_file`] is the matching reader for
//! state that later decides what the host loads or runs.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use rand_core::{OsRng, RngCore};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

/// Longest prefix of the target's file name kept in a temp name, so the temp
/// name stays inside the 255-byte component limit however long the target is.
const TEMP_NAME_PREFIX_MAX: usize = 100;
/// Fresh names tried before giving up when every candidate already exists.
const TEMP_NAME_ATTEMPTS: usize = 16;

/// Whether `value` is one safe path component under the state directory:
/// non-empty, not `.` or `..`, and only ASCII letters, digits, `.`, `_` and
/// `-`. Server-profile names and vault components meet this rule when the
/// profile list loads, and a state file keyed by such a name (the bank hint's
/// `<profile>` directory) checks the same rule rather than its own.
pub fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Writes `data` to `path`, replacing any file there, atomically: a reader (or
/// a crash) sees the old bytes or the new bytes, never a mix.
///
/// Every write stages its bytes in its own temp file, `.<name>.<random>.tmp`,
/// created with `create_new` (so it can never be an existing name, a planted
/// symlink or a directory) and `0o600` from the moment it exists. This call
/// removes only the temp it created, and only while the name still refers to
/// that same file; before publishing it checks that it does, and returns an
/// error instead of acknowledging bytes that are not its own. The target
/// directory is created `0o700` when missing. On Unix it must be owned by this
/// user or root and not writable by group or others: a directory this user
/// owns that is group/other-writable (an older or `umask 002` install) is
/// tightened to `mode & !0o022` on first publication, then re-checked; one
/// owned by anyone else that is writable by others is refused, and a sticky
/// directory such as `/tmp` is accepted as it is (others cannot remove or
/// rename this user's entries in it). That check is what keeps the temp bound
/// to this writer between the check and the rename; a failing one is
/// `PermissionDenied`.
///
/// Writers to the same target inside one process are serialized, so the file
/// left behind is the one whose call returned last. Processes are not
/// serialized against each other: each publication is still atomic and each
/// writer's `Ok` means its own bytes were published, but a later writer
/// replaces an earlier one (last writer wins), so a read-modify-write across
/// processes needs its own lock. A writer that dies leaves its temp file
/// behind; it is never reused or removed by a later call.
///
/// Windows: the temp is unique, exclusive and opened with sharing denied while
/// it is written, but there are no mode bits to check and this crate does not
/// inspect or set ACLs, so the directory's inherited ACL is what protects the
/// file.
pub fn write_private_file(path: &Path, data: &[u8]) -> Result<(), io::Error> {
    publish(path, data, Publish::Replace)
}

/// Creates `path` with `data`, failing with [`io::ErrorKind::AlreadyExists`]
/// (and leaving whatever is there untouched) when anything, including a
/// dangling symlink, already has that name. Same staging and directory rules
/// as [`write_private_file`]. Of any number of concurrent callers, in this
/// process or another, exactly one succeeds.
///
/// The staged file is linked to its final name, which the filesystem refuses
/// when the name exists and never replaces. On a filesystem without hard
/// links the final name is instead claimed with `create_new` and written in
/// place: still exclusive, but a crash mid-write leaves a truncated file
/// where a new file was being created (no earlier data is at risk).
pub fn create_private_file(path: &Path, data: &[u8]) -> Result<(), io::Error> {
    publish(path, data, Publish::CreateNew)
}

#[derive(Clone, Copy)]
enum Publish {
    Replace,
    CreateNew,
}

fn publish(path: &Path, data: &[u8], mode: Publish) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{}: not a file path", path.display()),
        )
    })?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure_private_dir(dir)?;
    serialized(fs::canonicalize(dir)?.join(name), || {
        publish_staged(dir, name, path, data, mode)
    })
}

fn publish_staged(
    dir: &Path,
    name: &OsStr,
    path: &Path,
    data: &[u8],
    mode: Publish,
) -> io::Result<()> {
    let staged = Staged::write(dir, name, data)?;
    if !staged.is_ours() {
        // The name now belongs to someone else: leave it alone.
        return Err(io::Error::other(format!(
            "{}: the staging file was replaced while it was written; nothing was published",
            staged.path.display()
        )));
    }
    let published = match mode {
        Publish::Replace => fs::rename(&staged.path, path),
        Publish::CreateNew => link_without_replacing(&staged, path, data),
    };
    if published.is_err() || matches!(mode, Publish::CreateNew) {
        staged.discard();
    }
    published
}

/// Publishes the staged file at `path` unless something is there. The staged
/// name is left for the caller to discard.
fn link_without_replacing(staged: &Staged, path: &Path, data: &[u8]) -> io::Result<()> {
    match fs::hard_link(&staged.path, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(error),
        // No hard links here (FAT/exFAT, some network filesystems): claim the
        // final name exclusively and write it in place.
        Err(_) => {
            let mut file = open_exclusive(path)?;
            let claimed = Identity::of(&file)?;
            if let Err(error) = file.write_all(data).and_then(|()| file.sync_all()) {
                drop(file);
                if claimed.names(path) {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
            Ok(())
        }
    }
}

/// A temp file this call created, with the identity it had when created.
struct Staged {
    path: PathBuf,
    id: Identity,
}

impl Staged {
    /// Creates a fresh temp beside the target and fills and flushes it. Any
    /// failure removes the temp again before returning.
    fn write(dir: &Path, name: &OsStr, data: &[u8]) -> io::Result<Self> {
        let prefix: String = name
            .to_string_lossy()
            .chars()
            .take(TEMP_NAME_PREFIX_MAX)
            .collect();
        let mut last = None;
        for _ in 0..TEMP_NAME_ATTEMPTS {
            let path = dir.join(format!(".{prefix}.{:016x}.tmp", OsRng.next_u64()));
            match open_exclusive(&path) {
                Ok(mut file) => {
                    let staged = Staged {
                        id: Identity::of(&file)?,
                        path,
                    };
                    return match file.write_all(data).and_then(|()| file.sync_all()) {
                        Ok(()) => Ok(staged),
                        Err(error) => {
                            drop(file);
                            staged.discard();
                            Err(error)
                        }
                    };
                }
                // Somebody else's name: never touch it, try another.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
                Err(error) => return Err(error),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("no free temp file name")))
    }

    /// Whether the temp name still refers to the file this call created.
    fn is_ours(&self) -> bool {
        self.id.names(&self.path)
    }

    /// Removes the temp name if, and only if, it is still this call's file.
    fn discard(&self) {
        if self.is_ours() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Which file a name refers to: device and inode on Unix. There is no stable
/// equivalent in `std` on Windows, where the check always holds and the
/// unique name plus the sharing-denied handle stand in for it.
#[cfg(unix)]
struct Identity {
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
impl Identity {
    fn of(file: &File) -> io::Result<Self> {
        let meta = file.metadata()?;
        Ok(Self {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }

    fn names(&self, path: &Path) -> bool {
        fs::symlink_metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.dev() == self.dev && meta.ino() == self.ino)
    }
}

#[cfg(not(unix))]
struct Identity;

#[cfg(not(unix))]
impl Identity {
    fn of(_file: &File) -> io::Result<Self> {
        Ok(Self)
    }

    fn names(&self, path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
    }
}

fn open_exclusive(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    // Nobody may open, rename or delete the temp while it is being written.
    #[cfg(windows)]
    options.share_mode(0);
    options.open(path)
}

type TargetLocks = HashMap<PathBuf, Arc<Mutex<()>>>;

static TARGET_LOCKS: Mutex<Option<TargetLocks>> = Mutex::new(None);

/// Runs `work` while holding the lock for `key`, so writers to one target
/// within this process take turns.
fn serialized<T>(key: PathBuf, work: impl FnOnce() -> T) -> T {
    let lock = TARGET_LOCKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .entry(key.clone())
        .or_default()
        .clone();
    let result = {
        let _turn = lock.lock().unwrap_or_else(PoisonError::into_inner);
        work()
    };
    let mut locks = TARGET_LOCKS.lock().unwrap_or_else(PoisonError::into_inner);
    // Only the map and this call hold the lock: nobody is waiting on it.
    if Arc::strong_count(&lock) == 2 {
        if let Some(map) = locks.as_mut() {
            map.remove(&key);
        }
    }
    result
}

/// Creates `dir` `0o700` when missing and makes it fit for staging: on Unix a
/// directory this user owns that group or others can write to (not sticky) is
/// tightened once to `mode & !0o022`, on the opened directory itself, then
/// re-checked; anything still unfit is refused.
#[cfg(unix)]
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    // `recursive` accepts a directory that already exists, so there is no
    // exists-then-create window.
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    let handle = File::open(dir).ok();
    let mut meta = match &handle {
        Some(handle) => handle.metadata()?,
        None => fs::metadata(dir)?,
    };
    if let Some(handle) = &handle {
        if let Some(mode) = tightened_dir_mode(&facts_of(&meta), euid) {
            // A failure leaves the mode as it was, and the verdict below
            // refuses it with the reason.
            if handle
                .set_permissions(fs::Permissions::from_mode(mode))
                .is_ok()
            {
                meta = handle.metadata()?;
            }
        }
    }
    match dir_verdict(&facts_of(&meta), euid) {
        Ok(()) => Ok(()),
        Err(reason) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: {reason}", dir.display()),
        )),
    }
}

#[cfg(unix)]
fn facts_of(meta: &fs::Metadata) -> DirFacts {
    DirFacts {
        is_dir: meta.is_dir(),
        mode: meta.mode(),
        owner: meta.uid(),
    }
}

/// Creates `dir` (and any missing parents) owner-only where the platform has
/// modes, without judging an existing directory. For the application's own
/// root, so a first creation by the host is never wider than the vault's.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

#[cfg(not(unix))]
fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

#[cfg(unix)]
struct DirFacts {
    is_dir: bool,
    mode: u32,
    owner: u32,
}

/// The mode to tighten a directory to before staging in it, if it needs it:
/// this user owns it, group or others can write it and it is not sticky.
/// Foreign-owned directories and root-owned ones (when this user is not root)
/// are never touched; the verdict refuses them.
#[cfg(unix)]
fn tightened_dir_mode(facts: &DirFacts, euid: u32) -> Option<u32> {
    let sticky = facts.mode & 0o1000 != 0;
    (facts.is_dir && facts.owner == euid && facts.mode & 0o022 != 0 && !sticky)
        .then_some(facts.mode & 0o7777 & !0o022)
}

/// Whether private state may be staged and published in a directory: owned by
/// this user or root, and not one others can add, remove or rename entries in
/// (group/other write), unless it is sticky, where they can only touch their
/// own entries.
#[cfg(unix)]
fn dir_verdict(facts: &DirFacts, euid: u32) -> Result<(), String> {
    if !facts.is_dir {
        return Err("is not a directory".into());
    }
    if facts.owner != euid && facts.owner != 0 {
        return Err(format!(
            "is owned by another user (uid {}); refusing to keep private state in it",
            facts.owner
        ));
    }
    let sticky = facts.mode & 0o1000 != 0;
    if facts.mode & 0o022 != 0 && !sticky {
        return Err(format!(
            "is writable by other users (mode {:04o}); they could replace the file \
             while it is being written. Run `chmod go-w` on it",
            facts.mode & 0o7777
        ));
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
/// Windows has no Unix mode bits here and this crate reads no ACLs, so only the
/// regular-file and size checks apply there. On Unix a filesystem that reports
/// every file as `0o777` is refused like any other group/other-writable file.
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
    check_regular_and_size(path, &meta, max_bytes)?;
    read_bounded(file, path, max_bytes)
}

/// Reads a file the caller only needs to be a regular file of bounded size (a
/// script source: it lives wherever the operator keeps it, including
/// filesystems that show every file world-writable, so there is no owner or
/// mode check). Opened once without blocking; the shape and size are checked
/// on that open file and the read is bounded, so a swap after the check cannot
/// substitute a FIFO or an endless file. Refusals are `PermissionDenied`; a
/// missing file is `NotFound`.
pub fn read_regular_file(path: &Path, max_bytes: u64) -> io::Result<String> {
    let file = open_for_check(path)?;
    check_regular_and_size(path, &file.metadata()?, max_bytes)?;
    read_bounded(file, path, max_bytes)
}

fn check_regular_and_size(path: &Path, meta: &fs::Metadata, max_bytes: u64) -> io::Result<()> {
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: not a regular file", path.display()),
        ));
    }
    if meta.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: larger than {max_bytes} bytes", path.display()),
        ));
    }
    Ok(())
}

fn read_bounded(file: File, path: &Path, max_bytes: u64) -> io::Result<String> {
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

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

    /// Every name in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn valid_component_accepts_profile_names_and_refuses_traversal() {
        for accepted in ["local-289", "a.b_c-d", "x", &"a".repeat(65)] {
            assert!(valid_component(accepted), "{accepted:?}");
        }
        for refused in ["", ".", "..", "a/b", "a\\b", "a b", "a\0b", "../x", "é"] {
            assert!(!valid_component(refused), "{refused:?}");
        }
    }

    #[test]
    fn a_symlink_at_the_target_is_replaced_never_written_through() {
        let dir = scratch("symlink");
        let victim = dir.join("victim");
        fs::write(&victim, "precious").unwrap();
        let target = dir.join("state.json");
        symlink(&victim, &target).unwrap();

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
    fn a_wide_target_never_widens_the_result_and_no_temp_survives() {
        let dir = scratch("wide");
        let target = dir.join("state.json");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o666)).unwrap();

        write_private_file(&target, b"fresh").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "fresh");
        assert_eq!(mode_of(&target), 0o600);
        assert_eq!(names(&dir), ["state.json"], "no temp file survives");
    }

    #[test]
    fn a_failed_publish_leaves_the_target_and_no_temp_behind() {
        let dir = scratch("fail");
        // Renaming a file over a directory fails on every platform.
        let target = dir.join("state.json");
        fs::create_dir(&target).unwrap();

        assert!(write_private_file(&target, b"data").is_err());

        assert!(target.is_dir(), "the target is untouched");
        assert_eq!(names(&dir), ["state.json"], "the temp was removed");
    }

    #[test]
    fn a_long_file_name_still_gets_a_valid_temp_name() {
        let dir = scratch("long");
        let target = dir.join("n".repeat(250));

        write_private_file(&target, b"data").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"data");
    }

    #[test]
    fn concurrent_writers_each_succeed_and_the_target_is_one_whole_payload() {
        let dir = scratch("concurrent");
        let target = dir.join("state.json");
        fs::write(&target, "old").unwrap();
        let size = 512 * 1024;

        let writers: Vec<_> = (0..8u8)
            .map(|id| {
                let target = target.clone();
                thread::spawn(move || {
                    let payload = vec![b'a' + id; size];
                    for round in 0..8 {
                        write_private_file(&target, &payload)
                            .unwrap_or_else(|e| panic!("writer {id} round {round}: {e}"));
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }

        let bytes = fs::read(&target).unwrap();
        assert_eq!(bytes.len(), size, "a whole payload, not a torn one");
        assert!(bytes.iter().all(|b| *b == bytes[0]), "one writer's bytes");
        assert_eq!(names(&dir), ["state.json"], "no temp survives");
    }

    /// An actor that can already write the directory unlinks the writer's
    /// staging file mid-write and puts its own bytes at that name. The writer
    /// must not acknowledge those bytes as its own, and must not remove them.
    #[test]
    fn a_swapped_staging_file_is_never_acknowledged_as_the_writers_data() {
        let dir = scratch("swap");
        let target = dir.join("state.json");
        fs::write(&target, "old").unwrap();
        let payload = vec![b'W'; 32 * 1024 * 1024];

        let mut swaps = 0;
        for _ in 0..5 {
            let stop = Arc::new(AtomicBool::new(false));
            let attacker = {
                let (dir, stop) = (dir.clone(), stop.clone());
                thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        for entry in fs::read_dir(&dir).unwrap().flatten() {
                            let path = entry.path();
                            if path.extension().is_some_and(|e| e == "tmp") {
                                let _ = fs::remove_file(&path);
                                fs::write(&path, b"ATTACKER").unwrap();
                                return Some(path);
                            }
                        }
                    }
                    None
                })
            };
            let outcome = write_private_file(&target, &payload);
            stop.store(true, Ordering::Relaxed);
            let Some(planted) = attacker.join().unwrap() else {
                continue;
            };
            swaps += 1;

            match outcome {
                Ok(()) => assert!(
                    fs::read(&target).unwrap() == payload,
                    "acknowledged, but the target does not hold the writer's bytes"
                ),
                Err(_) => {
                    assert_eq!(fs::read(&target).unwrap(), b"old", "nothing published");
                    assert_eq!(
                        fs::read(&planted).unwrap(),
                        b"ATTACKER",
                        "the writer removed a file it did not create"
                    );
                }
            }
            let _ = fs::remove_file(&planted);
            fs::write(&target, "old").unwrap();
        }
        assert!(swaps > 0, "the attacker never got to swap a staging file");
    }

    #[test]
    fn a_wide_directory_this_user_owns_is_tightened_on_first_publication() {
        let dir = scratch("wide-dir");
        let target = dir.join("state.json");
        for (wide, tightened) in [
            (0o777, 0o755),
            (0o775, 0o755),
            (0o757, 0o755),
            (0o770, 0o750),
            (0o772, 0o750),
        ] {
            fs::set_permissions(&dir, fs::Permissions::from_mode(wide)).unwrap();

            write_private_file(&target, b"data").unwrap();

            assert_eq!(mode_of(&dir), tightened, "{wide:o}");
            assert_eq!(fs::read(&target).unwrap(), b"data", "{wide:o}");
            assert_eq!(names(&dir), ["state.json"], "{wide:o}: no temp left");
        }
    }

    #[test]
    fn a_sticky_directory_and_a_tight_one_are_left_as_they_are() {
        let dir = scratch("sticky-dir");
        let target = dir.join("state.json");
        // A sticky directory (like /tmp) only lets others touch their own entries.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o1777)).unwrap();
        write_private_file(&target, b"data").unwrap();
        assert_eq!(mode_of(&dir), 0o1777);
        for mode in [0o700, 0o755] {
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).unwrap();
            write_private_file(&target, b"data").unwrap();
            assert_eq!(mode_of(&dir), mode);
        }
    }

    #[test]
    fn only_a_wide_non_sticky_directory_of_the_current_user_is_tightened() {
        let facts = |mode, owner| DirFacts {
            is_dir: true,
            mode,
            owner,
        };
        assert_eq!(tightened_dir_mode(&facts(0o40775, 501), 501), Some(0o755));
        assert_eq!(tightened_dir_mode(&facts(0o40777, 501), 501), Some(0o755));
        assert_eq!(tightened_dir_mode(&facts(0o41777, 501), 501), None);
        assert_eq!(tightened_dir_mode(&facts(0o40755, 501), 501), None);
        // Somebody else's directory is never chmodded, and is refused.
        for owner in [0, 502] {
            assert_eq!(tightened_dir_mode(&facts(0o40775, owner), 501), None);
            let refusal = dir_verdict(&facts(0o40775, owner), 501).unwrap_err();
            assert!(
                refusal.contains("another user") || refusal.contains("chmod go-w"),
                "{refusal}"
            );
        }
        assert!(dir_verdict(&facts(0o41777, 502), 501)
            .unwrap_err()
            .contains("another user"));
    }

    #[test]
    fn creating_the_private_dir_is_owner_only_and_leaves_an_existing_one_alone() {
        let base = scratch("private-dir");
        let fresh = base.join("a").join("b");

        create_private_dir(&fresh).unwrap();

        assert_eq!(mode_of(&fresh), 0o700);
        assert_eq!(mode_of(&base.join("a")), 0o700);
        fs::set_permissions(&fresh, fs::Permissions::from_mode(0o775)).unwrap();
        create_private_dir(&fresh).unwrap();
        assert_eq!(mode_of(&fresh), 0o775);
    }

    #[test]
    fn directory_owner_rule_accepts_the_current_user_and_root_only() {
        let facts = |owner| DirFacts {
            is_dir: true,
            mode: 0o700,
            owner,
        };
        assert!(dir_verdict(&facts(501), 501).is_ok());
        assert!(dir_verdict(&facts(0), 501).is_ok());
        assert!(dir_verdict(&facts(502), 501)
            .unwrap_err()
            .contains("another user"));
    }

    #[test]
    fn create_refuses_anything_at_the_name_and_leaves_it_untouched() {
        let dir = scratch("create-exists");
        let existing = dir.join("vault");
        fs::write(&existing, "keep").unwrap();
        let dangling = dir.join("dangling");
        let elsewhere = dir.join("elsewhere");
        symlink(&elsewhere, &dangling).unwrap();

        for path in [&existing, &dangling] {
            let error = create_private_file(path, b"new").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{path:?}");
        }

        assert_eq!(fs::read_to_string(&existing).unwrap(), "keep");
        assert!(
            !elsewhere.exists(),
            "a dangling link is not written through"
        );
        assert_eq!(names(&dir), ["dangling", "vault"], "no temp survives");
    }

    #[test]
    fn of_many_concurrent_creators_exactly_one_wins_and_its_bytes_are_kept() {
        let dir = scratch("create-race");
        let target = dir.join("vault");
        let start = Arc::new(std::sync::Barrier::new(8));

        let creators: Vec<_> = (0..8u8)
            .map(|id| {
                let (target, start) = (target.clone(), start.clone());
                thread::spawn(move || {
                    start.wait();
                    create_private_file(&target, &[id; 4096]).map(|()| id)
                })
            })
            .collect();
        let outcomes: Vec<_> = creators.into_iter().map(|c| c.join().unwrap()).collect();

        let winners: Vec<u8> = outcomes
            .iter()
            .filter_map(|o| o.as_ref().ok())
            .copied()
            .collect();
        assert_eq!(winners.len(), 1, "{outcomes:?}");
        for lost in outcomes.iter().filter_map(|o| o.as_ref().err()) {
            assert_eq!(lost.kind(), io::ErrorKind::AlreadyExists);
        }
        assert_eq!(fs::read(&target).unwrap(), vec![winners[0]; 4096]);
        assert_eq!(names(&dir), ["vault"], "no temp survives");
        assert_eq!(mode_of(&target), 0o600);
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
