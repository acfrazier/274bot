use script::interact::InteractReq;
use script_js::wire::{decode_interact_batch, encode_interact_batch};

#[test]
fn key_gesture_round_trip_keeps_duration_and_production_identity() {
    for key in ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"] {
        for duration_ms in [1, 10000] {
            let bytes = encode_interact_batch(&[InteractReq::KeyGesture {
                key: key.into(),
                duration_ms,
                identity: 37,
            }]);
            let decoded = decode_interact_batch(&bytes).unwrap();
            assert!(
                matches!(decoded.as_slice(), [InteractReq::KeyGesture { key: actual, duration_ms: duration, identity: 37 }] if actual == key && *duration == duration_ms)
            );
        }
    }
}

#[test]
fn key_gesture_wire_refuses_unadmitted_keys_and_durations() {
    for (key, duration_ms) in [
        ("Enter", 1),
        ("ArrowLeft", 0),
        ("ArrowLeft", 10001),
        ("ArrowLeft", u32::MAX),
    ] {
        let bytes = encode_interact_batch(&[InteractReq::KeyGesture {
            key: key.into(),
            duration_ms,
            identity: 37,
        }]);
        assert!(
            decode_interact_batch(&bytes).is_err(),
            "{key} / {duration_ms}"
        );
    }
}
