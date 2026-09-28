//! [`Secret`]: a string whose buffer is overwritten with zeros when it is
//! dropped or cleared. Used for account passwords and the vault passphrase so
//! a copy of either does not linger in freed heap memory.

use std::fmt;
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::{Zeroize, Zeroizing};

/// A secret string. `Debug` never prints the value, the whole allocation is
/// zeroed on drop and by [`Secret::clear`], and it serializes as the plain
/// string (the vault's JSON shape is unchanged).
///
/// Zeroing is best effort: it covers this buffer, not copies the value was
/// already moved into (an `Arc<str>`, a log-redaction registry, a network
/// login block), and not earlier reallocations of a buffer that grew. Reserve
/// capacity up front ([`Secret::with_capacity`]) for text typed by hand.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// An empty secret with room for `capacity` bytes, so typing into it does
    /// not reallocate (and leave an unzeroed copy behind) for a normal length.
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Zeroizing::new(String::with_capacity(capacity)))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Zero the buffer (the whole capacity) and empty the secret. `String::clear`
    /// would only reset the length and leave the bytes in place.
    pub fn clear(&mut self) {
        self.0.zeroize();
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(Zeroizing::new(value.to_owned()))
    }
}

impl Deref for Secret {
    type Target = String;

    fn deref(&self) -> &String {
        &self.0
    }
}

impl DerefMut for Secret {
    fn deref_mut(&mut self) -> &mut String {
        &mut self.0
    }
}

impl PartialEq<str> for Secret {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Secret {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Secret {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn debug_never_prints_the_value() {
        let secret = Secret::from("hunter2-hunter2");
        assert!(!format!("{secret:?}").contains("hunter2"));
        assert!(!format!("{:?}", Some(&secret)).contains("hunter2"));
    }

    #[test]
    fn clear_zeroes_the_whole_allocation_not_just_the_length() {
        let mut secret = Secret::with_capacity(64);
        secret.push_str("hunter2-hunter2");
        let (ptr, capacity) = (secret.as_ptr(), secret.capacity());

        secret.clear();

        assert!(secret.is_empty());
        // SAFETY: `clear` keeps the allocation, and zeroize wrote every byte
        // of its capacity, so all `capacity` bytes are initialized.
        let bytes = unsafe { std::slice::from_raw_parts(ptr, capacity) };
        assert!(bytes.iter().all(|b| *b == 0), "{bytes:?}");
    }

    #[test]
    fn it_serializes_as_a_plain_string_so_the_vault_json_is_unchanged() {
        let json = serde_json::to_string(&Secret::from("pa\"ss")).unwrap();
        assert_eq!(json, r#""pa\"ss""#);
        let back: Secret = serde_json::from_str(&json).unwrap();
        assert_eq!(back, "pa\"ss");
    }
}
