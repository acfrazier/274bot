use std::sync::LazyLock;

static DRIVER: LazyLock<host_play::PlayComposition> = LazyLock::new(|| {
    host_play::PlayComposition::start().expect("test fixture starts its Play network composition")
});

/// Retain one composition for this integration-test process before fixtures
/// perform runtime cache preparation or call `Client::maininit` directly.
pub(crate) fn ensure_started() -> &'static host_play::PlayComposition {
    &DRIVER
}
