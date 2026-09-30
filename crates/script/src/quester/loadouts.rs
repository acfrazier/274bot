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

    /// An exact operator row shadows the compiled row of the same qualified
    /// name. Neither collection is mutated.
    pub fn resolve(&self, name: &str) -> Option<LoadoutRef<'_>> {
        self.store
            .iter()
            .find(|row| row.name == name)
            .map(LoadoutRef::Store)
            .or_else(|| {
                self.compiled
                    .iter()
                    .find(|row| row.name == name)
                    .map(LoadoutRef::Compiled)
            })
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

const METALS: [(&str, i32); 8] = [
    ("bronze", 1),
    ("iron", 1),
    ("steel", 5),
    ("black", 10),
    ("mithril", 20),
    ("adamant", 30),
    ("rune", 40),
    ("dragon", 60),
];

/// Return strongest-first permissible candidates from an available item list.
/// The caller decides whether "available" means held, equipped, or banked.
pub fn tier_candidates(
    slot: &str,
    wanted: &str,
    available: &[String],
    facts: TierFacts,
) -> Vec<String> {
    let Some((wanted_tier, wanted_suffix)) = split_metal(wanted) else {
        return available
            .iter()
            .find(|name| name.eq_ignore_ascii_case(wanted))
            .cloned()
            .into_iter()
            .collect();
    };
    let wanted_rank = METALS
        .iter()
        .position(|(tier, _)| tier.eq_ignore_ascii_case(wanted_tier))
        .unwrap_or(0);
    let skill = if slot == "righthand" {
        facts.attack
    } else {
        facts.defence
    };
    let mut candidates = available
        .iter()
        .filter_map(|candidate| {
            let exact = candidate.eq_ignore_ascii_case(wanted);
            let ranked = split_metal(candidate).and_then(|(tier, suffix)| {
                let rank = METALS
                    .iter()
                    .position(|(known, _)| known.eq_ignore_ascii_case(tier))?;
                (rank <= wanted_rank && suffix.eq_ignore_ascii_case(wanted_suffix)).then_some(rank)
            });
            let special = ((wanted.eq_ignore_ascii_case("dragon longsword")
                && candidate.eq_ignore_ascii_case("rune sword"))
                || (wanted.eq_ignore_ascii_case("rune platebody")
                    && candidate.eq_ignore_ascii_case("rune chainbody")))
            .then_some(6);
            let rank = ranked.or(special)?;
            (exact || rank <= wanted_rank)
                .then_some(())
                .filter(|_| METALS[rank].1 <= skill && quest_usable(candidate, facts))
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
pub fn fill_owned_lower_tiers(row: &Loadout, owned: &[String], facts: TierFacts) -> Loadout {
    let mut filled = row.clone();
    for (slot, wanted) in &row.worn {
        if let Some(best) = tier_candidates(slot, wanted, owned, facts)
            .into_iter()
            .next()
        {
            filled.worn.insert(slot.clone(), best);
        }
    }
    filled
}

fn split_metal(name: &str) -> Option<(&str, &str)> {
    let (tier, suffix) = name.trim().split_once(' ')?;
    METALS
        .iter()
        .any(|(known, _)| tier.eq_ignore_ascii_case(known))
        .then_some((tier, suffix))
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
    fn store_shadows_compiled_without_mutating_either() {
        let compiled = Arc::from([Loadout::new("tree/melee").with_slot("righthand", "Rune sword")]);
        let store = Arc::from([Loadout::new("tree/melee").with_slot("righthand", "Iron sword")]);
        let overlay = LoadoutOverlay::new(Arc::clone(&store), Arc::clone(&compiled));
        let resolved = overlay.resolve("tree/melee").unwrap();
        assert!(resolved.operator_override());
        assert_eq!(resolved.row().worn["righthand"], "Iron sword");
        assert_eq!(compiled[0].worn["righthand"], "Rune sword");
        assert_eq!(overlay.list().len(), 1);
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
        );
        assert_eq!(filled.worn["righthand"], "Rune sword");
        assert_eq!(filled.worn["torso"], "Rune chainbody");
    }
}
