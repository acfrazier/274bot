//! Bank-pick work qualification without the lib-test allocation counter.
//!
//! This target calls the shared native harness and keeps the product allocator.
//! The CPU replay and live burst use the same production BankPickWorkerJob.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX, LIVE_EVIDENCE_DIR, local 289 assets and Engine A on 127.0.0.1:44594/1080"]
fn live_ten_guest_shared_bank_pick_full_retry_burst() {
    host_play::bank_npc_live::live_ten_guest_shared_bank_pick_full_retry_burst();
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a captured live Select input, its real navpack, and an output path"]
fn measures_captured_native_bank_pick_cpu() {
    host_play::measures_captured_native_bank_pick_cpu();
}
