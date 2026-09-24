import { host, notImplValue } from '../shim/_kernel.js';

export const ROCK_OPTIONS = [...((host().content && host().content.rock_type_names) || [])];

/**
 * No selected-revision rock-id fact is published here, so the id tables a
 * caller would mine from stay honest misses rather than empty lists.
 */
export const ROCK_TYPES = notImplValue('ROCK_TYPES');
export const QUEST_ROCK_TYPES = notImplValue('QUEST_ROCK_TYPES');
export const GAS_ROCK_IDS = notImplValue('GAS_ROCK_IDS');
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
