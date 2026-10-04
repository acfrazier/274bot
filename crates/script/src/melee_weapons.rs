//! Melee weapon pick for v1 `meleeWeapons.js` (`bestMeleeWeapon`,
//! `knownMeleeWeapon`). The weapon list is the selected `equipment_names`
//! `melee_weapons` family; wield levels and the stab/slash tie-breaks are
//! not in selected content, so the frozen tier and type orders live here.

use api::game_data::EquipmentNameEntry;

/// Frozen Attack level per metal tier; a tier missing here wields at 1.
const TIER_ATTACK: [(&str, i32); 8] = [
    ("bronze", 1),
    ("iron", 1),
    ("steel", 5),
    ("black", 10),
    ("mithril", 20),
    ("adamant", 30),
    ("rune", 40),
    ("dragon", 60),
];
/// Frozen tier rank, weakest first.
const TIER_RANK: [&str; 8] = [
    "bronze", "iron", "steel", "black", "mithril", "adamant", "rune", "dragon",
];
/// Frozen type order for a target soft to stab (the metal dragons).
const STAB_ORDER: [&str; 6] = [
    "longsword",
    "battleaxe",
    "dagger",
    "sword",
    "scimitar",
    "mace",
];
/// Frozen type order everywhere else.
const SLASH_ORDER: [&str; 6] = [
    "scimitar",
    "sword",
    "longsword",
    "dagger",
    "battleaxe",
    "mace",
];

/// Caller's pick: Attack level (a JS number, so NaN wields nothing), stab
/// preference, and names a wield already refused (exact match).
pub struct WeaponPick<'a> {
    pub attack: f64,
    pub prefer_stab: bool,
    pub unusable: &'a [String],
}

/// Selected melee weapon names, in family order.
fn weapon_names(family: &[EquipmentNameEntry]) -> impl Iterator<Item = &str> {
    family
        .iter()
        .filter(|row| row.disposition == "resolved")
        .map(|row| row.requested_name.as_str())
}

fn metal_suffix(name: &str) -> Option<&str> {
    name.trim().split_once(' ').map(|(_, suffix)| suffix)
}

pub(crate) fn metal_tier_rank(name: &str) -> Option<usize> {
    let tier = name.trim().split_once(' ')?.0;
    TIER_RANK
        .iter()
        .position(|known| known.eq_ignore_ascii_case(tier))
}

pub(crate) fn metal_level(name: &str) -> Option<i32> {
    let tier = name.trim().split_once(' ')?.0;
    TIER_ATTACK
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(tier))
        .map(|(_, level)| *level)
}

fn melee_type(name: &str) -> &str {
    let suffix = metal_suffix(name).unwrap_or(name);
    let Some(start) = suffix.len().checked_sub(3) else {
        return suffix;
    };
    if suffix
        .get(start..)
        .is_some_and(|poison| poison.eq_ignore_ascii_case("(p)"))
    {
        &suffix[..start]
    } else {
        suffix
    }
}

pub(crate) fn same_or_lower_melee_weapon(candidate: &str, wanted: &str) -> bool {
    match (metal_tier_rank(candidate), metal_tier_rank(wanted)) {
        (Some(candidate_rank), Some(wanted_rank)) => {
            candidate_rank <= wanted_rank
                && melee_type(candidate).eq_ignore_ascii_case(melee_type(wanted))
        }
        _ => false,
    }
}

pub(crate) fn same_or_lower_metal_item(candidate: &str, wanted: &str) -> bool {
    match (metal_tier_rank(candidate), metal_tier_rank(wanted)) {
        (Some(candidate_rank), Some(wanted_rank)) => {
            candidate_rank <= wanted_rank
                && metal_suffix(candidate)
                    .zip(metal_suffix(wanted))
                    .is_some_and(|(candidate, wanted)| candidate.eq_ignore_ascii_case(wanted))
        }
        _ => false,
    }
}

pub(crate) fn is_melee_weapon(family: &[EquipmentNameEntry], name: &str) -> bool {
    weapon_names(family).any(|candidate| candidate.eq_ignore_ascii_case(name))
}

/// JS `indexOf`: -1 when absent.
fn index_of(list: &[&str], key: &str) -> i32 {
    list.iter()
        .position(|entry| entry.eq_ignore_ascii_case(key))
        .map_or(-1, |i| i as i32)
}

fn wield_level(name: &str) -> i32 {
    metal_level(name).unwrap_or(1)
}

fn order_key(name: &str, order: &[&str]) -> (i32, i32) {
    let tier = metal_tier_rank(name).map_or(-1, |index| index as i32);
    (-tier, index_of(order, melee_type(name)))
}

/// Shared candidate policy for compat, native combat, and Quester loadouts.
/// `permitted` narrows the policy for a caller; ordering and Attack gates stay
/// native and refused names retain compat's exact-match semantics.
pub(crate) fn best_melee_weapon_by_where<'a>(
    family: &'a [EquipmentNameEntry],
    attack: f64,
    prefer_stab: bool,
    unusable: &[String],
    mut permitted: impl FnMut(&str) -> bool,
    mut available: impl FnMut(&str) -> bool,
) -> Option<&'a str> {
    let order = if prefer_stab {
        &STAB_ORDER
    } else {
        &SLASH_ORDER
    };
    weapon_names(family)
        .filter(|name| available(name))
        .filter(|name| !unusable.iter().any(|refused| refused == name))
        .filter(|name| permitted(name))
        .filter(|name| f64::from(wield_level(name)) <= attack)
        .min_by_key(|name| order_key(name, order))
}

/// The highest-tier weapon among `available` (case-insensitive) the pick can
/// wield, stab or slash order breaking ties, then family order.
pub fn best_melee_weapon(
    family: &[EquipmentNameEntry],
    available: &[String],
    pick: &WeaponPick<'_>,
) -> Option<String> {
    best_melee_weapon_by_where(
        family,
        pick.attack,
        pick.prefer_stab,
        pick.unusable,
        |_| true,
        |name| available.iter().any(|have| have.eq_ignore_ascii_case(name)),
    )
    .map(str::to_string)
}

/// Borrowed native pick. It uses the same refusal, wield, and ordering policy
/// without copying the inventory at engagement start.
pub(crate) fn best_melee_weapon_by<'a>(
    family: &'a [EquipmentNameEntry],
    attack: i32,
    prefer_stab: bool,
    unusable: &[String],
    available: impl FnMut(&str) -> bool,
) -> Option<&'a str> {
    best_melee_weapon_by_where(
        family,
        f64::from(attack),
        prefer_stab,
        unusable,
        |_| true,
        available,
    )
}

/// The first family weapon (family order) named in `names`, case-insensitive.
pub fn known_melee_weapon(family: &[EquipmentNameEntry], names: &[String]) -> Option<String> {
    weapon_names(family)
        .find(|name| names.iter().any(|have| have.eq_ignore_ascii_case(name)))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    // Expected picks are read off the frozen `meleeWeapons.ts` /
    // `equipment.ts` MELEE_WEAPONS, not from this implementation.

    fn family() -> Vec<EquipmentNameEntry> {
        api::game_data::for_revision(ClientRevision::R289)
            .expect("selected game data")
            .equipment_names()
            .expect("equipment names")
            .melee_weapons
            .clone()
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    fn best(
        available: &[&str],
        attack: f64,
        prefer_stab: bool,
        unusable: &[&str],
    ) -> Option<String> {
        let unusable = names(unusable);
        best_melee_weapon(
            &family(),
            &names(available),
            &WeaponPick {
                attack,
                prefer_stab,
                unusable: &unusable,
            },
        )
    }

    #[test]
    fn slash_prefers_scimitar_and_stab_prefers_longsword_within_a_tier() {
        let bank = ["Rune scimitar", "Rune longsword", "Adamant scimitar"];
        assert_eq!(
            best(&bank, 40.0, false, &[]).as_deref(),
            Some("Rune scimitar")
        );
        assert_eq!(
            best(&bank, 40.0, true, &[]).as_deref(),
            Some("Rune longsword")
        );
    }

    #[test]
    fn attack_level_gates_the_tier() {
        let bank = ["Rune scimitar", "Rune longsword", "Adamant scimitar"];
        assert_eq!(
            best(&bank, 39.0, false, &[]).as_deref(),
            Some("Adamant scimitar")
        );
        assert_eq!(best(&["Rune scimitar"], 1.0, false, &[]), None);
        assert_eq!(best(&["Rune scimitar"], f64::NAN, false, &[]), None);
        assert_eq!(
            best(&["Black scimitar", "Steel longsword"], 10.0, true, &[]).as_deref(),
            Some("Black scimitar")
        );
        assert_eq!(
            best(&["Bronze sword", "Iron dagger"], 1.0, false, &[]).as_deref(),
            Some("Iron dagger")
        );
    }

    #[test]
    fn higher_tier_wins_and_the_family_name_is_returned() {
        assert_eq!(
            best(&["rune scimitar", "dragon longsword"], 60.0, false, &[]).as_deref(),
            Some("Dragon longsword")
        );
    }

    #[test]
    fn dragon_types_follow_the_stab_and_slash_orders() {
        let bank = [
            "Dragon dagger(p)",
            "Dragon dagger",
            "Dragon mace",
            "Dragon battleaxe",
        ];
        assert_eq!(
            best(&bank, 60.0, true, &[]).as_deref(),
            Some("Dragon battleaxe")
        );
        // (p) ranks as a dagger; the tie keeps family order.
        assert_eq!(
            best(&bank, 60.0, false, &[]).as_deref(),
            Some("Dragon dagger")
        );
    }

    #[test]
    fn refused_names_match_exactly() {
        let bank = ["Rune scimitar", "Rune sword"];
        assert_eq!(
            best(&bank, 40.0, false, &["Rune scimitar"]).as_deref(),
            Some("Rune sword")
        );
        assert_eq!(
            best(&bank, 40.0, false, &["rune scimitar"]).as_deref(),
            Some("Rune scimitar")
        );
    }

    #[test]
    fn borrowed_native_and_compat_pickers_share_refusal_and_candidate_policy() {
        let family = family();
        let available = names(&["Rune scimitar", "Rune sword"]);
        let unusable = names(&["Rune scimitar"]);
        let compat = best_melee_weapon(
            &family,
            &available,
            &WeaponPick {
                attack: 40.0,
                prefer_stab: false,
                unusable: &unusable,
            },
        );
        let native = best_melee_weapon_by(&family, 40, false, &unusable, |name| {
            available.iter().any(|have| have.eq_ignore_ascii_case(name))
        });

        assert_eq!(native, Some("Rune sword"));
        assert_eq!(compat.as_deref(), native);
    }

    #[test]
    fn unknown_names_pick_nothing() {
        assert_eq!(best(&["Abyssal whip", "Lobster"], 99.0, false, &[]), None);
    }

    #[test]
    fn known_weapon_uses_family_order() {
        assert_eq!(
            known_melee_weapon(
                &family(),
                &names(&["Lobster", "rune sword", "Bronze scimitar"])
            )
            .as_deref(),
            Some("Bronze scimitar")
        );
        assert_eq!(
            known_melee_weapon(&family(), &names(&["Abyssal whip", ""])),
            None
        );
    }
}
