// Generated from tests/fixtures/js_declared_abi.json — do not edit by hand.
// Regen: cargo test -p script --test declared_abi regen_js_declared_abi -- --ignored
import { notImpl, notImplValue, proxy } from '../../shim/_kernel.js';

export const defineBot = globalThis.defineBot;

export { registerScript } from '../../runtime/ScriptRegistry.js';

export { Game } from '../../api/game/Game.js';
export { Inventory, InvItem } from '../../api/inventory/Inventory.js';
export { Skills } from '../../api/skills/Skills.js';
export { Bank, withdrawOp } from '../../api/bank/Bank.js';
export { Banking, COMMON_BANK_LOOT, depositAllExcept, depositMatcher, matchesCommonBankLoot, PERIODIC_BANK_SETTINGS, RANDOM_EVENT_CASKET_ID, NEARBY_BANK_RADIUS, resolveBankOpenRoute, parseBankStrategy, shouldBankNow } from '../../api/bank/Banking.js';
export { bankDistance, nearestUsableBank } from '../../api/bank/BankLocations.js';
export { reader } from '../../adapter/ClientAdapter.js';
export { DEFAULT_BOOTH_NAME, DEFAULT_BOOTH_OP, MAP_SQUARE, sameMapSquare, locationOptions, boothFields, resolveGatheringLocation } from '../../shim/gathering_locations.js';
export { AXES, PICKAXES, TINDERBOX, HAMMER, KNIFE, CHISEL, NEEDLE, exactTool, tinderboxReq, axeReq, pickaxeReq, toolKeepNames, hasToolReq, hasAllTools, bestAxe, bestPickaxe, bestFromTiers, canWieldTool, toolAttackLevel, toolsNeedingEquip, toolRestockPlan, missingToolLabels, toolKitLabel, bankHasBetterGatherTool, bestHeldToolNames, surplusHeldToolNames } from '../../api/acquisition/Tools.js';
export { Execution } from '../../api/execution/Execution.js';
export { LoopingBot, TaskBot, TreeBot, AbstractBot, BranchTask, LeafTask } from '../../api/bot/Bot.js';
export { Special } from '../../api/combat/Special.js';
export { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export { Quests } from '../../api/ui/questlog/Quests.js';
export { Traversal } from '../../api/walking/Traversal.js';
export { DirectNavigator } from '../../api/walking/DirectNavigator.js';
export { default as Tile } from '../../geometry/Tile.js';
export { Area } from '../../geometry/Area.js';
export { Npc, Npcs } from '../../api/npcs/Npcs.js';
export { Player, Players } from '../../api/players/Players.js';
export { Loc, Locs } from '../../api/locs/Locs.js';
export { GroundItem, GroundItems } from '../../api/grounditems/GroundItems.js';
export { Equipment } from '../../api/equipment/Equipment.js';
export { Trade } from '../../api/trade/Trade.js';
export { Shop } from '../../api/shop/Shop.js';
export { default as EntityQuery } from '../../api/query/Query.js';
export { BANK_LOCATIONS, bankUnlocked, nearestBank } from '../../api/bank/BankLocations.js';
export { NAV_PURE_WALK, NAV_WITH_TELES } from '../../api/walking/Traversal.js';
export { PICKPOCKET_TARGETS, PICKPOCKET_TARGET_NAMES, ARDOUGNE_PICKPOCKET_TARGETS } from '../../data/pickpocketTargets.js';
export { ENT_NPC_IDS, ENT_LIFE_TICKS, isEntNpcId, entNpcOnTile, WOODCUTTING_LOCATIONS, WOODCUTTING_LOCATION_OPTIONS, resolveWoodcuttingLocation } from '../../data/woodcuttingLocations.js';
export { MINING_LOCATIONS, MINING_LOCATION_OPTIONS, MINING_LOCATION_OPTION_LABELS, miningLocationLabel, resolveMiningLocation } from '../../data/miningLocations.js';
export { FISHING_LOCATIONS, FISHING_LOCATION_OPTIONS, resolveFishingLocation } from '../../data/fishingLocations.js';
export { WHIRLPOOL_IDS, FISHING_METHODS, FISHING_METHOD_OPTIONS, ALL_FISHING_GEAR_NAMES, resolveFishMethod, gearKeepNames, hasFishingGear, missingFishingGear, gearLabel, fishingRestockPlan, spotMatchesMethod } from '../../data/fishingMethods.js';
export { FISHING_SHOP_COSTS, GERRANT_ONLY_FISHING, fishingVendorFor, fishingShopCost, isFishingBaitPiece, withBaitTarget, planFishingGearBuys, planFishingGearAcquire, buyPlansCost, fishingGearShopCart, shopableMissingFishingGear } from '../../shim/fishing_gear.js';
export { ROCK_OPTIONS, ROCK_TYPES, GAS_ROCK_IDS, GAS_ROCK_TICKS, BROKEN_PICKAXE, resolveRockIds } from '../../data/miningRocks.js';
export { COW_LOCATIONS, COW_LOCATION_OPTIONS, AL_KHARID_BANK, TOLL_COIN_TARGET, isCowFieldLootTile, resolveCowLocation, nearestCowLocation, needsTollCoins, shouldBootstrapTollCoins } from '../../data/cowKillerLocations.js';
export { RUNES, RUNE_OPTIONS, DEFAULT_RUNE } from '../../data/runeCraftLocations.js';
export { WALK_DESTINATIONS, WALK_OPTIONS, resolveDestination } from '../../api/map/WalkDestinations.js';

export const AXE_BAR_FOR = notImplValue('AXE_BAR_FOR');
export const AXE_SHOP_COSTS = notImplValue('AXE_SHOP_COSTS');
export const AXE_SMITH_LEVEL = notImplValue('AXE_SMITH_LEVEL');
export class AcquireTask {
    constructor() {}
    execute() { throw notImpl('AcquireTask.execute'); }
    validate() { throw notImpl('AcquireTask.validate'); }
}
export const BOB_VENDOR = notImplValue('BOB_VENDOR');
export const BROKEN_AXE = notImplValue('BROKEN_AXE');
export const COINS = notImplValue('COINS');
export const FORGETFUL_BANK_ODDS = notImplValue('FORGETFUL_BANK_ODDS');
export const FORGETFUL_BANK_SETTING = notImplValue('FORGETFUL_BANK_SETTING');
export const GERRANT_VENDOR = notImplValue('GERRANT_VENDOR');
export const HARRY_VENDOR = notImplValue('HARRY_VENDOR');
export const NURMOF_VENDOR = notImplValue('NURMOF_VENDOR');
export const PICKAXE_SHOP_COSTS = notImplValue('PICKAXE_SHOP_COSTS');
export const TOOL_ACQUIRE_OPTIONS = notImplValue('TOOL_ACQUIRE_OPTIONS');
export const TOOL_ACQUIRE_SETTING = notImplValue('TOOL_ACQUIRE_SETTING');
export const VARROCK_ANVIL_BANK = notImplValue('VARROCK_ANVIL_BANK');
export const VARROCK_ANVIL_STAND = notImplValue('VARROCK_ANVIL_STAND');
export function acquireKeepNames() { throw notImpl('acquireKeepNames'); }
export const apiVersion = 1;
export function axeShopOffers() { throw notImpl('axeShopOffers'); }
export function bestAffordableShopTier() { throw notImpl('bestAffordableShopTier'); }
export function bestOwnedTier() { throw notImpl('bestOwnedTier'); }
export function bestSmithableAxe() { throw notImpl('bestSmithableAxe'); }
export function canFundPlan() { throw notImpl('canFundPlan'); }
export function coinsToWithdraw() { throw notImpl('coinsToWithdraw'); }
export const events = proxy('events', {
    off(event, cb) { globalThis.__rs2b0t_native_event_off(event, cb); },
    on(event, cb) { const id = globalThis.__rs2b0t_native_event_on(event, cb); return () => globalThis.__rs2b0t_native_event_off(id); },
});
export function hasAll() { throw notImpl('hasAll'); }
export function held() { throw notImpl('held'); }
export function parseToolAcquireMode() { throw notImpl('parseToolAcquireMode'); }
export function pickaxeShopOffers() { throw notImpl('pickaxeShopOffers'); }
export function planAxeAcquire() { throw notImpl('planAxeAcquire'); }
export function planBrokenToolRepair() { throw notImpl('planBrokenToolRepair'); }
export function planGatherToolAcquire() { throw notImpl('planGatherToolAcquire'); }
export function planPickaxeAcquire() { throw notImpl('planPickaxeAcquire'); }
