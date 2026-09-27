//! Redirect fd 2 while the TUI alternate screen is active.
//!
//! Client worker threads `eprintln!` cache reuse notes. Those writes scroll the
//! real cursor and corrupt Ratatui's differential frame. Capture them into
//! a truncated `~/.274bot/tui-stderr.log` (current run only) and restore fd 2
//! before a TUI-thread panic or a normal exit.

use std::ffi::CString;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

const STDERR_FD: i32 = 2;
static SAVED: AtomicI32 = AtomicI32::new(-1);
static ACTIVE: AtomicBool = AtomicBool::new(false);

unsafe fn sys_dup(fd: i32) -> i32 {
    libc::dup(fd)
}

unsafe fn sys_dup2(src: i32, dst: i32) -> i32 {
    libc::dup2(src, dst)
}

unsafe fn sys_close(fd: i32) -> i32 {
    libc::close(fd)
}

fn open_log_fd(path: &Path) -> io::Result<i32> {
    let c = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o644,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

/// Point stderr at `~/.274bot/tui-stderr.log`. Idempotent.
pub fn capture() {
    if ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = io::stderr().flush();
    let path = script::bot_file("tui-stderr.log");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let log_fd = match open_log_fd(&path) {
        Ok(fd) => fd,
        Err(_) => {
            ACTIVE.store(false, Ordering::SeqCst);
            return;
        }
    };
    unsafe {
        let saved = sys_dup(STDERR_FD);
        if saved < 0 {
            sys_close(log_fd);
            ACTIVE.store(false, Ordering::SeqCst);
            return;
        }
        if sys_dup2(log_fd, STDERR_FD) < 0 {
            sys_close(saved);
            sys_close(log_fd);
            ACTIVE.store(false, Ordering::SeqCst);
            return;
        }
        sys_close(log_fd);
        SAVED.store(saved, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// Put the original stderr fd back. Idempotent. Safe to call from a panic hook.
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let saved = SAVED.swap(-1, Ordering::SeqCst);
    if saved < 0 {
        return;
    }
    let _ = io::stderr().flush();
    unsafe {
        sys_dup2(saved, STDERR_FD);
        sys_close(saved);
    }
}
