use super::*;

#[test]
fn policy_digest_tracks_bank_bytes_revision_and_algorithm_not_pack() {
    let a = policy_digest(289, b"zones-a");
    let b = policy_digest(289, b"zones-b");
    let c = policy_digest(274, b"zones-a");
    assert_ne!(a, b);
    assert_ne!(a, c);
    let pack = [0x11u8; 32];
    assert_ne!(header_binding(&pack, &a), header_binding(&pack, &b));
    assert_eq!(header_binding(&pack, &a), header_binding(&[0x11u8; 32], &a));
    let other_pack = [0x22u8; 32];
    assert_ne!(header_binding(&pack, &a), header_binding(&other_pack, &a));
}

#[test]
fn expected_header_binding_roundtrips_hex_and_rejects_malformed() {
    let pack = [0xABu8; 32];
    let policy = policy_digest(289, b"row");
    let binding = header_binding(&pack, &policy);
    let got = expected_header_binding(
        &crate::pack::sha256_hex(&pack),
        &crate::pack::sha256_hex(&policy),
    )
    .unwrap();
    assert_eq!(got, binding);
    assert!(expected_header_binding("zz", &"aa".repeat(32)).is_err());
    assert!(expected_header_binding(&"aa".repeat(32), "short").is_err());
}
