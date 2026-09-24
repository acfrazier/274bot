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

fn tier_of(name: &str) -> String {
    name.split(' ').next().unwrap_or("").to_lowercase()
}

fn type_of(name: &str) -> String {
    let rest = name
        .split(' ')
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    rest.strip_suffix("(p)").map(str::to_string).unwrap_or(rest)
}

/// JS `indexOf`: -1 when absent.
fn index_of(list: &[&str], key: &str) -> i32 {
    list.iter()
        .position(|entry| *entry == key)
        .map_or(-1, |i| i as i32)
}

fn wield_level(name: &str) -> i32 {
    let tier = tier_of(name);
    TIER_ATTACK
        .iter()
        .find(|(t, _)| *t == tier)
        .map_or(1, |(_, level)| *level)
}

fn lowered(names: &[String]) -> Vec<String> {
    names.iter().map(|name| name.to_lowercase()).collect()
}

/// The highest-tier weapon among `available` (case-insensitive) the pick can
/// wield, stab or slash order breaking ties, then family order.
pub fn best_melee_weapon(
    family: &[EquipmentNameEntry],
    available: &[String],
    pick: &WeaponPick<'_>,
) -> Option<String> {
    let order: &[&str] = if pick.prefer_stab {
        &STAB_ORDER
    } else {
        &SLASH_ORDER
    };
    let have = lowered(available);
    let mut usable: Vec<&str> = weapon_names(family)
        .filter(|name| have.contains(&name.to_lowercase()))
        .filter(|name| !pick.unusable.iter().any(|refused| refused == name))
        .filter(|name| f64::from(wield_level(name)) <= pick.attack)
        .collect();
    // Stable, like the frozen `Array.prototype.sort`.
    usable.sort_by_key(|name| {
        (
            -index_of(&TIER_RANK, &tier_of(name)),
            index_of(order, &type_of(name)),
        )
    });
    usable.first().map(|name| (*name).to_string())
}

/// The first family weapon (family order) named in `names`, case-insensitive.
pub fn known_melee_weapon(family: &[EquipmentNameEntry], names: &[String]) -> Option<String> {
    let have = lowered(names);
    weapon_names(family)
        .find(|name| have.contains(&name.to_lowercase()))
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
