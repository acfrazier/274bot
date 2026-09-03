// Generated from tests/fixtures/js_declared_abi.json — do not edit by hand.
// Regen: cargo test -p script --test declared_abi regen_js_declared_abi -- --ignored
import { notV1, proxy } from '../../shim/_kernel.js';

export const defineBot = globalThis.defineBot;

export { Game } from '../../api/game/Game.js';
export { Inventory } from '../../api/inventory/Inventory.js';
export { Skills } from '../../api/skills/Skills.js';
export { Bank, withdrawOp } from '../../api/bank/Bank.js';
export { Banking, COMMON_BANK_LOOT, depositAllExcept, depositMatcher, matchesCommonBankLoot, PERIODIC_BANK_SETTINGS, parseBankStrategy } from '../../api/bank/Banking.js';
export { Execution } from '../../api/execution/Execution.js';
export { LoopingBot, TaskBot, TreeBot, AbstractBot } from '../../api/bot/Bot.js';
export { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export { Traversal } from '../../api/walking/Traversal.js';
export { DirectNavigator } from '../../api/walking/DirectNavigator.js';
export { default as Tile } from '../../geometry/Tile.js';
export { Npc, Npcs } from '../../api/npcs/Npcs.js';
export { Player, Players } from '../../api/players/Players.js';
export { Loc, Locs } from '../../api/locs/Locs.js';
export { GroundItem, GroundItems } from '../../api/grounditems/GroundItems.js';
export { Equipment } from '../../api/equipment/Equipment.js';
export { default as EntityQuery } from '../../api/query/Query.js';
export { nearestBank } from '../../api/bank/BankLocations.js';
export { PICKPOCKET_TARGETS, PICKPOCKET_TARGET_NAMES, ARDOUGNE_PICKPOCKET_TARGETS } from '../../data/pickpocketTargets.js';

export const NAV_PURE_WALK = { useTeleportCatalog: false, policy: { useTeleports: false } };
export const NAV_WITH_TELES = { useTeleportCatalog: true, policy: { useTeleports: true } };
export const ALL_FISHING_GEAR_NAMES = [];
export const AL_KHARID_BANK = '';
export const AXES = [];
export const AXE_BAR_FOR = [];
export const AXE_SHOP_COSTS = [];
export const AXE_SMITH_LEVEL = [];
export class AcquireTask {
    constructor() {}
    execute() { throw notV1('AcquireTask.execute'); }
    validate() { throw notV1('AcquireTask.validate'); }
}
export class Area {
    static circular() { throw notV1('Area.circular'); }
    contains() { throw notV1('Area.contains'); }
    getRandomTile() { throw notV1('Area.getRandomTile'); }
    static rectangular() { throw notV1('Area.rectangular'); }
    constructor() {}
}
export const BANK_LOCATIONS = [];
export const BOB_VENDOR = '';
export const BROKEN_AXE = '';
export const BROKEN_PICKAXE = '';
export class BranchTask {
    failure() { throw notV1('BranchTask.failure'); }
    success() { throw notV1('BranchTask.success'); }
    validate() { throw notV1('BranchTask.validate'); }
    constructor() {}
}
export const CHISEL = '';
export const COINS = '';
export const COW_LOCATIONS = [];
export const COW_LOCATION_OPTIONS = [];
export const DEFAULT_BOOTH_NAME = '';
export const DEFAULT_BOOTH_OP = '';
export const DEFAULT_RUNE = '';
export const FISHING_LOCATIONS = [];
export const FISHING_LOCATION_OPTIONS = [];
export const FISHING_METHODS = [];
export const FISHING_METHOD_OPTIONS = [];
export const FISHING_SHOP_COSTS = [];
export const FORGETFUL_BANK_ODDS = -1;
export const FORGETFUL_BANK_SETTING = '';
export const GAS_ROCK_IDS = [];
export const GAS_ROCK_TICKS = -1;
export const GERRANT_ONLY_FISHING = [];
export const GERRANT_VENDOR = '';
export const HAMMER = '';
export const HARRY_VENDOR = '';
export class InvItem {
    actions() { throw notV1('InvItem.actions'); }
    count() { throw notV1('InvItem.count'); }
    id() { throw notV1('InvItem.id'); }
    interact() { throw notV1('InvItem.interact'); }
    name() { throw notV1('InvItem.name'); }
    slot() { throw notV1('InvItem.slot'); }
    useOn() { throw notV1('InvItem.useOn'); }
    constructor() {}
}
export const KNIFE = '';
export class LeafTask {
    execute() { throw notV1('LeafTask.execute'); }
    constructor() {}
}
export const MAP_SQUARE = -1;
export const MINING_LOCATIONS = [];
export const MINING_LOCATION_OPTIONS = [];
export const MINING_LOCATION_OPTION_LABELS = [];
export const NEARBY_BANK_RADIUS = -1;
export const NEEDLE = '';
export const NURMOF_VENDOR = '';
export const PICKAXES = [];
export const PICKAXE_SHOP_COSTS = [];
export const Quests = proxy('Quests', {
    all() { throw notV1('Quests.all'); },
    journal() { throw notV1('Quests.journal'); },
    points() { throw notV1('Quests.points'); },
    status() { throw notV1('Quests.status'); },
});
export const RANDOM_EVENT_CASKET_ID = -1;
export const ROCK_OPTIONS = [];
export const ROCK_TYPES = [];
export const RUNES = [];
export const RUNE_OPTIONS = [];
export const Shop = proxy('Shop', {
    buy() { throw notV1('Shop.buy'); },
    buyById() { throw notV1('Shop.buyById'); },
    close() { throw notV1('Shop.close'); },
    isOpen() { throw notV1('Shop.isOpen'); },
    open() { throw notV1('Shop.open'); },
    sell() { throw notV1('Shop.sell'); },
    stock() { throw notV1('Shop.stock'); },
});
export const TINDERBOX = '';
export const TOLL_COIN_TARGET = -1;
export const TOOL_ACQUIRE_OPTIONS = [];
export const TOOL_ACQUIRE_SETTING = '';
export const Trade = proxy('Trade', {
    accept() { throw notV1('Trade.accept'); },
    active() { throw notV1('Trade.active'); },
    decline() { throw notV1('Trade.decline'); },
    myOffer() { throw notV1('Trade.myOffer'); },
    offer() { throw notV1('Trade.offer'); },
    offerAll() { throw notV1('Trade.offerAll'); },
    onConfirmScreen() { throw notV1('Trade.onConfirmScreen'); },
    onOfferScreen() { throw notV1('Trade.onOfferScreen'); },
    partner() { throw notV1('Trade.partner'); },
    removeAll() { throw notV1('Trade.removeAll'); },
    request() { throw notV1('Trade.request'); },
    theirOffer() { throw notV1('Trade.theirOffer'); },
});
export const VARROCK_ANVIL_BANK = '';
export const VARROCK_ANVIL_STAND = '';
export const WALK_DESTINATIONS = [];
export const WALK_OPTIONS = [];
export const WHIRLPOOL_IDS = [];
export const WOODCUTTING_LOCATIONS = [];
export const WOODCUTTING_LOCATION_OPTIONS = [];
export function acquireKeepNames() { throw notV1('acquireKeepNames'); }
export const apiVersion = 1;
export function axeReq() { throw notV1('axeReq'); }
export function axeShopOffers() { throw notV1('axeShopOffers'); }
export function bankDistance() { throw notV1('bankDistance'); }
export function bankHasBetterGatherTool() { throw notV1('bankHasBetterGatherTool'); }
export function bankUnlocked() { throw notV1('bankUnlocked'); }
export function bestAffordableShopTier() { throw notV1('bestAffordableShopTier'); }
export function bestAxe() { throw notV1('bestAxe'); }
export function bestFromTiers() { throw notV1('bestFromTiers'); }
export function bestHeldToolNames() { throw notV1('bestHeldToolNames'); }
export function bestOwnedTier() { throw notV1('bestOwnedTier'); }
export function bestPickaxe() { throw notV1('bestPickaxe'); }
export function bestSmithableAxe() { throw notV1('bestSmithableAxe'); }
export function boothFields() { throw notV1('boothFields'); }
export function buyPlansCost() { throw notV1('buyPlansCost'); }
export function canFundPlan() { throw notV1('canFundPlan'); }
export function canWieldTool() { throw notV1('canWieldTool'); }
export function coinsToWithdraw() { throw notV1('coinsToWithdraw'); }
export const events = proxy('events', {
    off() { throw notV1('events.off'); },
    on() { throw notV1('events.on'); },
});
export function exactTool() { throw notV1('exactTool'); }
export function fishingGearShopCart() { throw notV1('fishingGearShopCart'); }
export function fishingRestockPlan() { throw notV1('fishingRestockPlan'); }
export function fishingShopCost() { throw notV1('fishingShopCost'); }
export function fishingVendorFor() { throw notV1('fishingVendorFor'); }
export function gearKeepNames() { throw notV1('gearKeepNames'); }
export function gearLabel() { throw notV1('gearLabel'); }
export function hasAll() { throw notV1('hasAll'); }
export function hasAllTools() { throw notV1('hasAllTools'); }
export function hasFishingGear() { throw notV1('hasFishingGear'); }
export function hasToolReq() { throw notV1('hasToolReq'); }
export function held() { throw notV1('held'); }
export function isCowFieldLootTile() { throw notV1('isCowFieldLootTile'); }
export function isFishingBaitPiece() { throw notV1('isFishingBaitPiece'); }
export function locationOptions() { throw notV1('locationOptions'); }
export function miningLocationLabel() { throw notV1('miningLocationLabel'); }
export function missingFishingGear() { throw notV1('missingFishingGear'); }
export function missingToolLabels() { throw notV1('missingToolLabels'); }
export function nearestCowLocation() { throw notV1('nearestCowLocation'); }
export function nearestUsableBank() { throw notV1('nearestUsableBank'); }
export function needsTollCoins() { throw notV1('needsTollCoins'); }
export function parseToolAcquireMode() { throw notV1('parseToolAcquireMode'); }
export function pickaxeReq() { throw notV1('pickaxeReq'); }
export function pickaxeShopOffers() { throw notV1('pickaxeShopOffers'); }
export function planAxeAcquire() { throw notV1('planAxeAcquire'); }
export function planBrokenToolRepair() { throw notV1('planBrokenToolRepair'); }
export function planFishingGearAcquire() { throw notV1('planFishingGearAcquire'); }
export function planFishingGearBuys() { throw notV1('planFishingGearBuys'); }
export function planGatherToolAcquire() { throw notV1('planGatherToolAcquire'); }
export function planPickaxeAcquire() { throw notV1('planPickaxeAcquire'); }
export const reader = '';
export function registerScript() { throw notV1('registerScript'); }
export function resolveBankOpenRoute() { throw notV1('resolveBankOpenRoute'); }
export function resolveCowLocation() { throw notV1('resolveCowLocation'); }
export function resolveDestination() { throw notV1('resolveDestination'); }
export function resolveFishMethod() { throw notV1('resolveFishMethod'); }
export function resolveFishingLocation() { throw notV1('resolveFishingLocation'); }
export function resolveGatheringLocation() { throw notV1('resolveGatheringLocation'); }
export function resolveMiningLocation() { throw notV1('resolveMiningLocation'); }
export function resolveRockIds() { throw notV1('resolveRockIds'); }
export function resolveWoodcuttingLocation() { throw notV1('resolveWoodcuttingLocation'); }
export function sameMapSquare() { throw notV1('sameMapSquare'); }
export function shopableMissingFishingGear() { throw notV1('shopableMissingFishingGear'); }
export function shouldBankNow() { throw notV1('shouldBankNow'); }
export function shouldBootstrapTollCoins() { throw notV1('shouldBootstrapTollCoins'); }
export function spotMatchesMethod() { throw notV1('spotMatchesMethod'); }
export function surplusHeldToolNames() { throw notV1('surplusHeldToolNames'); }
export function tinderboxReq() { throw notV1('tinderboxReq'); }
export function toolAttackLevel() { throw notV1('toolAttackLevel'); }
export function toolKeepNames() { throw notV1('toolKeepNames'); }
export function toolKitLabel() { throw notV1('toolKitLabel'); }
export function toolRestockPlan() { throw notV1('toolRestockPlan'); }
export function toolsNeedingEquip() { throw notV1('toolsNeedingEquip'); }
export function withBaitTarget() { throw notV1('withBaitTarget'); }
