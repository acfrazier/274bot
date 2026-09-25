//! Frozen catalog identities, selected-content access placements, and resolved
//! per-profile stands. Geometry is resolved by `nav`; eligibility by the caller's
//! captured account facts. Missing geometry remains an air-ranking fallback.

use crate::snapshot::WorldTile;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankOperation {
    pub name: &'static str,
    pub op: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankDefinition {
    pub name: &'static str,
    pub tile: WorldTile,
    pub approach: Option<WorldTile>,
    pub skill: Option<(i32, i32)>,
    pub quest: Option<&'static str>,
    pub setting: Option<&'static str>,
    pub object: Option<BankOperation>,
    pub open_first: Option<BankOperation>,
    pub npc: Option<BankOperation>,
    pub choose: Option<&'static str>,
}

include!("../data/game-data/bank-catalog.rs");

/// These opt-ins are false unless the calling script explicitly enables them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BankPreferences {
    pub use_mage_bank: bool,
    pub use_zanaris_bank: bool,
}

impl BankDefinition {
    pub fn eligible(
        &self,
        skill: impl Fn(i32) -> Option<i32>,
        quest_complete: impl Fn(&str) -> bool,
        preferences: BankPreferences,
    ) -> bool {
        self.skill.is_none_or(|(id, minimum)| skill(id).is_some_and(|level| level >= minimum))
            && self.quest.is_none_or(quest_complete)
            && self.setting.is_none_or(|setting| match setting {
                "useMageBank" => preferences.use_mage_bank,
                "useZanarisBank" => preferences.use_zanaris_bank,
                _ => false,
            })
    }
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BankPlacementKind {
    Object,
    Npc,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct BankPlacement {
    pub name: String,
    pub kind: BankPlacementKind,
    pub id: i32,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub width: i32,
    pub length: i32,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct BankPlacementFacts {
    pub rows: Vec<BankPlacement>,
    pub missing: Vec<String>,
}

/// `tile` is a resolved access stand, or the original frozen stand when the
/// selected content/world cannot prove one. Only `routable` rows enter a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedBank {
    pub name: &'static str,
    pub tile: WorldTile,
    pub definition: Option<&'static BankDefinition>,
    pub routable: bool,
}

impl NamedBank {
    pub const fn new(name: &'static str, tile: WorldTile) -> Self {
        Self { name, tile, definition: None, routable: true }
    }

    pub fn air_tile(&self) -> WorldTile {
        self.definition.and_then(|definition| definition.approach).unwrap_or(self.tile)
    }

    pub fn eligible(
        &self,
        skill: impl Fn(i32) -> Option<i32>,
        quest_complete: impl Fn(&str) -> bool,
        preferences: BankPreferences,
    ) -> bool {
        self.definition.is_none_or(|definition| definition.eligible(skill, quest_complete, preferences))
    }
}

/// Immutable per-profile rows, in frozen catalog order, shared by host and isolate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedBankFacts {
    banks: Vec<NamedBank>,
}

impl NamedBankFacts {
    pub fn empty() -> Self {
        Self { banks: Vec::new() }
    }

    pub fn from_banks(banks: Vec<NamedBank>) -> Self {
        Self { banks }
    }

    pub fn banks(&self) -> &[NamedBank] {
        &self.banks
    }
}
