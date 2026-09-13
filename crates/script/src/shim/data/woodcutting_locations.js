// Thin native binding for the Ent quartet. IDs/lifetime live in Rust
// (`api::ent`); this module does not copy a JS game-data table.
export const ENT_NPC_IDS = new Set(
    globalThis.rustyscript.functions.__rs2b0t_ent_npc_ids(),
);
export const ENT_LIFE_TICKS = globalThis.rustyscript.functions.__rs2b0t_ent_life_ticks();

export function isEntNpcId(id) {
    return globalThis.rustyscript.functions.__rs2b0t_is_ent_npc_id(id);
}

export function entNpcOnTile(npcs, tile) {
    return globalThis.rustyscript.functions.__rs2b0t_ent_npc_on_tile(npcs, tile);
}
