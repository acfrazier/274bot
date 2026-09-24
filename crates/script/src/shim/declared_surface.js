// Generated from tests/fixtures/js_declared_abi.json — do not edit by hand.
// Regen: cargo test -p script --test declared_abi regen_js_declared_abi -- --ignored
import { notImpl, proxy } from '../../shim/_kernel.js';

export const defineBot = globalThis.defineBot;

export { Game } from '../../api/game/Game.js';
export { Inventory } from '../../api/inventory/Inventory.js';
export { Skills } from '../../api/skills/Skills.js';
export { Bank, withdrawOp } from '../../api/bank/Bank.js';
export { Banking, COMMON_BANK_LOOT, depositAllExcept, depositMatcher, matchesCommonBankLoot, PERIODIC_BANK_SETTINGS, RANDOM_EVENT_CASKET_ID, parseBankStrategy } from '../../api/bank/Banking.js';
export { reader } from '../../adapter/ClientAdapter.js';
export { AXES, PICKAXES, TINDERBOX, HAMMER, KNIFE, CHISEL, NEEDLE, exactTool, tinderboxReq, axeReq, pickaxeReq, toolKeepNames, hasToolReq, hasAllTools, bestAxe, bestPickaxe, bestFromTiers, canWieldTool, toolRestockPlan, missingToolLabels, toolKitLabel, bankHasBetterGatherTool } from '../../api/acquisition/Tools.js';
export { Execution } from '../../api/execution/Execution.js';
export { LoopingBot, TaskBot, TreeBot, AbstractBot } from '../../api/bot/Bot.js';
export { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export { Quests } from '../../api/ui/questlog/Quests.js';
export { Traversal } from '../../api/walking/Traversal.js';
export { DirectNavigator } from '../../api/walking/DirectNavigator.js';
export { default as Tile } from '../../geometry/Tile.js';
export { Npc, Npcs } from '../../api/npcs/Npcs.js';
export { Player, Players } from '../../api/players/Players.js';
export { Loc, Locs } from '../../api/locs/Locs.js';
export { GroundItem, GroundItems } from '../../api/grounditems/GroundItems.js';
export { Equipment } from '../../api/equipment/Equipment.js';
export { Trade } from '../../api/trade/Trade.js';
export { Shop } from '../../api/shop/Shop.js';
export { default as EntityQuery } from '../../api/query/Query.js';
export { BANK_LOCATIONS, bankUnlocked, nearestBank } from '../../api/bank/BankLocations.js';
export { PICKPOCKET_TARGETS, PICKPOCKET_TARGET_NAMES, ARDOUGNE_PICKPOCKET_TARGETS } from '../../data/pickpocketTargets.js';
export { ENT_NPC_IDS, ENT_LIFE_TICKS, isEntNpcId, entNpcOnTile } from '../../data/woodcuttingLocations.js';
export { ROCK_OPTIONS, ROCK_TYPES, GAS_ROCK_IDS, GAS_ROCK_TICKS, BROKEN_PICKAXE, resolveRockIds } from '../../data/miningRocks.js';
export { COW_LOCATIONS, COW_LOCATION_OPTIONS, AL_KHARID_BANK, resolveCowLocation, nearestCowLocation } from '../../data/cowKillerLocations.js';
export { RUNES, RUNE_OPTIONS, DEFAULT_RUNE } from '../../data/runeCraftLocations.js';

export const NAV_PURE_WALK = { useTeleportCatalog: false, policy: { useTeleports: false } };
export const NAV_WITH_TELES = { useTeleportCatalog: true, policy: { useTeleports: true } };
export const ALL_FISHING_GEAR_NAMES = proxy('ALL_FISHING_GEAR_NAMES', {});
export const AXE_BAR_FOR = proxy('AXE_BAR_FOR', {});
export const AXE_SHOP_COSTS = proxy('AXE_SHOP_COSTS', {});
export const AXE_SMITH_LEVEL = proxy('AXE_SMITH_LEVEL', {});
export class AcquireTask {
    constructor() {}
    execute() { throw notImpl('AcquireTask.execute'); }
    validate() { throw notImpl('AcquireTask.validate'); }
}
export class Area {
    static circular() { throw notImpl('Area.circular'); }
    contains() { throw notImpl('Area.contains'); }
    getRandomTile() { throw notImpl('Area.getRandomTile'); }
    static rectangular() { throw notImpl('Area.rectangular'); }
    constructor() {}
}
export const BOB_VENDOR = proxy('BOB_VENDOR', {});
export const BROKEN_AXE = proxy('BROKEN_AXE', {});
export class BranchTask {
    failure() { throw notImpl('BranchTask.failure'); }
    success() { throw notImpl('BranchTask.success'); }
    validate() { throw notImpl('BranchTask.validate'); }
    constructor() {}
}
export const COINS = proxy('COINS', {});
export const DEFAULT_BOOTH_NAME = proxy('DEFAULT_BOOTH_NAME', {});
export const DEFAULT_BOOTH_OP = proxy('DEFAULT_BOOTH_OP', {});
export const FISHING_LOCATIONS = proxy('FISHING_LOCATIONS', {});
export const FISHING_LOCATION_OPTIONS = proxy('FISHING_LOCATION_OPTIONS', {});
export const FISHING_METHODS = proxy('FISHING_METHODS', {});
export const FISHING_METHOD_OPTIONS = proxy('FISHING_METHOD_OPTIONS', {});
export const FISHING_SHOP_COSTS = proxy('FISHING_SHOP_COSTS', {});
export const FORGETFUL_BANK_ODDS = proxy('FORGETFUL_BANK_ODDS', {});
export const FORGETFUL_BANK_SETTING = proxy('FORGETFUL_BANK_SETTING', {});
export const GERRANT_ONLY_FISHING = proxy('GERRANT_ONLY_FISHING', {});
export const GERRANT_VENDOR = proxy('GERRANT_VENDOR', {});
export const HARRY_VENDOR = proxy('HARRY_VENDOR', {});
export class InvItem {
    actions() { throw notImpl('InvItem.actions'); }
    count() { throw notImpl('InvItem.count'); }
    id() { throw notImpl('InvItem.id'); }
    interact() { throw notImpl('InvItem.interact'); }
    name() { throw notImpl('InvItem.name'); }
    slot() { throw notImpl('InvItem.slot'); }
    useOn() { throw notImpl('InvItem.useOn'); }
    constructor() {}
}
export class LeafTask {
    execute() { throw notImpl('LeafTask.execute'); }
    constructor() {}
}
export const MAP_SQUARE = proxy('MAP_SQUARE', {});
export const MINING_LOCATIONS = proxy('MINING_LOCATIONS', {});
export const MINING_LOCATION_OPTIONS = proxy('MINING_LOCATION_OPTIONS', {});
export const MINING_LOCATION_OPTION_LABELS = proxy('MINING_LOCATION_OPTION_LABELS', {});
export const NEARBY_BANK_RADIUS = proxy('NEARBY_BANK_RADIUS', {});
export const NURMOF_VENDOR = proxy('NURMOF_VENDOR', {});
export const PICKAXE_SHOP_COSTS = proxy('PICKAXE_SHOP_COSTS', {});
export const TOLL_COIN_TARGET = proxy('TOLL_COIN_TARGET', {});
export const TOOL_ACQUIRE_OPTIONS = proxy('TOOL_ACQUIRE_OPTIONS', {});
export const TOOL_ACQUIRE_SETTING = proxy('TOOL_ACQUIRE_SETTING', {});
export const VARROCK_ANVIL_BANK = proxy('VARROCK_ANVIL_BANK', {});
export const VARROCK_ANVIL_STAND = proxy('VARROCK_ANVIL_STAND', {});
export const WALK_DESTINATIONS = proxy('WALK_DESTINATIONS', {});
export const WALK_OPTIONS = proxy('WALK_OPTIONS', {});
export const WHIRLPOOL_IDS = proxy('WHIRLPOOL_IDS', {});
export const WOODCUTTING_LOCATIONS = proxy('WOODCUTTING_LOCATIONS', {});
export const WOODCUTTING_LOCATION_OPTIONS = proxy('WOODCUTTING_LOCATION_OPTIONS', {});
export function acquireKeepNames() { throw notImpl('acquireKeepNames'); }
export const apiVersion = 1;
export function axeShopOffers() { throw notImpl('axeShopOffers'); }
export function bankDistance() { throw notImpl('bankDistance'); }
export function bestAffordableShopTier() { throw notImpl('bestAffordableShopTier'); }
export function bestHeldToolNames() { throw notImpl('bestHeldToolNames'); }
export function bestOwnedTier() { throw notImpl('bestOwnedTier'); }
export function bestSmithableAxe() { throw notImpl('bestSmithableAxe'); }
export function boothFields() { throw notImpl('boothFields'); }
export function buyPlansCost() { throw notImpl('buyPlansCost'); }
export function canFundPlan() { throw notImpl('canFundPlan'); }
export function coinsToWithdraw() { throw notImpl('coinsToWithdraw'); }
export const events = proxy('events', {
    off() { throw notImpl('events.off'); },
    on() { throw notImpl('events.on'); },
});
export function fishingGearShopCart() { throw notImpl('fishingGearShopCart'); }
export function fishingRestockPlan() { throw notImpl('fishingRestockPlan'); }
export function fishingShopCost() { throw notImpl('fishingShopCost'); }
export function fishingVendorFor() { throw notImpl('fishingVendorFor'); }
export function gearKeepNames() { throw notImpl('gearKeepNames'); }
export function gearLabel() { throw notImpl('gearLabel'); }
export function hasAll() { throw notImpl('hasAll'); }
export function hasFishingGear() { throw notImpl('hasFishingGear'); }
export function held() { throw notImpl('held'); }
export function isCowFieldLootTile() { throw notImpl('isCowFieldLootTile'); }
export function isFishingBaitPiece() { throw notImpl('isFishingBaitPiece'); }
export function locationOptions() { throw notImpl('locationOptions'); }
export function miningLocationLabel() { throw notImpl('miningLocationLabel'); }
export function missingFishingGear() { throw notImpl('missingFishingGear'); }
export function nearestUsableBank() { throw notImpl('nearestUsableBank'); }
export function needsTollCoins() { throw notImpl('needsTollCoins'); }
export function parseToolAcquireMode() { throw notImpl('parseToolAcquireMode'); }
export function pickaxeShopOffers() { throw notImpl('pickaxeShopOffers'); }
export function planAxeAcquire() { throw notImpl('planAxeAcquire'); }
export function planBrokenToolRepair() { throw notImpl('planBrokenToolRepair'); }
export function planFishingGearAcquire() { throw notImpl('planFishingGearAcquire'); }
export function planFishingGearBuys() { throw notImpl('planFishingGearBuys'); }
export function planGatherToolAcquire() { throw notImpl('planGatherToolAcquire'); }
export function planPickaxeAcquire() { throw notImpl('planPickaxeAcquire'); }
export function registerScript() { throw notImpl('registerScript'); }
export function resolveBankOpenRoute() { throw notImpl('resolveBankOpenRoute'); }
export function resolveDestination() { throw notImpl('resolveDestination'); }
export function resolveFishMethod() { throw notImpl('resolveFishMethod'); }
export function resolveFishingLocation() { throw notImpl('resolveFishingLocation'); }
export function resolveGatheringLocation() { throw notImpl('resolveGatheringLocation'); }
export function resolveMiningLocation() { throw notImpl('resolveMiningLocation'); }
export function resolveWoodcuttingLocation() { throw notImpl('resolveWoodcuttingLocation'); }
export function sameMapSquare() { throw notImpl('sameMapSquare'); }
export function shopableMissingFishingGear() { throw notImpl('shopableMissingFishingGear'); }
export function shouldBankNow() { throw notImpl('shouldBankNow'); }
export function shouldBootstrapTollCoins() { throw notImpl('shouldBootstrapTollCoins'); }
export function spotMatchesMethod() { throw notImpl('spotMatchesMethod'); }
export function surplusHeldToolNames() { throw notImpl('surplusHeldToolNames'); }
export function toolAttackLevel() { throw notImpl('toolAttackLevel'); }
export function toolsNeedingEquip() { throw notImpl('toolsNeedingEquip'); }
export function withBaitTarget() { throw notImpl('withBaitTarget'); }
