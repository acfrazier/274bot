//! Read-only operator/Path loadout overlay and conservative owned-tier fills.
use crate::loadouts_store::Loadout;
use std::sync::Arc;

#[derive(Clone)]
pub struct LoadoutOverlay {
    store: Arc<[Loadout]>,
    compiled: Arc<[Loadout]>,
}

#[derive(Clone, Copy)]
pub enum LoadoutRef<'a> {
    Store(&'a Loadout),
    Compiled(&'a Loadout),
}

impl<'a> LoadoutRef<'a> {
    pub fn row(self) -> &'a Loadout {
        match self {
            Self::Store(row) | Self::Compiled(row) => row,
        }
    }

    pub fn operator_override(self) -> bool {
        matches!(self, Self::Store(_))
    }
}

impl LoadoutOverlay {
    pub fn new(store: Arc<[Loadout]>, compiled: Arc<[Loadout]>) -> Self {
        Self { store, compiled }
    }

    pub fn from_default_store(compiled: Arc<[Loadout]>) -> Self {
        let store = crate::loadouts_store::LoadoutsStore::with_default_path().snapshot();
        Self::new(Arc::from(store), compiled)
    }

    /// Resolve an exact case-sensitive name across operator and compiled rows,
    /// preferring the operator row. A missing name remains unresolved.
    pub fn resolve(&self, name: &str) -> Option<LoadoutRef<'_>> {
        let compiled = self
            .compiled
            .iter()
            .filter(|candidate| !self.store.iter().any(|row| row.name == candidate.name));
        let selected =
            crate::loadouts_store::select_loadout(self.store.iter().chain(compiled), name)?;
        self.store
            .iter()
            .find(|row| std::ptr::eq(*row, selected))
            .map(LoadoutRef::Store)
            .or(Some(LoadoutRef::Compiled(selected)))
    }

    pub fn list(&self) -> Vec<LoadoutRef<'_>> {
        let mut rows: Vec<_> = self.store.iter().map(LoadoutRef::Store).collect();
        rows.extend(
            self.compiled
                .iter()
                .filter(|compiled| !self.store.iter().any(|row| row.name == compiled.name))
                .map(LoadoutRef::Compiled),
        );
        rows
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TierFacts {
    pub attack: i32,
    pub defence: i32,
    pub ranged: i32,
    pub lost_city: bool,
    pub heroes: bool,
    pub dragon_slayer: bool,
}

/// Return strongest-first permissible candidates from an available item list.
/// The caller decides whether "available" means held, equipped, or banked.
pub fn tier_candidates(
    slot: &str,
    wanted: &str,
    available: &[String],
    facts: TierFacts,
    melee_family: &[api::game_data::EquipmentNameEntry],
) -> Vec<String> {
    if slot.eq_ignore_ascii_case("righthand")
        && crate::melee_weapons::is_melee_weapon(melee_family, wanted)
    {
        let mut result = Vec::new();
        while let Some(candidate) = crate::melee_weapons::best_melee_weapon_by_where(
            melee_family,
            f64::from(facts.attack),
            false,
            &result,
            |candidate| {
                crate::melee_weapons::same_or_lower_melee_weapon(candidate, wanted)
                    || (wanted.eq_ignore_ascii_case("dragon longsword")
                        && candidate.eq_ignore_ascii_case("rune sword"))
                    || (wanted.eq_ignore_ascii_case("rune platebody")
                        && candidate.eq_ignore_ascii_case("rune chainbody"))
            },
            |candidate| {
                available
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(candidate))
                    && quest_usable(candidate, facts)
            },
        ) {
            result.push(candidate.to_string());
        }
        return result;
    }

    let exact = || {
        available
            .iter()
            .find(|name| name.eq_ignore_ascii_case(wanted))
            .cloned()
    };
    let Some(wanted_rank) = crate::melee_weapons::metal_tier_rank(wanted) else {
        return exact().into_iter().collect();
    };
    let skill = if slot.eq_ignore_ascii_case("righthand") {
        facts.attack
    } else {
        facts.defence
    };
    let mut candidates = available
        .iter()
        .filter_map(|candidate| {
            let exact = candidate.eq_ignore_ascii_case(wanted);
            let ranked = crate::melee_weapons::same_or_lower_metal_item(candidate, wanted)
                .then(|| crate::melee_weapons::metal_tier_rank(candidate))
                .flatten();
            let special = ((wanted.eq_ignore_ascii_case("dragon longsword")
                && candidate.eq_ignore_ascii_case("rune sword"))
                || (wanted.eq_ignore_ascii_case("rune platebody")
                    && candidate.eq_ignore_ascii_case("rune chainbody")))
            .then_some(6);
            let rank = ranked.or(special)?;
            (exact || rank <= wanted_rank)
                .then_some(())
                .filter(|_| {
                    crate::melee_weapons::metal_level(candidate).unwrap_or(1) <= skill
                        && quest_usable(candidate, facts)
                })
                .map(|_| (rank, exact, candidate.clone()))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| std::cmp::Reverse((candidate.0, candidate.1)));
    candidates.dedup_by(|a, b| a.2.eq_ignore_ascii_case(&b.2));
    candidates.into_iter().map(|(_, _, name)| name).collect()
}

/// Fill missing or unusable named worn pieces, never carry rows. A substitute
/// must be an actually owned, same-type lower metal tier and pass its
/// stat/quest gate.
pub fn fill_owned_lower_tiers(
    row: &Loadout,
    owned: &[String],
    facts: TierFacts,
    melee_family: &[api::game_data::EquipmentNameEntry],
) -> Loadout {
    let mut filled = row.clone();
    for (slot, wanted) in &row.worn {
        if let Some(best) = tier_candidates(slot, wanted, owned, facts, melee_family)
            .into_iter()
            .next()
        {
            filled.worn.insert(slot.clone(), best);
        }
    }
    filled
}

fn quest_usable(name: &str, facts: TierFacts) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "rune platebody" && !facts.dragon_slayer {
        return false;
    }
    if matches!(lower.as_str(), "dragon longsword" | "dragon dagger") && !facts.lost_city {
        return false;
    }
    if matches!(lower.as_str(), "dragon battleaxe" | "dragon mace") && !facts.heroes {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_shadows_compiled_for_exact_name_only() {
        let compiled = Arc::from([Loadout::new("tree/melee").with_slot("righthand", "Rune sword")]);
        let store = Arc::from([Loadout::new("tree/melee").with_slot("righthand", "Iron sword")]);
        let overlay = LoadoutOverlay::new(Arc::clone(&store), Arc::clone(&compiled));
        let resolved = overlay.resolve("tree/melee").unwrap();
        assert!(resolved.operator_override());
        assert_eq!(resolved.row().worn["righthand"], "Iron sword");
        assert!(overlay.resolve("TREE/melee").is_none());
        assert!(overlay.resolve("missing").is_none());
        assert_eq!(compiled[0].worn["righthand"], "Rune sword");
        assert_eq!(overlay.list().len(), 1);
    }

    fn melee_family() -> Vec<api::game_data::EquipmentNameEntry> {
        api::game_data::for_revision(client::io::ClientRevision::R289)
            .unwrap()
            .equipment_names()
            .unwrap()
            .melee_weapons
            .clone()
    }

    #[test]
    fn tier_fill_uses_owned_stat_legal_piece_and_respects_quest_gate() {
        let row = Loadout::new("quest/melee")
            .with_slot("righthand", "Dragon longsword")
            .with_slot("torso", "Rune platebody");
        let owned = vec![
            "Dragon longsword".into(),
            "Rune sword".into(),
            "Rune platebody".into(),
            "Rune chainbody".into(),
        ];
        let filled = fill_owned_lower_tiers(
            &row,
            &owned,
            TierFacts {
                attack: 40,
                defence: 40,
                ..TierFacts::default()
            },
            &melee_family(),
        );
        assert_eq!(filled.worn["righthand"], "Rune sword");
        assert_eq!(filled.worn["torso"], "Rune chainbody");
    }

    #[test]
    fn melee_loadout_candidates_follow_shared_weapon_preference_order() {
        let available = vec!["Rune longsword".into(), "Rune sword".into()];
        let candidates = tier_candidates(
            "righthand",
            "Dragon longsword",
            &available,
            TierFacts {
                attack: 40,
                ..TierFacts::default()
            },
            &melee_family(),
        );
        assert_eq!(candidates.first().map(String::as_str), Some("Rune sword"));
    }
}
