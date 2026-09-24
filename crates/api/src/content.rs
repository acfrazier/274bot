//! Shared 274/289 content facts: rock display names and the common-loot
//! predicate.
//!
//! Curated training sites / leashes and the hostile-attacker policy live with
//! their script consumer in `script::content`; this module keeps the facts a
//! lower layer must not depend on `script` to read.

/// Posted rock option names. No rock-id sidecar is selected, so
/// `resolveRockIds` throws `not impl` and these stay display names.
pub const ROCK_TYPE_NAMES: &[&str] = &[
    "Clay",
    "Copper",
    "Tin",
    "Iron",
    "Silver",
    "Coal",
    "Gold",
    "Mithril",
    "Adamantite",
    "Runite",
];

/// Lowercase substrings treated as common loot by the compatibility API.
/// The policy stays in Rust; shims only publish and invoke it.
pub const COMMON_BANK_LOOT: &[&str] = &[
    "uncut",
    "sapphire",
    "emerald",
    "ruby",
    "diamond",
    "opal",
    "jade",
    "topaz",
    "strange fruit",
    "beer",
    "kebab",
];

/// Random-event fishing casket id in both selected 274 and 289 content/cache inputs.
pub const RANDOM_EVENT_CASKET_ID: i32 = 405;

/// Match one observed object against the host-owned common-loot policy.
pub fn matches_common_bank_loot(name: &str, id: i32) -> bool {
    id == RANDOM_EVENT_CASKET_ID
        || (!name.is_empty()
            && COMMON_BANK_LOOT
                .iter()
                .any(|part| contains_ascii_case_insensitive(name, part)))
}

fn contains_ascii_case_insensitive(value: &str, needle: &str) -> bool {
    value
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_bank_loot_matches_names_and_verified_casket_id() {
        assert!(matches_common_bank_loot("Uncut sapphire", -1));
        assert!(matches_common_bank_loot("STRANGE FRUIT", -1));
        assert!(matches_common_bank_loot("", 405));
        assert!(!matches_common_bank_loot("Rune scimitar", -1));
        assert!(!matches_common_bank_loot("", -1));
        assert_eq!(RANDOM_EVENT_CASKET_ID, 405);
        assert_eq!(
            COMMON_BANK_LOOT,
            [
                "uncut",
                "sapphire",
                "emerald",
                "ruby",
                "diamond",
                "opal",
                "jade",
                "topaz",
                "strange fruit",
                "beer",
                "kebab",
            ]
        );
    }
}
