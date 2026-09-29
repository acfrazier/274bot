import { host, notImplValue } from '../shim/_kernel.js';

export const ROCK_OPTIONS = [...((host().content && host().content.rock_type_names) || [])];

/**
 * No selected-revision normal rock-id table is published here, so the ids a
 * caller would mine from stay an honest miss rather than an empty list.
 */
export const ROCK_TYPES = notImplValue('ROCK_TYPES');
export const QUEST_ROCK_TYPES = notImplValue('QUEST_ROCK_TYPES');
/** This isolate's own ordinary `Set`, built by Rust from the selected core (`load/selected_facts_v8.rs`). */
export const GAS_ROCK_IDS = globalThis.__rs2b0t_selected_facts('gas-rock-ids') ?? notImplValue('GAS_ROCK_IDS');
export const GAS_ROCK_TICKS = 60;
export const BROKEN_PICKAXE = 'Broken pickaxe';

/**
 * Frozen `resolveRockIds` maps rock option names to their loc ids. No
 * selected-revision normal rock-id table is published here, so this fails
 * closed instead of answering an empty set for a matching name.
 */
export function resolveRockIds(_names) {
    throw new Error('not impl: resolveRockIds: no selected rock-id facts');
}
