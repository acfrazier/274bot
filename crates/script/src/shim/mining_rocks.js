import { host, notImpl, notImplValue } from '../shim/_kernel.js';

export const ROCK_OPTIONS = [...((host().content && host().content.rock_type_names) || [])];

/**
 * No selected-revision normal rock-id table is published here, so the ids a
 * caller would mine from stay an honest miss rather than an empty list.
 */
export const ROCK_TYPES = notImplValue('ROCK_TYPES');
export const QUEST_ROCK_TYPES = notImplValue('QUEST_ROCK_TYPES');
/**
 * A name map onto Rust's `gas-rock-ids` Set. Nothing is read at module
 * evaluation: the selected gathering family is only reached when a script
 * actually touches the set, and each touch asks Rust (no JS-side copy).
 */
export const GAS_ROCK_IDS = new Proxy(new Set(), {
    get(_target, prop) {
        const ids = globalThis.__rs2b0t_selected_facts('gas-rock-ids');
        if (ids === undefined) {
            throw notImpl(typeof prop === 'string' ? 'GAS_ROCK_IDS.' + prop : 'GAS_ROCK_IDS');
        }
        const value = Reflect.get(ids, prop, ids);
        return typeof value === 'function' ? value.bind(ids) : value;
    },
});
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
