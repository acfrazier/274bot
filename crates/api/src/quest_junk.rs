//! The bank quest-junk table of the rs2b0t script API (`api/bank/bankQuestJunk.ts:15-`): quest items that stay in a
//! bank after their quest, in the upstream table's order. Generated from the rs2b0t source into
//! `gather-catalog.json` by `tools/game-data/generate-gather-catalogs.ts` (an AST read; nothing is evaluated) and
//! compiled by this crate's build script. The table is an immutable process-wide static; the bank family's
//! `findQuestJunk` reads it.

use crate::selected::{embedded_table, FactError};

/// One quest-junk row (`QuestJunkEntry`, `bankQuestJunk.ts:3-7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestJunkEntry {
    /// Obj id.
    pub id: i32,
    /// Item name.
    pub name: &'static str,
    /// The quest the item belongs to.
    pub quest: &'static str,
}

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
mod generated {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/quest_junk.rs"));
}

/// Every quest-junk row in the upstream table's order.
pub fn quest_junk() -> Result<&'static [QuestJunkEntry], FactError> {
    embedded_table!(generated::QUEST_JUNK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_keeps_the_pinned_rows_in_source_order() {
        if !crate::test_support::embedded_game_data_available() {
            return;
        }
        let rows = quest_junk().expect("quest junk");
        assert_eq!(rows.len(), 319);
        assert_eq!(
            rows[0],
            QuestJunkEntry {
                id: 2399,
                name: "Key",
                quest: "Demon Slayer"
            }
        );
        assert!(rows.iter().any(|row| row.quest == "Romeo & Juliet"));
        assert!(rows
            .iter()
            .all(|row| row.id >= 0 && !row.name.is_empty() && !row.quest.is_empty()));
    }
}
