//! Operator `HOME` / `$RS2B0T` for `~/.274bot` paths.
//!
//! Unit tests must not `set_var("HOME")`. That is process-global: a
//! parallel test (or a panic before restore) rewrites the operator's
//! panel prefs, catalog path, JS store, settings, and loadouts.
//! [`IsolatedEnv`] pins those lookups to a unique scratch on **this
//! thread only** and restores on drop.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    static HOME_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static RS2B0T_OVERRIDE: RefCell<Option<Rs2b0tOverride>> = const { RefCell::new(None) };
    static THREAD_PIN: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

static SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
enum Rs2b0tOverride {
    Cleared,
    Set(PathBuf),
}

/// `$HOME`, or this thread's [`IsolatedEnv`] scratch when a test pinned one.
pub fn bot_home() -> PathBuf {
    if let Some(home) = HOME_OVERRIDE.with(|c| c.borrow().clone()) {
        return home;
    }
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => PathBuf::from(h),
        _ => PathBuf::from("."),
    }
}

/// `$RS2B0T` when set and non-empty. A live [`IsolatedEnv`] starts cleared
/// so a unit test cannot inherit the operator catalog.
pub fn rs2b0t_env() -> Option<PathBuf> {
    match RS2B0T_OVERRIDE.with(|c| c.borrow().clone()) {
        Some(Rs2b0tOverride::Cleared) => return None,
        Some(Rs2b0tOverride::Set(p)) => return Some(p),
        None => {}
    }
    match std::env::var("RS2B0T") {
        Ok(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => None,
    }
}

/// `{bot_home()}/.274bot/{name}`.
pub fn bot_file(name: &str) -> PathBuf {
    bot_home().join(".274bot").join(name)
}

/// Unique scratch `$HOME` (and optional `$RS2B0T`) for one test thread.
/// Drop restores the previous pin even on panic. Does **not** mutate the
/// process environment.
pub struct IsolatedEnv {
    pub dir: PathBuf,
    pub home: PathBuf,
    prev_home: Option<PathBuf>,
    prev_rs2b0t: Option<Rs2b0tOverride>,
}

impl IsolatedEnv {
    /// Pin this thread's `~/.274bot` away from the operator home for the
    /// rest of the thread (cargo reuses test threads). Catalog tests that
    /// need a clean empty home still call [`IsolatedEnv::enter`].
    pub fn ensure_thread() {
        THREAD_PIN.with(|pin| {
            if pin.borrow().is_some() {
                restore_thread_pin();
                return;
            }
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("274bot-test-thread-{n}"));
            let home = dir.join("home");
            let _ = std::fs::create_dir_all(&home);
            HOME_OVERRIDE.with(|c| {
                if c.borrow().is_none() {
                    *c.borrow_mut() = Some(home.clone());
                }
            });
            RS2B0T_OVERRIDE.with(|c| {
                if c.borrow().is_none() {
                    *c.borrow_mut() = Some(Rs2b0tOverride::Cleared);
                }
            });
            *pin.borrow_mut() = Some(home);
        });
    }

    pub fn enter(label: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("274bot-iso-{label}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let prev_home = HOME_OVERRIDE.with(|c| c.borrow().clone());
        let prev_rs2b0t = RS2B0T_OVERRIDE.with(|c| c.borrow().clone());
        HOME_OVERRIDE.with(|c| *c.borrow_mut() = Some(home.clone()));
        RS2B0T_OVERRIDE.with(|c| *c.borrow_mut() = Some(Rs2b0tOverride::Cleared));
        Self {
            dir,
            home,
            prev_home,
            prev_rs2b0t,
        }
    }

    pub fn set_rs2b0t(&self, root: &Path) {
        RS2B0T_OVERRIDE.with(|c| {
            *c.borrow_mut() = Some(Rs2b0tOverride::Set(root.to_path_buf()));
        });
    }

    pub fn clear_rs2b0t(&self) {
        RS2B0T_OVERRIDE.with(|c| {
            *c.borrow_mut() = Some(Rs2b0tOverride::Cleared);
        });
    }
}

impl Drop for IsolatedEnv {
    fn drop(&mut self) {
        HOME_OVERRIDE.with(|c| *c.borrow_mut() = self.prev_home.clone());
        RS2B0T_OVERRIDE.with(|c| *c.borrow_mut() = self.prev_rs2b0t.clone());
        restore_thread_pin();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn restore_thread_pin() {
    THREAD_PIN.with(|pin| {
        let Some(home) = pin.borrow().clone() else {
            return;
        };
        HOME_OVERRIDE.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(home);
            }
        });
        RS2B0T_OVERRIDE.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(Rs2b0tOverride::Cleared);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::{bot_file, bot_home, rs2b0t_env, IsolatedEnv};
    use std::path::Path;

    #[test]
    fn isolated_env_does_not_mutate_process_home() {
        let before_home = std::env::var("HOME").ok();
        let before_rs2b0t = std::env::var("RS2B0T").ok();
        {
            let iso = IsolatedEnv::enter("no-process");
            assert_eq!(std::env::var("HOME").ok(), before_home);
            assert_eq!(std::env::var("RS2B0T").ok(), before_rs2b0t);
            assert_eq!(bot_home(), iso.home);
            assert!(rs2b0t_env().is_none());
            iso.set_rs2b0t(Path::new("/tmp/does-not-need-to-exist"));
            assert_eq!(
                rs2b0t_env().as_deref(),
                Some(Path::new("/tmp/does-not-need-to-exist"))
            );
            assert_eq!(std::env::var("RS2B0T").ok(), before_rs2b0t);
        }
        assert_eq!(std::env::var("HOME").ok(), before_home);
        assert_eq!(std::env::var("RS2B0T").ok(), before_rs2b0t);
    }

    #[test]
    fn isolated_env_restores_bot_home_and_deletes_scratch() {
        let before = bot_home();
        let scratch;
        {
            let iso = IsolatedEnv::enter("restore");
            scratch = iso.dir.clone();
            assert_eq!(bot_home(), iso.home);
            assert_eq!(
                bot_file("js-scripts.json"),
                iso.home.join(".274bot/js-scripts.json")
            );
            assert!(rs2b0t_env().is_none(), "enter starts with no catalog env");
        }
        assert_eq!(bot_home(), before);
        assert!(!scratch.exists(), "drop must remove the unique scratch");
    }

    #[test]
    fn isolated_env_is_thread_local() {
        let iso = IsolatedEnv::enter("tls");
        let pinned = iso.home.clone();
        let other = std::thread::spawn(|| bot_home()).join().unwrap();
        assert_eq!(bot_home(), pinned);
        assert_ne!(
            other, pinned,
            "other threads keep process HOME, not this test's scratch"
        );
    }

    #[test]
    fn ensure_thread_does_not_use_operator_home() {
        IsolatedEnv::ensure_thread();
        let home = bot_home();
        if let Ok(op) = std::env::var("HOME") {
            assert_ne!(
                home,
                Path::new(&op),
                "unit tests must not read or write the operator ~/.274bot"
            );
        }
        assert!(
            rs2b0t_env().is_none(),
            "ensure_thread must not inherit $RS2B0T"
        );
        assert!(home.ends_with("home"));
    }

    #[test]
    fn ensure_thread_survives_isolated_env_drop() {
        IsolatedEnv::ensure_thread();
        let pinned = bot_home();
        {
            let _iso = IsolatedEnv::enter("after-pin");
            IsolatedEnv::ensure_thread();
            assert_ne!(bot_home(), pinned);
        }
        IsolatedEnv::ensure_thread();
        assert_eq!(
            bot_home(),
            pinned,
            "Session::new on a reused cargo thread must not fall through to operator HOME"
        );
        if let Ok(op) = std::env::var("HOME") {
            assert_ne!(bot_home(), Path::new(&op));
        }
        assert!(rs2b0t_env().is_none());
    }

    #[test]
    fn enter_then_ensure_thread_drop_keeps_the_thread_pin() {
        {
            let _iso = IsolatedEnv::enter("before-pin");
            IsolatedEnv::ensure_thread();
        }
        IsolatedEnv::ensure_thread();
        if let Ok(op) = std::env::var("HOME") {
            assert_ne!(
                bot_home(),
                Path::new(&op),
                "IsolatedEnv drop must not clear ensure_thread's pin"
            );
        }
        assert!(rs2b0t_env().is_none());
    }
}
