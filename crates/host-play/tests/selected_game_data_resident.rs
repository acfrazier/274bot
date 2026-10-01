//! Separate test process: no other test may warm the selected-data OnceLock or
//! add resident pages while its one-time load is measured.

#[cfg(target_os = "macos")]
#[test]
fn selected_289_facts_do_not_retain_an_inflated_source_copy() {
    let before = host_play::current_resident_bytes().expect("current process RSS before facts");
    let facts =
        api::game_data::for_revision(client::io::ClientRevision::R289).expect("selected 289 facts");
    let pin = facts.selected_pin().expect("original-byte selected pin");
    assert_eq!(pin.revision, client::io::ClientRevision::R289);
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
