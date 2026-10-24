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

#[test]
fn same_sized_sidecar_with_new_bank_policy_has_a_new_binding() {
    let pack = [0xCDu8; 32];
    let old_policy = policy_digest(289, b"old-zones");
    let new_policy = policy_digest(289, b"new-zones");
    assert_eq!(old_policy.len(), new_policy.len());
    let old = header_binding(&pack, &old_policy);
    let new = header_binding(&pack, &new_policy);
    assert_ne!(old, new);
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let bits = [0u64; 1];
    let a = crate::pack::encode_canlight_sidecar(origin, 2, 1, &bits, &old);
    let b = crate::pack::encode_canlight_sidecar(origin, 2, 1, &bits, &new);
    assert_eq!(a.len(), b.len());
    let da = crate::pack::decode_canlight_sidecar(&a).unwrap();
    let expected_new = expected_header_binding(
        &crate::pack::sha256_hex(&pack),
        &crate::pack::sha256_hex(&new_policy),
    )
    .unwrap();
    assert_ne!(da.binding, expected_new);
}
