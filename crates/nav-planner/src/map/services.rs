//! Policy identity for generated navpois.

use super::identity::Digest;
use sha2::{Digest as _, Sha256};


pub const POIS_POLICY_ALGORITHM: &str = "openbank-reach-1";


pub fn pois_policy_digest() -> Digest {
    let mut h = Sha256::new();
    h.update(b"274bot.map.navpois-policy\0");
    h.update(1u16.to_be_bytes());
    h.update((POIS_POLICY_ALGORITHM.len() as u16).to_be_bytes());
    h.update(POIS_POLICY_ALGORITHM.as_bytes());
    Digest(h.finalize().into())
}


