//! Shared DTOs for the catalog bank aliases published as
//! `content.named_banks`.
//!
//! The curated catalog preferences (the five alias names, their preferred
//! stands, and the packed booth tiles that identify each cluster) live in
//! `script::content::BANK_ALIASES`. The accessible-geometry resolution
//! against the bound world's packed booth tiles and walk surface lives in
//! `nav::named_banks`, driven by `NavWorld::named_bank_facts`. This module
//! holds only the shared row shapes so `script` and `nav` agree on them
//! without either depending on the other. Omitted names stay absent so
//! `BANK_LOCATIONS.find` remains undefined.

use crate::snapshot::WorldTile;

/// One published alias: the catalog name and the walk stand (not the booth loc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedBank {
    pub name: &'static str,
    pub tile: WorldTile,
}

/// Immutable per-profile alias rows posted once onto `__rs2b0t_host.content`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedBankFacts {
    banks: Vec<NamedBank>,
}

/// Whether `name`/`tile` match a host-published unrestricted catalog alias row.
///
/// Gated banks, unknown names, and tile mismatches fail closed. This does not
/// evaluate quest/skill/global-setting policy; only rows posted in
/// `content.named_banks` count as unlocked.
pub fn bank_unlocked(facts: &NamedBankFacts, name: &str, tile: WorldTile) -> bool {
    facts
        .banks()
        .iter()
        .any(|bank| bank.name == name && bank.tile == tile)
}

impl NamedBankFacts {
    pub fn empty() -> Self {
        Self { banks: Vec::new() }
    }

    /// Wrap already-resolved rows (the nav resolver's output).
    pub fn from_banks(banks: Vec<NamedBank>) -> Self {
        Self { banks }
    }

    pub fn banks(&self) -> &[NamedBank] {
        &self.banks
    }
}

/// One curated catalog alias row: the name, the preferred walk stand, and
/// the packed booth loc tiles that identify that named cluster. Rows the
/// bound world cannot support are omitted from the published facts.
#[derive(Debug, Clone, Copy)]
pub struct BankAliasCandidate {
    pub name: &'static str,
    pub stand: WorldTile,
    pub booths: &'static [WorldTile],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::WorldTile;

    fn t(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    fn falador_facts() -> NamedBankFacts {
        NamedBankFacts::from_banks(vec![NamedBank {
            name: "Falador East",
            tile: t(3013, 3355),
        }])
    }

    #[test]
    fn published_alias_name_and_tile_unlock() {
        let facts = falador_facts();
        assert!(bank_unlocked(
            &facts,
            "Falador East",
            t(3013, 3355)
        ));
    }

    #[test]
    fn mismatched_tile_fails_closed() {
        assert!(!bank_unlocked(
            &falador_facts(),
            "Falador East",
            t(3013, 3356)
        ));
    }

    #[test]
    fn gated_or_unknown_name_fails_closed() {
        let facts = falador_facts();
        assert!(!bank_unlocked(&facts, "Canifis", t(3512, 3480)));
        assert!(!bank_unlocked(&facts, "Varrock West", t(3185, 3440)));
        assert!(!bank_unlocked(
            &NamedBankFacts::empty(),
            "Falador East",
            t(3013, 3355)
        ));
    }
}
