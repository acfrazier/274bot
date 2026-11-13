//! Dragon Slayer progress stage read from the quest journal.
//!
//! The server sends the client only the quest list colour, never the quest
//! varp (`dragonquest` is `scope=perm` with no transmit,
//! `quest_dragon/configs/quest_dragon.varp:1-2`), so the stage comes from the
//! journal text that `quest_dragon/scripts/dragon_journal.rs2` builds from
//! that varp. The stage values are the varp values in
//! `quest_dragon/configs/quest_dragon.constant:1-7` and `^dragon_complete`
//! (`general/configs/quest.constant:18`); the engine skips 4 to 6. They are
//! the numbers rs2b0t's `DRAGON_STAGE` uses, so a compat script reads the
//! content's own stage.
//!
//! The journal text cannot tell one case apart, and the resolver reads it as
//! the earlier stage: stage 3 (ship bought) while the player has not yet
//! heard all three of Oziach's hints shows "I should return to Oziach", the
//! same line stage 2 shows (`dragon_journal.rs2:27` and `:39`).

use crate::quester::progress::normalize_journal;
use std::sync::Arc;

/// The quest's catalog id (`crates/script/paths/289/dragon.json`).
const QUEST_ID: &str = "dragon";

/// One `%dragonquest` value, named as rs2b0t's `DRAGON_STAGE` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragonStage {
    /// `^quest_dragon_not_started`.
    NotStarted,
    /// `^quest_dragon_spoken_to_guildmaster`.
    SpokenGuildmaster,
    /// `^quest_dragon_spoken_to_oziach`.
    SpokenOziach,
    /// `^quest_dragon_bought_ship`.
    BoughtShip,
    /// `^quest_dragon_repaired_ship`.
    RepairedShip,
    /// `^quest_dragon_ned_given_map`.
    NedGivenMap,
    /// `^quest_dragon_sailed_to_crandor`.
    SailedToCrandor,
    /// `^dragon_complete`.
    Complete,
}

impl DragonStage {
    /// Every stage, in `%dragonquest` order.
    pub const ALL: [Self; 8] = [
        Self::NotStarted,
        Self::SpokenGuildmaster,
        Self::SpokenOziach,
        Self::BoughtShip,
        Self::RepairedShip,
        Self::NedGivenMap,
        Self::SailedToCrandor,
        Self::Complete,
    ];

    /// The `%dragonquest` value.
    pub const fn value(self) -> i32 {
        match self {
            Self::NotStarted => 0,
            Self::SpokenGuildmaster => 1,
            Self::SpokenOziach => 2,
            Self::BoughtShip => 3,
            Self::RepairedShip => 7,
            Self::NedGivenMap => 8,
            Self::SailedToCrandor => 9,
            Self::Complete => 10,
        }
    }

    /// The stage for a `%dragonquest` value; `None` for the values the
    /// engine never stores (4 to 6) and anything past completion.
    pub fn from_value(value: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.value() == value)
    }

    /// The member name in rs2b0t's `DRAGON_STAGE`.
    pub const fn upstream_name(self) -> &'static str {
        match self {
            Self::NotStarted => "NOT_STARTED",
            Self::SpokenGuildmaster => "SPOKEN_GUILDMASTER",
            Self::SpokenOziach => "SPOKEN_OZIACH",
            Self::BoughtShip => "BOUGHT_SHIP",
            Self::RepairedShip => "REPAIRED_SHIP",
            Self::NedGivenMap => "NED_GIVEN_MAP",
            Self::SailedToCrandor => "SAILED_TO_CRANDOR",
            Self::Complete => "COMPLETE",
        }
    }
}

/// One journal rule: the stage when every needle occurs in the normalized
/// journal. Needles are already normalized (`normalize_text`).
struct Rule {
    stage: DragonStage,
    all: &'static [&'static str],
}

/// Newest stage first, so the first match wins: the journal script appends
/// to one string, so each stage's text keeps the earlier history.
const RULES: [Rule; 9] = [
    // `dragon_journal.rs2:82-94`: the completed journal.
    Rule {
        stage: DragonStage::Complete,
        all: &["quest complete!"],
    },
    // `:70-80`: the last line of the sailed-to-Crandor journal.
    Rule {
        stage: DragonStage::SailedToCrandor,
        all: &["all i need to do is kill the"],
    },
    // `:61-68`: the last line after Ned has the map.
    Rule {
        stage: DragonStage::NedGivenMap,
        all: &["set sail for"],
    },
    // `:51-59` with `ds_ship_repairs` (`:162-169`): the repaired line is
    // printed once `%dragonquestvar` is 3, and `lady_lumbridge.rs2:84-86`
    // moves the stage from 3 to 7 in the same script.
    Rule {
        stage: DragonStage::RepairedShip,
        all: &[
            "i still need to find a captain",
            "i have repaired my ship using wooden planks",
        ],
    },
    // `:37-49`: the captain line without the repaired line.
    Rule {
        stage: DragonStage::BoughtShip,
        all: &["i still need to find a captain"],
    },
    // `:25-35`: stage 2 once Oziach's three hints are known.
    Rule {
        stage: DragonStage::SpokenOziach,
        all: &["i should see if there is a ship for sale"],
    },
    // `:27`, `:39`: stage 2, or stage 3 before the hints are known.
    Rule {
        stage: DragonStage::SpokenOziach,
        all: &["i should return to oziach for more detailed instructions"],
    },
    // `:21-23`.
    Rule {
        stage: DragonStage::SpokenGuildmaster,
        all: &["i should speak to oziach"],
    },
    // `:4-14`.
    Rule {
        stage: DragonStage::NotStarted,
        all: &["i can start this quest"],
    },
];

/// The stage a Dragon Slayer journal shows, or `None` when no rule matches.
pub fn resolve(lines: &[Arc<str>]) -> Option<DragonStage> {
    let text = normalize_journal(lines);
    RULES
        .iter()
        .find(|rule| rule.all.iter().all(|needle| text.contains(needle)))
        .map(|rule| rule.stage)
}

/// The compat stage value for one journal read: the Dragon Slayer stage for
/// that quest, `None` for any other quest or an unreadable journal.
pub fn journal_stage(quest: &str, lines: &[Arc<str>]) -> Option<i32> {
    (quest == QUEST_ID)
        .then(|| resolve(lines))
        .flatten()
        .map(DragonStage::value)
}

#[cfg(test)]
#[path = "dragon_stage_tests.rs"]
mod tests;
