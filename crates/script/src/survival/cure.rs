//! The Cure poison profile setting, in the form the cure decision reads.
//!
//! The setting is stored per profile in the vault
//! ([`vault::SurvivalSettings`]), mirrored to each running slot as one
//! atomic byte, and posted to native machines as this enum through
//! [`crate::native::ActionContext::cure_poison`]. The byte encoding is
//! fixed: `WhenNeeded` is 0 so a freshly created mirror already holds
//! the default, and any other byte decodes to `WhenNeeded` — a corrupt
//! mirror can never silently disable curing.

/// When a script drinks a held antipoison: the profile's Cure poison
/// setting. The default is `WhenNeeded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum CurePoison {
    /// Drink when the poison's worst case would take the bot below its
    /// eating line or kill it.
    #[default]
    WhenNeeded = 0,
    /// Drink as soon as the game reports the poison.
    Always = 1,
    /// Never drink. The need is still reported.
    Never = 2,
}

impl CurePoison {
    /// The mirrored byte for this policy (the per-slot `AtomicU8`).
    pub const fn to_byte(self) -> u8 {
        self as u8
    }

    /// Decode a mirrored byte. A byte no writer produces decodes to
    /// `WhenNeeded`, the default: an unknown value must not read as
    /// `Never`.
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::Always,
            2 => Self::Never,
            _ => Self::WhenNeeded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CurePoison;

    #[test]
    fn cure_poison_byte_round_trip_and_unknown_byte_defaults() {
        for policy in [
            CurePoison::WhenNeeded,
            CurePoison::Always,
            CurePoison::Never,
        ] {
            assert_eq!(CurePoison::from_byte(policy.to_byte()), policy);
        }
        assert_eq!(CurePoison::default(), CurePoison::WhenNeeded);
        assert_eq!(CurePoison::WhenNeeded.to_byte(), 0);
        // A byte no writer produces must not disable curing.
        assert_eq!(CurePoison::from_byte(3), CurePoison::WhenNeeded);
        assert_eq!(CurePoison::from_byte(u8::MAX), CurePoison::WhenNeeded);
    }
}
