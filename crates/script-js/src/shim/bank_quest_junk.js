// The table and the matching are Rust (`api::quest_junk`); `statusOf` stays
// the caller's callback, called once per matched entry in table order.
export const QUEST_JUNK = globalThis.__rs2b0t_quest_junk_table();

export function findQuestJunk(banked, statusOf) {
    return globalThis.__rs2b0t_find_quest_junk(banked, statusOf);
}
