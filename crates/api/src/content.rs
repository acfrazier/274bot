//! Shared 274/289 content facts: the common-loot predicate.
//!
//!
//! Curated training sites / leashes and the hostile-attacker policy live with
//! their script consumer in `script::content`; this module keeps the facts a
//! lower layer must not depend on `script` to read.

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

/// Selected random-event fishing casket id, or no identity without selected
/// game data.
pub fn random_event_casket_id(data: Option<&crate::game_data::SelectedGameData>) -> Option<i32> {
    data.and_then(|data| data.item_by_alias("casket"))
        .map(|item| item.id)
}

/// Match one observed object against the host-owned common-loot policy.
/// Casket identity is resolved from selected item aliases and cached by the
/// calling selected-data slot.
pub fn matches_common_bank_loot(casket_id: Option<i32>, name: &str, id: i32) -> bool {
    casket_id == Some(id)
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
    use client::io::ClientRevision;

    #[test]
    fn common_bank_loot_matches_names_and_selected_casket_alias() {
        let data = crate::game_data::for_revision(ClientRevision::R289).expect("R289 data");
        let casket_id = random_event_casket_id(Some(data.as_ref())).expect("casket alias");
        assert!(matches_common_bank_loot(
            Some(casket_id),
            "Uncut sapphire",
            -1
        ));
        assert!(matches_common_bank_loot(
            Some(casket_id),
            "STRANGE FRUIT",
            -1
        ));
        assert!(matches_common_bank_loot(Some(casket_id), "", casket_id));
        assert!(!matches_common_bank_loot(None, "", casket_id));
        assert!(!matches_common_bank_loot(
            Some(casket_id),
            "Rune scimitar",
            -1
        ));
        assert!(!matches_common_bank_loot(Some(casket_id), "", -1));
    }
}
