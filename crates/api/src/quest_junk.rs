//! The bank quest-junk table of the rs2b0t script API (`api/bank/bankQuestJunk.ts:15-`): quest items that stay in a
//! bank after their quest, in the upstream table's order. Generated from the pinned source by
//! `tools/game-data/generate-gather-catalogs.ts` (an AST read; nothing is evaluated). The table is an immutable
//! process-wide static; the bank family's `findQuestJunk` reads it.

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

include!("../data/game-data/quest-junk-catalog.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_keeps_the_pinned_rows_in_source_order() {
        assert_eq!(QUEST_JUNK.len(), 319);
        assert_eq!(
            QUEST_JUNK[0],
            QuestJunkEntry {
                id: 2399,
                name: "Key",
                quest: "Demon Slayer"
            }
        );
        assert!(QUEST_JUNK.iter().any(|row| row.quest == "Romeo & Juliet"));
        assert!(QUEST_JUNK
            .iter()
            .all(|row| row.id >= 0 && !row.name.is_empty() && !row.quest.is_empty()));
    }
}
