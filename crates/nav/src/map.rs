//! Revision-bound map data contracts. No renderer, profile lookup or game actions.
//!
//! Producers stream/filter placements; these bounded records are POI candidates,
//! never a second world. Use checked readers rather than bare serde deserialization
//! at disk boundaries. Image identity is independent of nav/service facts.

pub mod cache;
pub mod formats;
pub mod identity;
pub mod poi;
pub mod records;
pub mod spatial;

use std::fmt;
use std::io;

use serde::de::{DeserializeOwned, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// Limits apply before bulk payload/pixel allocation, not after decoding a world.
#[derive(Debug)]
pub enum MapError {
    Io(io::Error),
    Schema(serde_json::Error),
    Unsupported { format: &'static str, version: u16 },
    Limit(&'static str),
    Invalid(&'static str),
    Identity,
    Digest,
    Duplicate(&'static str),
    NotReady,
    Path,
    Truncated,
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "map I/O: {e}"),
            Self::Schema(e) => write!(f, "map schema: {e}"),
            Self::Unsupported { format, version } => {
                write!(f, "unsupported {format} version {version}")
            }
            Self::Limit(what) => write!(f, "map limit exceeded: {what}"),
            Self::Invalid(what) => write!(f, "invalid map {what}"),
            Self::Identity => f.write_str("map identity mismatch"),
            Self::Digest => f.write_str("map payload digest mismatch"),
            Self::Duplicate(what) => write!(f, "duplicate map {what}"),
            Self::NotReady => f.write_str("partial map entry is not ready"),
            Self::Path => f.write_str("noncanonical map entry path"),
            Self::Truncated => f.write_str("truncated map data"),
        }
    }
}

impl std::error::Error for MapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Schema(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for MapError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Bounded sequence: an untrusted size hint never reserves memory. The next
/// element beyond N is rejected before it is deserialized/allocated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Rows<T, const N: usize>(Vec<T>);

impl<T, const N: usize> Rows<T, N> {
    pub fn new(rows: Vec<T>) -> Result<Self, MapError> {
        if rows.len() > N {
            return Err(MapError::Limit("record count"));
        }
        Ok(Self(rows))
    }
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
}

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Rows<T, N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bounded<T, const N: usize>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Bounded<T, N> {
            type Value = Rows<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "at most {N} records")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut rows = Vec::new();
                while rows.len() < N {
                    let Some(row) = seq.next_element()? else {
                        return Ok(Rows(rows));
                    };
                    rows.push(row);
                }
                // IgnoredAny skips without constructing another T (or its strings).
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("record count limit exceeded"));
                }
                Ok(Rows(rows))
            }
        }
        deserializer.deserialize_seq(Bounded::<T, N>(std::marker::PhantomData))
    }
}

/// Display/provenance text, never an arbitrary filesystem path. Bounded UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Text(String);

impl Text {
    pub const MAX_BYTES: usize = 256;
    pub fn new(value: &str) -> Result<Self, MapError> {
        if value.len() > Self::MAX_BYTES {
            return Err(MapError::Limit("text bytes"));
        }
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(MapError::Invalid("text"));
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BoundedText;
        impl Visitor<'_> for BoundedText {
            type Value = Text;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("1..=256 bytes of display text")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Text, E> {
                Text::new(value).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(BoundedText)
    }
}

pub(crate) fn json<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T, MapError> {
    if bytes.len() > limit {
        return Err(MapError::Limit("JSON bytes"));
    }
    serde_json::from_slice(bytes).map_err(MapError::Schema)
}

/// First pass skips all payload collections (IgnoredAny). Reject an unsupported
/// schema or identity before allocating records, even when JSON fields are reordered.
pub(crate) fn preflight<I: DeserializeOwned + PartialEq>(
    bytes: &[u8],
    limit: usize,
    expected: I,
    format: &'static str,
    schema: u16,
) -> Result<(), MapError> {
    #[derive(Deserialize)]
    struct Header<I> {
        schema: u16,
        identity: I,
    }
    let header: Header<I> = json(bytes, limit)?;
    version(format, header.schema, schema)?;
    if header.identity != expected {
        return Err(MapError::Identity);
    }
    Ok(())
}

pub(crate) fn version(format: &'static str, actual: u16, expected: u16) -> Result<(), MapError> {
    if actual != expected {
        return Err(MapError::Unsupported {
            format,
            version: actual,
        });
    }
    Ok(())
}

pub(crate) fn revision(revision: u16) -> Result<(), MapError> {
    if !matches!(revision, 274 | 289) {
        return Err(MapError::Invalid("revision"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "map/tests.rs"]
mod tests;
