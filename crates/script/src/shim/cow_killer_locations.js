import Tile from '../geometry/Tile.js';

const selectedFacts = globalThis.__rs2b0t_selected_facts;
const facts = selectedFacts('cow-locations', Tile);

export const COW_LOCATIONS = facts.locations;
export const COW_LOCATION_OPTIONS = facts.options;
export const DRAYNOR_BANK = facts.draynorBank;
export const FALADOR_EAST_BANK = facts.faladorEastBank;
export const ARDOUGNE_WEST_BANK = facts.ardougneWestBank;
export const AL_KHARID_BANK = facts.alKharidBank;
export const TOLL_COIN_TARGET = facts.tollCoinTarget;

export function isCowFieldLootTile(anchor, leashRadius, tile) {
    return selectedFacts('cow-loot-tile', anchor, leashRadius, tile);
}

export function resolveCowLocation(setting, start) {
    return selectedFacts('cow-resolve', setting, start, COW_LOCATIONS);
}

export function nearestCowLocation(tile) {
    return selectedFacts('cow-nearest', tile, COW_LOCATIONS);
}

export function needsTollCoins(location, enabled) {
    return selectedFacts('cow-needs-toll', location, enabled);
}

export function cowBankDestination(location, tollEnabled) {
    return selectedFacts('cow-bank-destination', location, tollEnabled, AL_KHARID_BANK);
}

export function shouldBootstrapTollCoins(location, start, coins, enabled) {
    return selectedFacts('cow-bootstrap-toll', location, start, coins, enabled);
}
