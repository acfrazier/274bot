#![cfg(all(
    feature = "test-support",
    any(target_os = "windows", target_os = "macos")
))]

//! Direct platform-verifier checks for the test-only extra-root constructor.
//! Worker handoff and WSS traffic are exercised by rustls_backend_real.rs.
//! These checks do not count verifier entry across transfer.

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::client::danger::ServerCertVerifier;
use rustls::crypto::ring::default_provider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls_platform_verifier::Verifier;
use std::sync::Arc;

fn subject(common_name: &str) -> DistinguishedName {
    let mut subject = DistinguishedName::new();
    subject.push(DnType::CommonName, common_name);
    subject
}

fn ca(name: &str) -> (Certificate, KeyPair, CertificateDer<'static>) {
    let key = KeyPair::generate().expect("generate throwaway CA key");
    let mut params = CertificateParams::new(vec![name.to_owned()]).expect("create CA parameters");
    // Each certificate needs its own subject name. With rcgen's shared default
    // subject, Windows CryptoAPI reads a leaf as self-signed and rejects the
    // chain (CERT_E_SIGNATURE, os error -2146869244).
    params.distinguished_name = subject(name);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let cert = params.self_signed(&key).expect("self-sign throwaway CA");
    let der = cert.der().clone();
    (cert, key, der)
}

fn leaf(issuer: &Certificate, issuer_key: &KeyPair, expired: bool) -> CertificateDer<'static> {
    let key = KeyPair::generate().expect("generate leaf key");
    let mut params =
        CertificateParams::new(vec!["localhost".to_owned()]).expect("create leaf parameters");
    params.distinguished_name = subject("platform trust test leaf");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    if expired {
        params.not_after = params.not_before;
    }
    params
        .signed_by(&key, issuer, issuer_key)
        .expect("sign test leaf")
        .der()
        .clone()
}

fn verify(verifier: &Verifier, certificate: &CertificateDer<'static>, name: &str) -> bool {
    let name = ServerName::try_from(name.to_owned()).expect("valid test DNS name");
    verifier
        .verify_server_cert(certificate, &[], &name, &[], UnixTime::now())
        .is_ok()
}

#[test]
fn platform_verifier_augments_native_trust_and_rejects_invalid_identity() {
    let (test_ca, test_key, test_root) = ca("platform trust test root");
    let (other_ca, other_key, other_root) = ca("unrelated platform trust root");
    let valid = leaf(&test_ca, &test_key, false);
    let expired = leaf(&test_ca, &test_key, true);

    // An empty extra-root set retains the platform's ordinary root loading.
    let native_only = Verifier::new_with_extra_roots(
        Vec::<CertificateDer<'static>>::new(),
        Arc::new(default_provider()),
    )
    .expect("construct verifier with native roots only");
    assert!(
        !verify(&native_only, &valid, "localhost"),
        "diagnostic=unknown-issuer"
    );

    let augmented = Verifier::new_with_extra_roots([test_root], Arc::new(default_provider()))
        .expect("construct platform verifier augmented by throwaway CA");
    assert!(
        verify(&augmented, &valid, "localhost"),
        "diagnostic=extra-root-valid-localhost"
    );
    assert!(
        !verify(&augmented, &valid, "wrong-name.invalid"),
        "diagnostic=hostname-mismatch"
    );
    assert!(
        !verify(&augmented, &expired, "localhost"),
        "diagnostic=expired-certificate"
    );

    let unrelated = Verifier::new_with_extra_roots([other_root], Arc::new(default_provider()))
        .expect("construct verifier with unrelated extra root");
    assert!(
        !verify(&unrelated, &valid, "localhost"),
        "diagnostic=untrusted-issuer"
    );
    let _ = other_ca;
    let _ = other_key;
}
