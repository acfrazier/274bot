use scenario::evidence::{ReceiptEvent, ReceiptManifest, RunReceipt};

#[test]
fn receipt_state_size_tripwire() {
    // Pin the receipt structs, not ScenarioRunner fields that vary by build feature.
    assert_eq!(
        std::mem::size_of::<ReceiptEvent>(),
        168,
        "receipt event bytes"
    );
    assert_eq!(
        std::mem::size_of::<ReceiptManifest>(),
        144,
        "receipt manifest bytes"
    );
    assert_eq!(std::mem::size_of::<RunReceipt>(), 168, "run receipt bytes");
}
