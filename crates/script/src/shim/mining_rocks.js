import { host } from '../shim/_kernel.js';

export const ROCK_OPTIONS = [...((host().content && host().content.rock_type_names) || [])];

export const ROCK_TYPES = Object.fromEntries(ROCK_OPTIONS.map((n) => [n, []]));
export const QUEST_ROCK_TYPES = {};
export const GAS_ROCK_IDS = new Set();
export const GAS_ROCK_TICKS = 60;
export const BROKEN_PICKAXE = 'Broken pickaxe';

/**
 * Frozen `resolveRockIds` maps rock option names to their loc ids. No
 * selected-revision rock-id fact is published here, so this fails closed
 * instead of answering an empty set for a matching name.
 */
export function resolveRockIds(_names) {
    throw new Error('not impl: resolveRockIds: no selected rock-id facts');
}
