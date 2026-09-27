//! Canonical identities hash binary preimages, never JSON, paths or transfer CRCs.
//! Every integer in these preimages is big-endian. Text is u16 byte length + UTF-8;
//! a digest is its raw 32 bytes (not 64 hex bytes). Domain tags include the NUL.

use super::{MapError, Rows, Text};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};
use std::fmt;

pub const IMAGE_SCHEMA: u16 = 1;
pub const CATALOGUE_SCHEMA: u16 = 1;
pub const IMAGE_KEY_DOMAIN: &[u8] = b"274bot.map.image\0";
pub const CATALOGUE_KEY_DOMAIN: &[u8] = b"274bot.map.catalogue\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
    pub fn from_hex(value: &str) -> Result<Self, MapError> {
        if value.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(MapError::Invalid("noncanonical digest"));
        }
        crate::pack::sha256_from_hex(value)
            .map(Self)
            .map_err(|_| MapError::Invalid("digest"))
    }
}
impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Hex;
        impl serde::de::Visitor<'_> for Hex {
            type Value = Digest;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("64 lowercase SHA-256 hex digits")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Digest, E> {
                Digest::from_hex(value).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Hex)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ImageKey(pub Digest);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CatalogueKey(pub Digest);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageIdentity {
    pub revision: u16,
    /// Verified decoded client identity (274DCI01), not cache-transfer identity.
    pub content: Digest,
    pub policy: Digest,
}
impl ImageIdentity {
    /// SHA256(IMAGE_KEY_DOMAIN || revision:u16 || schema:u16 || content || policy).
    pub fn key(self) -> Result<ImageKey, MapError> {
        super::revision(self.revision)?;
        Ok(ImageKey(key(
            IMAGE_KEY_DOMAIN,
            self.revision,
            IMAGE_SCHEMA,
            self.content,
            self.policy,
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueIdentity {
    pub revision: u16,
    pub content: Digest,
    pub policy: Digest,
}
impl CatalogueIdentity {
    /// SHA256(CATALOGUE_KEY_DOMAIN || revision:u16 || schema:u16 || content || policy).
    pub fn key(self) -> Result<CatalogueKey, MapError> {
        super::revision(self.revision)?;
        Ok(CatalogueKey(key(
            CATALOGUE_KEY_DOMAIN,
            self.revision,
            CATALOGUE_SCHEMA,
            self.content,
            self.policy,
        )))
    }
    /// Cheap merged-view identity; changing routing or services never changes art.
    /// SHA256("274bot.map.merge\0" || catalogue_key || nav || presence:u8 || [pois]).
    pub fn merged_key(self, nav: Digest, pois: Option<Digest>) -> Result<Digest, MapError> {
        let mut h = Sha256::new();
        h.update(b"274bot.map.merge\0");
        h.update(self.key()?.0 .0);
        h.update(nav.0);
        h.update([u8::from(pois.is_some())]);
        if let Some(pois) = pois {
            h.update(pois.0);
        }
        Ok(Digest(h.finalize().into()))
    }
}

fn key(domain: &[u8], revision: u16, schema: u16, content: Digest, policy: Digest) -> Digest {
    let mut h = Sha256::new();
    h.update(domain);
    h.update(revision.to_be_bytes());
    h.update(schema.to_be_bytes());
    h.update(content.0);
    h.update(policy.0);
    Digest(h.finalize().into())
}
fn text(h: &mut Sha256, value: &Text) {
    h.update((value.as_str().len() as u16).to_be_bytes());
    h.update(value.as_str().as_bytes());
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncoderLibrary {
    pub name: Text,
    pub version: Text,
}

/// Output-affecting producer closure. B supplies the actual encoder and exact
/// compression/libpng dependencies it uses; there is no implicit system default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BakePolicy {
    pub algorithm: Text,
    pub producer_sources: Digest,
    pub encoder: EncoderLibrary,
    /// Sorted unique library names, including compression backend/libpng if used.
    pub libraries: Rows<EncoderLibrary, 8>,
    /// Canonical producer-owned options, including filter/compression choices.
    pub options: Text,
}
impl BakePolicy {
    /// SHA256("274bot.map.bake-policy\0" || version:u16=1 || algorithm:text ||
    /// sources:32 || encoder.name:text || encoder.version:text || count:u8 ||
    /// each library(name:text,version:text) || options:text).
    pub fn identity(&self) -> Result<Digest, MapError> {
        for pair in self.libraries.as_slice().windows(2) {
            if pair[0].name >= pair[1].name {
                return Err(MapError::Invalid("encoder library ordering"));
            }
        }
        let mut h = Sha256::new();
        h.update(b"274bot.map.bake-policy\0");
        h.update(1u16.to_be_bytes());
        text(&mut h, &self.algorithm);
        h.update(self.producer_sources.0);
        text(&mut h, &self.encoder.name);
        text(&mut h, &self.encoder.version);
        h.update([self.libraries.as_slice().len() as u8]);
        for lib in self.libraries.as_slice() {
            text(&mut h, &lib.name);
            text(&mut h, &lib.version);
        }
        text(&mut h, &self.options);
        Ok(Digest(h.finalize().into()))
    }
}

/// Local classifier policy does not include server facts or terrain policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CataloguePolicy {
    pub algorithm: Text,
    pub producer_sources: Digest,
}
impl CataloguePolicy {
    /// SHA256("274bot.map.catalogue-policy\0" || version:u16=1 || algorithm:text || sources:32).
    pub fn identity(&self) -> Digest {
        let mut h = Sha256::new();
        h.update(b"274bot.map.catalogue-policy\0");
        h.update(1u16.to_be_bytes());
        text(&mut h, &self.algorithm);
        h.update(self.producer_sources.0);
        Digest(h.finalize().into())
    }
}
