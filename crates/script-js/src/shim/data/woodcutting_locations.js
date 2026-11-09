// Thin native binding. The camp table is the immutable Rust table (`api::gather_camps`), materialized once per
// isolate; the Ent ids and lifetime are `api::ent`. This module copies no JS game-data table.
import Tile from '../geometry/Tile.js';
import { locationOptions, resolveGatheringLocation } from '../shim/gathering_locations.js';

export const WOODCUTTING_LOCATIONS = globalThis.__rs2b0t_gather_catalog(0, Tile);
export const WOODCUTTING_LOCATION_OPTIONS = locationOptions(WOODCUTTING_LOCATIONS);

export function resolveWoodcuttingLocation(setting, startTile) {
    return resolveGatheringLocation(setting, startTile, WOODCUTTING_LOCATIONS);
}

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
