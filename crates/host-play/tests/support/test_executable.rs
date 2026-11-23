use std::io;
use std::path::PathBuf;

/// Use the running image even if another Cargo invocation relinks its path.
pub(crate) fn executable() -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        // A PID also works when a shell launches the child on our behalf.
        Ok(PathBuf::from(format!("/proc/{}/exe", std::process::id())))
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::env::current_exe()
    }
}
