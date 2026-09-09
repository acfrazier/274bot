//! Generated snapshot read regression, exercised with feature on and off.
use api::snapshot::GameSnapshot;

#[test]
fn census_does_not_mutate_or_retain_empty_snapshot() {
    let snapshot = GameSnapshot::new();
    let before = serde_json::to_value(&snapshot).unwrap();
    #[cfg(feature = "memory-owner-capture")]
    {
        let fragment = snapshot.owner_payload(&mut api::owner_capture::Budget::new());
        assert!(fragment.complete, "{:?}", fragment.reason);
        assert!(fragment.rows.len() <= api::owner_capture::MAX_FIELD_ROWS);
        assert!(fragment.rows.iter().any(|r| r.field == "inventory"));
        drop(fragment);
    }
    assert_eq!(serde_json::to_value(&snapshot).unwrap(), before);
}
