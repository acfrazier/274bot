use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// Process- and test-unique scratch directory removed even when a test unwinds.
pub(crate) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub(crate) fn new(label: &str) -> Self {
        let serial = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "274bot-panel-{label}-{}-{serial}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// A file path that keeps its unique parent scratch directory alive.
pub(crate) struct TestPath {
    _dir: TestDir,
    path: PathBuf,
}

impl TestPath {
    pub(crate) fn new(label: &str, file_name: &str) -> Self {
        let dir = TestDir::new(label);
        let path = dir.join(file_name);
        Self { _dir: dir, path }
    }
}

impl Deref for TestPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TestPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            if let Ok(metadata) = std::fs::metadata(&self.path) {
                let mut permissions = metadata.permissions();
                permissions.set_mode(0o700);
                let _ = std::fs::set_permissions(&self.path, permissions);
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Test guards protect resettable process globals, so a failed assertion does
/// not make the mutex's poison bit a second, unrelated test failure.
pub(crate) fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Dear ImGui's context singleton is dropped before this guard during unwind;
/// recovering poison is therefore safe once the failed test has unwound.
/// Outermost in the test lock order (see `picker::lock_nav_statics`): taking
/// it while holding a nav lock panics instead of deadlocking.
pub(crate) fn imgui_context_guard() -> MutexGuard<'static, ()> {
    assert!(
        !crate::picker::holds_nav_locks(),
        "lock order: imgui_context_guard taken while holding a picker nav lock"
    );
    lock_unpoisoned(&crate::IMGUI_CTX_TEST_GUARD)
}
