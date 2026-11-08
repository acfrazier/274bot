//! Separate test process: no other test may warm the selected-data OnceLock or
//! add resident pages while its one-time load is measured.

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn selected_289_facts_do_not_retain_an_inflated_source_copy() {
    if !api::test_support::game_data_available() {
        return;
    }
    let before = host_play::current_resident_bytes().expect("current process RSS before facts");
    let _facts =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected 289 facts");
    let after = host_play::current_resident_bytes().expect("current process RSS after facts");
    let growth = after.saturating_sub(before);
    eprintln!("selected 289 RSS: before={before} after={after} growth={growth}");
    // This covers the shared parsed tables, parser/index code and compact
    // embedded input, but not another multi-MiB inflated JSON representation.
    assert!(
        growth <= 8 * 1024 * 1024,
        "selected 289 facts grew RSS by {growth} bytes (limit: 8 MiB)"
    );
}
