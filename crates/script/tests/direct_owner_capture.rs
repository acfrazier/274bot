//! Public SlotScript ownership boundary, with and without capture.
use script::SlotScript;

#[test]
fn stop_keeps_builder_unknown_and_drops_fingerprint() {
    let mut slot = SlotScript::new();
    slot.stop();
    assert_eq!(slot.state(), script::RunState::Idle);
    #[cfg(feature = "memory-owner-capture")]
    {
        let fragment = slot.owner_payload(&mut api::owner_capture::Budget::new());
        assert!(fragment.complete);
        assert_eq!(fragment.fingerprint_present, Some(false));
        let builder = fragment
            .rows
            .iter()
            .find(|r| r.field == "ipc_builder_capacity")
            .unwrap();
        assert_eq!(builder.capacity_bytes, None);
        assert_eq!(builder.reason, api::owner_capture::Reason::OpaqueUnknown);
        assert_eq!(fragment.script_state, Some("Idle"));
    }
}
