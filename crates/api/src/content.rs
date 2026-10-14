//! Shared 274/289 content facts: bank-loot and gathering-junk policies.
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

/// Selected random-event casket id from game data, or `None` when no revision
/// is selected. R289 `content/pack/obj.pack:406` records `405=casket`.
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

/// Engine object definitions list `Flier` at
/// `content/scripts/_unpack/225/all.obj:68-69` (obj 956); the event-drop
/// scripts are `content/scripts/drop tables/scripts/barbarian.rs2:53,113`,
/// `content/scripts/drop tables/scripts/imp.rs2:73` and
/// `content/scripts/npc/scripts/man.rs2:45`.
/// The only half-pie objects are at lines 62, 88 and 114 of
/// `content/scripts/skill_cooking/configs/cooking_inv/configs/pies/pies.obj`.
const DISPOSABLE_GATHER_EVENT_LEFTOVERS: &[&str] = &[
    "Flier",
    "Half a meat pie",
    "Half a redberry pie",
    "Half an apple pie",
];

/// Whether an observed item is disposable gathering junk.
///
/// The caller still decides which tools, gear, and gathered products to keep.
pub fn is_disposable_gather_junk(casket_id: Option<i32>, name: &str, id: i32) -> bool {
    if matches_common_bank_loot(casket_id, name, id) {
        return true;
    }

    let name = name.trim();
    DISPOSABLE_GATHER_EVENT_LEFTOVERS
        .iter()
        .any(|item| name.eq_ignore_ascii_case(item))
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

    #[test]
    fn disposable_gather_junk_matches_common_loot_and_exact_event_leftovers() {
        let data = crate::game_data::for_revision(ClientRevision::R289).expect("R289 data");
        let casket_id = random_event_casket_id(Some(data.as_ref())).expect("casket alias");

        assert!(is_disposable_gather_junk(
            Some(casket_id),
            "Mystery box",
            casket_id
        ));
        assert!(is_disposable_gather_junk(
            Some(casket_id),
            "Uncut sapphire",
            -1
        ));
        assert!(is_disposable_gather_junk(Some(casket_id), "  fLiEr  ", -1));
        assert!(is_disposable_gather_junk(
            Some(casket_id),
            "Half a meat pie",
            -1
        ));
        assert!(is_disposable_gather_junk(
            Some(casket_id),
            "Half a redberry pie",
            -1
        ));
        assert!(is_disposable_gather_junk(
            Some(casket_id),
            "Half an apple pie",
            -1
        ));
        assert!(!is_disposable_gather_junk(
            Some(casket_id),
            "Half a meat pie slice",
            -1
        ));
        assert!(!is_disposable_gather_junk(Some(casket_id), "Rune axe", -1));
        assert!(!is_disposable_gather_junk(Some(casket_id), "", -1));
    }
}
