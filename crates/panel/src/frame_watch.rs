//! Panel frame heartbeat. The watcher lives in `frontend-core` and is off
//! in release builds.

pub(crate) fn beat() {
    frontend_core::frame_watch::beat("panel");
}
