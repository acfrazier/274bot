use scenario::evidence::{ReceiptEvent, ReceiptManifest, RunReceipt};

#[test]
fn receipt_state_size_tripwire() {
    // Measured receipt state (168/144/168 B) plus the default 64 B struct headroom.
    let event_bytes = std::mem::size_of::<ReceiptEvent>();
    let manifest_bytes = std::mem::size_of::<ReceiptManifest>();
    let receipt_bytes = std::mem::size_of::<RunReceipt>();
    assert!(
        event_bytes <= 232,
        "receipt event uses {event_bytes} bytes, ceiling 232"
    );
    assert!(
        manifest_bytes <= 208,
        "receipt manifest uses {manifest_bytes} bytes, ceiling 208"
    );
    assert!(
        receipt_bytes <= 232,
        "run receipt uses {receipt_bytes} bytes, ceiling 232"
    );
}
