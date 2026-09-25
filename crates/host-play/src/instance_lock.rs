//! Process-lifetime advisory lock on `~/.274bot/instance.lock`.
//!
//! Uses [`std::fs::File::try_lock`] (POSIX `flock` / Windows `LockFileEx`).
//! The lock is released when this process exits or the handle drops.
//!
//! Holder identity (`panel|tui` + pid) is a separate plain file,
//! `~/.274bot/instance.holder`, published after the lock is taken (tmp +
//! fsync + rename) and removed when [`InstanceLock`] drops. Windows
//! `LockFileEx` is mandatory, so another process cannot read bytes covered
//! by the lock; a same-file marker would always look like `pid unknown`
//! there. A contested reader accepts only a complete newline-terminated
//! marker whose pid is still alive; anything else is retried for 500 ms,
//! then `pid unknown`. A crash can leave a marker; liveness rejects it.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Which frontend holds `~/.274bot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceKind {
    Panel,
    Tui,
}

impl InstanceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Tui => "tui",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Tui => "TUI",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "panel" => Some(Self::Panel),
            "tui" => Some(Self::Tui),
            _ => None,
        }
    }
}

/// Identity recorded in `instance.holder` by the process that holds
/// `instance.lock`. `pid`/`kind` are `None` when the OS lock is held but
/// the marker is missing, empty, partial, or unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceHolder {
    pub pid: Option<u32>,
    pub kind: Option<InstanceKind>,
}

impl InstanceHolder {
    pub fn unknown() -> Self {
        Self {
            pid: None,
            kind: None,
        }
    }
}

/// Exclusive lock held for the process lifetime.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(instance_holder_path());
    }
}

/// Outcome of an exclusive lock attempt. A held lock is always
/// [`Self::Contested`], even when the marker cannot be parsed.
#[derive(Debug)]
pub enum InstanceLockResult {
    Acquired(InstanceLock),
    Contested(InstanceHolder),
}

/// Proof that the caller already decided the instance lock: either this
/// process holds it, or a harness / Continue-anyway path skipped it.
#[derive(Debug)]
pub enum InstancePermit {
    Locked(InstanceLock),
    SkipLock,
}

impl InstancePermit {
    pub fn skip() -> Self {
        Self::SkipLock
    }

    pub fn locked(lock: InstanceLock) -> Self {
        Self::Locked(lock)
    }
}

/// Interactive acquire may need an operator confirm before a permit exists.
#[derive(Debug)]
pub enum InstancePermitOutcome {
    Ready(InstancePermit),
    NeedsConfirm(InstanceHolder),
}

/// `~/.274bot/instance.lock` — exclusive OS lock, not the pid/kind marker.
pub fn instance_lock_path() -> PathBuf {
    script::bot_file("instance.lock")
}

/// `~/.274bot/instance.holder` — plain `kind pid` line, always readable.
fn instance_holder_path() -> PathBuf {
    script::bot_file("instance.holder")
}

pub fn instance_conflict_message(holder: &InstanceHolder) -> String {
    let lock = instance_lock_path();
    let dir = lock
        .parent()
        .expect("instance.lock lives in the bot directory")
        .display();
    match (holder.kind, holder.pid) {
        (Some(kind), Some(pid)) => format!(
            "Another 274bot {} (pid {}) is using {} — running both can overwrite each other's settings",
            kind.label(),
            pid,
            dir
        ),
        _ => format!(
            "Another 274bot instance (pid unknown) is using {} — running both can overwrite each other's settings",
            dir
        ),
    }
}

/// Harness / live / memory paths pass `skip`. Interactive paths pass `false`
/// and either receive a held lock or a confirm.
pub fn resolve_instance_permit(
    kind: InstanceKind,
    skip: bool,
) -> io::Result<InstancePermitOutcome> {
    if skip {
        return Ok(InstancePermitOutcome::Ready(InstancePermit::SkipLock));
    }
    match try_acquire_instance_lock(kind)? {
        InstanceLockResult::Acquired(lock) => {
            Ok(InstancePermitOutcome::Ready(InstancePermit::Locked(lock)))
        }
        InstanceLockResult::Contested(holder) => Ok(InstancePermitOutcome::NeedsConfirm(holder)),
    }
}

/// Take the exclusive instance lock, or report who holds it.
pub fn try_acquire_instance_lock(kind: InstanceKind) -> io::Result<InstanceLockResult> {
    let path = instance_lock_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => {
            write_holder(kind)?;
            Ok(InstanceLockResult::Acquired(InstanceLock { _file: file }))
        }
        Err(TryLockError::WouldBlock) => Ok(InstanceLockResult::Contested(read_holder_retry())),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

const MARKER_WAIT: Duration = Duration::from_millis(500);
const MARKER_STEP: Duration = Duration::from_millis(20);

fn write_holder(kind: InstanceKind) -> io::Result<()> {
    let dest = instance_holder_path();
    let tmp = script::bot_file(&format!("instance.holder.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)?;
    let synced = (|| {
        writeln!(file, "{} {}", kind.as_str(), std::process::id())?;
        file.sync_all()
    })();
    drop(file);
    if let Err(e) = synced {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // `rename` replaces an existing marker on every platform (Windows uses
    // MOVEFILE_REPLACE_EXISTING), so a failure here is a real error.
    if let Err(err) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

fn read_holder_retry() -> InstanceHolder {
    let deadline = Instant::now() + MARKER_WAIT;
    loop {
        if let Some(holder) = read_holder_once() {
            return holder;
        }
        if Instant::now() >= deadline {
            return InstanceHolder::unknown();
        }
        std::thread::sleep(MARKER_STEP);
    }
}

fn read_holder_once() -> Option<InstanceHolder> {
    let text = std::fs::read_to_string(instance_holder_path()).ok()?;
    let holder = parse_holder(&text)?;
    let pid = holder.pid?;
    pid_alive(pid).then_some(holder)
}

/// Complete marker: `kind pid` plus a terminating newline, nothing after.
fn parse_holder(text: &str) -> Option<InstanceHolder> {
    let (line, rest) = text.split_once('\n')?;
    if !rest.is_empty() {
        return None;
    }
    let mut parts = line.split_whitespace();
    let kind = InstanceKind::parse(parts.next()?)?;
    let pid = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(InstanceHolder {
        pid: Some(pid),
        kind: Some(kind),
    })
}

/// Unix `kill(pid, 0)` (exists if success or `EPERM`); Windows `OpenProcess`
/// and `STILL_ACTIVE`. Justified unsafe: the same OS probes host-play already
/// uses for RSS/CPU. No `cfg(windows)` beyond this helper.
fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(raw) = i32::try_from(pid) else {
            return false;
        };
        if raw <= 0 {
            return false;
        }
        if unsafe { libc::kill(raw as libc::pid_t, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        if pid == 0 {
            return false;
        }
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Command, Stdio};
    use std::sync::{Mutex, MutexGuard};

    fn lock_test_guard() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn skip_permit_does_not_create_the_lock_file() {
        let _iso = script::IsolatedEnv::enter("instance-permit-skip");
        match resolve_instance_permit(InstanceKind::Panel, true).unwrap() {
            InstancePermitOutcome::Ready(InstancePermit::SkipLock) => {}
            other => panic!("{other:?}"),
        }
        assert!(!instance_lock_path().exists());
        assert!(!instance_holder_path().exists());
    }

    #[test]
    fn lock_is_reacquired_after_the_holder_drops() {
        let _guard = lock_test_guard();
        let _iso = script::IsolatedEnv::enter("instance-lock-drop");
        let first = try_acquire_instance_lock(InstanceKind::Panel).unwrap();
        assert!(matches!(first, InstanceLockResult::Acquired(_)));
        assert!(instance_holder_path().is_file());
        drop(first);
        assert!(!instance_holder_path().exists());
        let second = try_acquire_instance_lock(InstanceKind::Tui).unwrap();
        match second {
            InstanceLockResult::Acquired(_) => {}
            InstanceLockResult::Contested(h) => panic!("lock survived drop: {h:?}"),
        }
    }

    #[test]
    fn contested_empty_marker_is_unknown_not_an_error() {
        let _guard = lock_test_guard();
        let iso = script::IsolatedEnv::enter("instance-lock-empty");
        let path = instance_lock_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let holder = spawn_holder("empty", &iso.home);
        match try_acquire_instance_lock(InstanceKind::Tui).unwrap() {
            InstanceLockResult::Contested(h) => {
                assert_eq!(h, InstanceHolder::unknown());
                assert!(
                    instance_conflict_message(&h).contains("pid unknown"),
                    "{}",
                    instance_conflict_message(&h)
                );
            }
            InstanceLockResult::Acquired(_) => panic!("expected contest"),
        }
        holder.reap();
        let _iso = iso;
    }

    #[test]
    fn contested_stale_complete_marker_is_unknown() {
        let _guard = lock_test_guard();
        let iso = script::IsolatedEnv::enter("instance-lock-stale");
        std::fs::create_dir_all(instance_lock_path().parent().unwrap()).unwrap();
        std::fs::write(instance_holder_path(), "panel 424242\n").unwrap();
        let holder = spawn_holder("empty", &iso.home);
        match try_acquire_instance_lock(InstanceKind::Tui).unwrap() {
            InstanceLockResult::Contested(h) => {
                assert_eq!(h, InstanceHolder::unknown());
                assert_ne!(h.pid, Some(424242));
            }
            InstanceLockResult::Acquired(_) => panic!("expected contest"),
        }
        holder.reap();
        let _iso = iso;
    }

    #[test]
    fn contested_partial_marker_is_unknown() {
        let _guard = lock_test_guard();
        let iso = script::IsolatedEnv::enter("instance-lock-partial");
        std::fs::create_dir_all(instance_lock_path().parent().unwrap()).unwrap();
        std::fs::write(instance_holder_path(), "tui 12").unwrap();
        let holder = spawn_holder("empty", &iso.home);
        match try_acquire_instance_lock(InstanceKind::Tui).unwrap() {
            InstanceLockResult::Contested(h) => {
                assert_eq!(h, InstanceHolder::unknown());
                assert_ne!(h.pid, Some(12));
            }
            InstanceLockResult::Acquired(_) => panic!("expected contest"),
        }
        holder.reap();
        let _iso = iso;
    }

    #[test]
    fn second_acquirer_sees_holder_pid_and_kind() {
        let _guard = lock_test_guard();
        let iso = script::IsolatedEnv::enter("instance-lock-hold");
        let holder = spawn_holder("panel", &iso.home);
        let pid = holder.pid;
        let marker = std::fs::read_to_string(instance_holder_path()).expect("plain holder");
        assert!(
            marker.contains(&pid.to_string()) && marker.contains("panel") && marker.ends_with('\n'),
            "{marker:?}"
        );
        match try_acquire_instance_lock(InstanceKind::Tui).unwrap() {
            InstanceLockResult::Contested(h) => {
                assert_eq!(h.pid, Some(pid));
                assert_eq!(h.kind, Some(InstanceKind::Panel));
                assert!(instance_conflict_message(&h).contains(&pid.to_string()));
            }
            InstanceLockResult::Acquired(_) => panic!("expected contest"),
        }
        holder.reap();
        match try_acquire_instance_lock(InstanceKind::Tui).unwrap() {
            InstanceLockResult::Acquired(_) => {}
            InstanceLockResult::Contested(h) => panic!("child drop left the lock: {h:?}"),
        }
        let _iso = iso;
    }

    #[test]
    #[ignore = "lock-holder child"]
    fn hold_instance_lock_until_stdin_closes() {
        let mode = std::env::var("274BOT_LOCK_HOLDER").unwrap_or_default();
        let path = instance_lock_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        file.try_lock().expect("child lock");
        if mode != "empty" {
            let kind = if mode == "tui" {
                InstanceKind::Tui
            } else {
                InstanceKind::Panel
            };
            write_holder(kind).unwrap();
        }
        println!("holder-ready {}", std::process::id());
        let mut buf = [0u8; 1];
        let _ = std::io::stdin().read(&mut buf);
        drop(file);
    }

    struct HolderProc {
        child: std::process::Child,
        stdout: BufReader<std::process::ChildStdout>,
        pid: u32,
    }

    impl HolderProc {
        fn reap(mut self) {
            drop(self.child.stdin.take());
            let mut stdout = self.stdout.into_inner();
            let _ = std::io::copy(&mut stdout, &mut std::io::sink());
            let status = self.child.wait().expect("wait holder");
            assert!(status.success(), "holder child {status:?}");
        }
    }

    fn spawn_holder(mode: &str, home: &std::path::Path) -> HolderProc {
        let exe = std::env::current_exe().unwrap();
        let mut child = Command::new(exe)
            .args([
                "--ignored",
                "--exact",
                "--nocapture",
                "instance_lock::tests::hold_instance_lock_until_stdin_closes",
            ])
            .env("274BOT_LOCK_HOLDER", mode)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        let pid = loop {
            line.clear();
            let n = stdout.read_line(&mut line).expect("holder line");
            assert!(n > 0, "holder-ready");
            if let Some(rest) = line.trim_end().strip_prefix("holder-ready ") {
                break rest.parse().unwrap();
            }
        };
        HolderProc { child, stdout, pid }
    }
}
