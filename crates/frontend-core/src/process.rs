//! Process-wide facts the front ends read.

/// Whether host debug diagnostics are on (`BOT_DEBUG`).
pub fn debug_enabled() -> bool {
    host::debug_enabled()
}
