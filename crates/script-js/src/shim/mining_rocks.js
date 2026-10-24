import { notImplValue } from '../shim/_kernel.js';

const rockData = globalThis.__rs2b0t_selected_facts('mining-rock-data');

export const ROCK_OPTIONS = rockData.rockOptions;
export const ROCK_TYPES = rockData.rockTypes;
export const QUEST_ROCK_TYPES = rockData.questRockTypes;
export const ROCK_TIER_ORDER = rockData.rockTierOrder;
export const GAS_ROCK_IDS = globalThis.__rs2b0t_selected_facts('gas-rock-ids') ?? notImplValue('GAS_ROCK_IDS');
export const GAS_ROCK_TICKS = rockData.gasRockTicks;
export const BROKEN_PICKAXE = rockData.brokenPickaxe;

export function rockTierById(id) {
    return globalThis.__rs2b0t_selected_facts('rock-tier-by-id', id);
}

export function resolveRockIds(names) {
    return globalThis.__rs2b0t_selected_facts('resolve-rock-ids', names);
}
