//! Optional per-session log file under `~/.274bot/logs/`.
//!
//! Off by default; the operator turns it on with the shared
//! `session_log_file` preference in `panel-ui.json` (absent = off, so every
//! existing prefs file keeps the old behaviour). While on, the store hands
//! each formatted line to a bounded queue drained by one writer thread; a
//! full queue drops lines (counted and reported in the file) instead of
//! blocking a slot thread. The writer rotates by size and prunes old
//! sessions, so disk use stays bounded.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;

use crate::log::{global, logs_dir, LocalTime};
use api::hostlog::{Level, Source};

/// `panel-ui.json` key of the shared on/off preference.
pub const SESSION_LOG_KEY: &str = "session_log_file";

/// Size rotation and retention for session files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rotation {
    /// Rotate the live file once it reaches this many bytes.
    pub max_bytes: u64,
    /// Rotated segments kept per session (`.log.1` … `.log.N`).
    pub segments: usize,
    /// Sessions kept in the directory, this one included.
    pub sessions: usize,
}

/// 4 MiB × (1 live + 3 rotated) per session, 10 sessions: ≤ 160 MiB.
pub const DEFAULT_ROTATION: Rotation = Rotation {
    max_bytes: 4 * 1024 * 1024,
    segments: 3,
    sessions: 10,
};

/// Lines queued for the writer before new ones are dropped.
const QUEUE_LINES: usize = 4_096;

/// Handle to a running session writer. Dropping it closes the queue; the
/// writer flushes what it holds and exits without being joined.
#[derive(Debug)]
pub struct SessionLogFile {
    tx: SyncSender<Box<str>>,
    path: PathBuf,
    dropped: Arc<AtomicU64>,
}

impl SessionLogFile {
    /// Start a writer for a new session file in `dir`
    /// (`session-<YYYYmmdd-HHMMSS>-<pid>.log`). Directory creation, pruning
    /// and the open happen on the writer thread; a failure is reported to
    /// the process log.
    pub fn start(dir: PathBuf, rotation: Rotation) -> Self {
        let name = format!(
            "session-{}-{}.log",
            LocalTime::now().stamp(),
            std::process::id()
        );
        let path = dir.join(name);
        let (tx, rx) = mpsc::sync_channel(QUEUE_LINES);
        let dropped = Arc::new(AtomicU64::new(0));
        let writer = Writer {
            dir,
            path: path.clone(),
            rotation,
            dropped: Arc::clone(&dropped),
        };
        let spawned = std::thread::Builder::new()
            .name("session-log".into())
            .spawn(move || {
                if let Err(e) = writer.run(rx) {
                    global().process_line(
                        Source::Host,
                        Level::Error,
                        &format!("session log {}: {e}", writer.path.display()),
                    );
                }
            });
        if let Err(e) = spawned {
            global().process_line(
                Source::Host,
                Level::Error,
                &format!("session log: writer thread: {e}"),
            );
        }
        Self { tx, path, dropped }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Queue one formatted line; never blocks.
    pub(crate) fn send(&self, line: &str) {
        match self.tx.try_send(line.into()) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

struct Writer {
    dir: PathBuf,
    path: PathBuf,
    rotation: Rotation,
    dropped: Arc<AtomicU64>,
}

impl Writer {
    fn run(&self, rx: Receiver<Box<str>>) -> io::Result<()> {
        create_private_dir(&self.dir)?;
        prune_sessions(&self.dir, self.rotation.sessions.saturating_sub(1))?;
        let mut out = BufWriter::new(open_private(&self.path)?);
        let mut size = 0u64;
        loop {
            let line = match rx.try_recv() {
                Ok(line) => line,
                Err(TryRecvError::Empty) => {
                    out.flush()?;
                    match rx.recv() {
                        Ok(line) => line,
                        Err(_) => break,
                    }
                }
                Err(TryRecvError::Disconnected) => break,
            };
            let lost = self.dropped.swap(0, Ordering::Relaxed);
            if lost > 0 {
                let note = format!("… {lost} line(s) dropped: the log writer fell behind\n");
                out.write_all(note.as_bytes())?;
                size += note.len() as u64;
            }
            out.write_all(line.as_bytes())?;
            out.write_all(b"\n")?;
            size += line.len() as u64 + 1;
            if size >= self.rotation.max_bytes {
                out.flush()?;
                drop(out);
                rotate(&self.path, self.rotation.segments)?;
                out = BufWriter::new(open_private(&self.path)?);
                size = 0;
            }
        }
        out.flush()
    }
}

fn segment(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

/// `x.log` → `x.log.1`, `x.log.1` → `x.log.2` …; the oldest past
/// `segments` is deleted.
fn rotate(path: &Path, segments: usize) -> io::Result<()> {
    if segments == 0 {
        return remove_if_exists(path);
    }
    remove_if_exists(&segment(path, segments))?;
    for n in (1..segments).rev() {
        rename_if_exists(&segment(path, n), &segment(path, n + 1))?;
    }
    rename_if_exists(path, &segment(path, 1))
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Keep the newest `keep` sessions' files (names sort by start time).
fn prune_sessions(dir: &Path, keep: usize) -> io::Result<()> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(stem) = session_stem(&name) {
            files.push((stem.to_string(), entry.path()));
        }
    }
    let mut stems: Vec<&str> = files.iter().map(|(s, _)| s.as_str()).collect();
    stems.sort_unstable();
    stems.dedup();
    let cut = stems.len().saturating_sub(keep);
    let old: Vec<String> = stems[..cut].iter().map(|s| s.to_string()).collect();
    for (stem, path) in &files {
        if old.iter().any(|o| o == stem) {
            remove_if_exists(path)?;
        }
    }
    Ok(())
}

/// `session-<stamp>-<pid>` for a live or rotated session file name.
fn session_stem(name: &str) -> Option<&str> {
    if !name.starts_with("session-") {
        return None;
    }
    let end = name.find(".log")?;
    let rest = &name[end + 4..];
    (rest.is_empty() || (rest.starts_with('.') && rest[1..].bytes().all(|b| b.is_ascii_digit())))
        .then(|| &name[..end])
}

fn open_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// The shared preference: `session_log_file` in `panel-ui.json`, off when
/// the file or the key is absent (every 0.1.8.1 prefs file).
pub fn session_log_setting() -> bool {
    session_log_setting_at(&host_play::panel_ui_path())
}

pub fn session_log_setting_at(prefs: &Path) -> bool {
    std::fs::read(prefs)
        .ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        .and_then(|v| v.get(SESSION_LOG_KEY)?.as_bool())
        .unwrap_or(false)
}

/// Store the preference, keeping every other key in the file.
pub fn persist_session_log_setting(on: bool) -> io::Result<()> {
    persist_session_log_setting_at(&host_play::panel_ui_path(), on)
}

pub fn persist_session_log_setting_at(prefs: &Path, on: bool) -> io::Result<()> {
    let mut value = match std::fs::read(prefs) {
        Ok(data) => {
            serde_json::from_slice(&data).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?
        }
        Err(e) if e.kind() == ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e),
    };
    let Some(obj) = value.as_object_mut() else {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("{} is not a JSON object", prefs.display()),
        ));
    };
    obj.insert(SESSION_LOG_KEY.into(), serde_json::Value::Bool(on));
    let data =
        serde_json::to_vec_pretty(&value).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    vault::write_private_file(prefs, &data)
}

/// Open (on) or close (off) this process's session file on the global
/// store. Idempotent; returns the open file's path.
pub fn apply_session_log(on: bool) -> Option<PathBuf> {
    let store = global();
    match (on, store.file_path()) {
        (true, Some(path)) => Some(path),
        (true, None) => {
            let file = SessionLogFile::start(logs_dir(), DEFAULT_ROTATION);
            let path = file.path().to_path_buf();
            store.set_file(Some(file));
            Some(path)
        }
        (false, _) => {
            store.set_file(None);
            None
        }
    }
}

#[cfg(test)]
#[path = "log_file_tests.rs"]
mod tests;
