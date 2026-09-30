// Generated from the JS shim name maps (shim_modules + prelude) — do not edit by hand.
// Regen: cargo test -p script --test compat_dts regen_compat_dts -- --ignored
// Compat (JS API v1) as the isolate exposes it. O-SCRIPT-API appends via DtsExtension.
// Not a clone of rs2b0t-api; stubs are declared ABI names with no shim owner.

declare module '@rs2b0t/api' {
  export function defineBot(manifest: any): any;

  export const ALL_FISHING_GEAR_NAMES: any;

  export const AL_KHARID_BANK: any;

  export const ARDOUGNE_PICKPOCKET_TARGETS: any;

  export const AXES: any;

  export const AXE_BAR_FOR: any;

  export const AXE_SHOP_COSTS: any;

  export const AXE_SMITH_LEVEL: any;

  export class AbstractBot {
    loopDelay: any;
    loopCadence: any;
    onStart(): any;
    onStop(): any;
    onPause(): any;
    onResume(): any;
    onPaint(): any;
    loop(): any;
    recoveryAnchor(): any;
    grindTargets(): any;
    ignoredRandoms(): any;
    on(event: any, cb: any): any;
    log(message: any): any;
    get settings(): any;
  }

  export class AcquireTask {
    constructor();
    execute(...args: any): any;
    validate(...args: any): any;
  }

  export class Area {
    constructor();
    static circular(...args: any): any;
    contains(...args: any): any;
    getRandomTile(...args: any): any;
    static rectangular(...args: any): any;
  }

  export const BANK_LOCATIONS: any;

  export const BOB_VENDOR: any;

  export const BROKEN_AXE: any;

  export const BROKEN_PICKAXE: any;

  export const Bank: {
    isOpen(): any;
    loaded(): any;
    ready(): any;
    items(): any;
    count(name: any): any;
    deposit(name: any): any;
    depositInventory(): Promise<any>;
    depositAllMatching(predicate: any, log: any): Promise<any>;
    depositAllExcept(keep: any): Promise<any>;
    withdraw(name: any, amount: any): any;
    setNoteMode(on: any): Promise<any>;
    close(): Promise<any>;
    withdrawById(id: any, op?: any): any;
    withdrawX(name: any, count: any): any;
    withdrawXById(id: any, count: any, landsAsId?: any): any;
    openBooth(stand: any, boothName: any, op: any, _log: any): Promise<any>;
    openNearest(boothName: any, op: any, log: any): Promise<any>;
    openNearestWorld(): Promise<any>;
    waitReady(timeoutMs: any, log: any): Promise<any>;
    snapshotReady(): any;
    snapshotGeneration(): any;
    waitSnapshotAfter(generation: any, timeoutMs: any): Promise<any>;
    countById(id: any): any;
    withdrawLoad(name: any): Promise<any>;
    openNearestAccess(access: any, log: any): Promise<any>;
    openNpcAccess(access: any, log: any): Promise<any>;
  };

  export const Banking: {
    open(opts?: any): Promise<any>;
    bankNearest(opts?: any): Promise<any>;
  };

  export class BranchTask {
    constructor();
    failure(...args: any): any;
    success(...args: any): any;
    validate(...args: any): any;
  }

  export const CHISEL: any;

  export const COINS: any;

  export const COMMON_BANK_LOOT: any;

  export const COW_LOCATIONS: any;

  export const COW_LOCATION_OPTIONS: any;

  export const ChatDialog: {
    isOpen(): any;
    canContinue(): any;
    options(): any;
    texts(): any;
    isMakeMenu(): any;
    makeProducts(): any;
    make(match: any): Promise<any>;
    makeX(match: any, count: any): Promise<any>;
    continue(): Promise<any>;
    chooseOption(match: any): Promise<any>;
    isMainMakePanel(): any;
    mainMakeProducts(): any;
    makeFromPanel(match: any, op: any): Promise<any>;
    makeFromPanelMax(match: any): Promise<any>;
    makeOne(): Promise<any>;
  };

  export const DEFAULT_BOOTH_NAME: any;

  export const DEFAULT_BOOTH_OP: any;

  export const DEFAULT_RUNE: any;

  export const DirectNavigator: {
    walk(dest: any): any;
    walkTo(dest: any, radius?: any, timeoutMs?: any): Promise<any>;
  };

  export const ENT_LIFE_TICKS: any;

  export const ENT_NPC_IDS: any;

  export class EntityQuery {
    constructor(supplySnaps: any, wrap: any);
    supplySnaps: any;
    wrap: any;
    snapFilters: any;
    entityFilters: any;
    static fromSnapshots(supply: any, wrap: any): any;
    name(...names: any): any;
    action(action: any): any;
    within(dist: any): any;
    withinOf(origin: any, dist: any): any;
    where(pred: any): any;
    forEachMatch(seen: any): any;
    results(): any;
    nearest(): any;
    first(): any;
    exists(): any;
    count(): any;
    inside(_area: any): any;
    nearestPreferLocal(_preferRadius: any): any;
  }

  export const Equipment: {
    items(): any;
    contains(name: any): any;
    equip(name: any): Promise<any>;
    unequip(name: any): Promise<any>;
  };

  export const Execution: {
    delay(ms: any): Promise<any>;
    delayTicks(n: any): Promise<any>;
    delayUntil(cond: any, timeoutMs?: any): any;
    delayUntilTicks(cond: any, maxTicks: any): any;
    noteProgress(): any;
  };

  export const FISHING_LOCATIONS: any;

  export const FISHING_LOCATION_OPTIONS: any;

  export const FISHING_METHODS: any;

  export const FISHING_METHOD_OPTIONS: any;

  export const FISHING_SHOP_COSTS: any;

  export const FORGETFUL_BANK_ODDS: any;

  export const FORGETFUL_BANK_SETTING: any;

  export const GAS_ROCK_IDS: any;

  export const GAS_ROCK_TICKS: any;

  export const GERRANT_ONLY_FISHING: any;

  export const GERRANT_VENDOR: any;

  export const Game: {
    ingame(): any;
    tile(): any;
    tick(): any;
    inCombat(): any;
    animating(): any;
    runEnabled(): any;
    autoRetaliate(): any;
    autoRetaliateOn(): any;
    myName(): any;
    combatMode(): any;
    combatStyles(): any;
    hasCombatStyle(style: any): any;
    combatStyleResolution(style: any): any;
    setCombatMode(mode: any): any;
    setCombatStyle(style: any): any;
    setAutoRetaliate(on: any): any;
    openSideTab(tab: any): Promise<any>;
    castOnItem(spell: any, item: any): Promise<any>;
    castOnLoc(spell: any, loc: any): Promise<any>;
    teleport(name: any): Promise<any>;
    energy(): any;
    weight(): any;
    cameraYaw(): any;
    cameraPitch(): any;
    setCameraYaw(yaw: any): any;
    combatStyleMode(): any;
    sceneReady(): any;
    sceneState(): any;
    attackedByPlayer(): any;
    castOnNpc(): Promise<any>;
  };

  export class GroundItem {
    constructor(row: any);
    snap: any;
    get name(): any;
    get id(): any;
    get count(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    interact(action: any): any;
  }

  export const GroundItems: {
    query(): any;
  };

  export const HAMMER: any;

  export const HARRY_VENDOR: any;

  export class InvItem {
    constructor();
    actions(...args: any): any;
    count(...args: any): any;
    id(...args: any): any;
    interact(...args: any): any;
    name(...args: any): any;
    slot(...args: any): any;
    useOn(...args: any): any;
  }

  export const Inventory: {
    count(name: any): any;
    countById(id: any): any;
    first(name: any): any;
    items(): any;
    contains(name: any): any;
    used(): any;
    isFull(): any;
    free(): any;
  };

  export const KNIFE: any;

  export class LeafTask {
    constructor();
    execute(...args: any): any;
  }

  export class Loc {
    constructor(row: any);
    snap: any;
    get name(): any;
    get id(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    interact(action: any): any;
  }

  export const Locs: {
    query(): any;
  };

  export class LoopingBot {
    loopDelay: any;
    loopCadence: any;
    onStart(): any;
    onStop(): any;
    onPause(): any;
    onResume(): any;
    onPaint(): any;
    loop(): any;
    recoveryAnchor(): any;
    grindTargets(): any;
    ignoredRandoms(): any;
    on(event: any, cb: any): any;
    log(message: any): any;
    get settings(): any;
  }

  export const MAP_SQUARE: any;

  export const MINING_LOCATIONS: any;

  export const MINING_LOCATION_OPTIONS: any;

  export const MINING_LOCATION_OPTION_LABELS: any;

  export const NAV_PURE_WALK: {
    policy(...args: any): any;
    useTeleportCatalog(...args: any): any;
  };

  export const NAV_WITH_TELES: {
    policy(...args: any): any;
    useTeleportCatalog(...args: any): any;
  };

  export const NEARBY_BANK_RADIUS: any;

  export const NEEDLE: any;

  export const NURMOF_VENDOR: any;

  export class Npc {
    constructor(row: any);
    snap: any;
    get name(): any;
    get id(): any;
    get index(): any;
    get inCombat(): any;
    get health(): any;
    get level(): any;
    get size(): any;
    networkOrigin(): any;
    networkTile(): any;
    targetsMe(): any;
    targetsAnotherPlayer(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    valid(): any;
    interact(action: any): any;
  }

  export const Npcs: {
    query(): any;
    all(): any;
    nearest(count?: any): any;
  };

  export const PERIODIC_BANK_SETTINGS: {
    bankStrategy: any;
    bankEveryItems: any;
    bankEveryMinutes: any;
    bankCommonJunk: any;
  };

  export const PICKAXES: any;

  export const PICKAXE_SHOP_COSTS: any;

  export const PICKPOCKET_TARGETS: any;

  export const PICKPOCKET_TARGET_NAMES: any;

  export class Player {
    constructor(row: any);
    snap: any;
    get name(): any;
    get index(): any;
    get inCombat(): any;
    get combatLevel(): any;
    targetsMe(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    interact(action: any): any;
  }

  export const Players: {
    query(): any;
    all(): any;
  };

  export const Quests: {
    all(): any;
    status(name: any): any;
    journal(): any;
    points(): any;
  };

  export const RANDOM_EVENT_CASKET_ID: any;

  export const ROCK_OPTIONS: any;

  export const ROCK_TYPES: any;

  export const RUNES: any;

  export const RUNE_OPTIONS: any;

  export const Shop: {
    isOpen(): any;
    stock(): any;
    player(): any;
    open(npcName: any): any;
    buy(name: any, qty: any): any;
    sell(name: any, qty: any, pick: any): any;
    sellAll(name: any, pick: any): any;
    close(): any;
    buyById(): any;
  };

  export const Skills: {
    index(name: any): any;
    xp(name: any): any;
    level(name: any): any;
    effective(name: any): any;
    hpFraction(): any;
  };

  export const Special: {
    energy(): any;
    armed(): any;
    wielded(): any;
    cost(weaponName: any): any;
    ready(weaponName: any): any;
    barComponent(): any;
    arm(): Promise<any>;
  };

  export const TINDERBOX: any;

  export const TOLL_COIN_TARGET: any;

  export const TOOL_ACQUIRE_OPTIONS: any;

  export const TOOL_ACQUIRE_SETTING: any;

  export class TaskBot extends LoopingBot {
    constructor();
    add(...tasks: any): any;
    loop(): Promise<any>;
  }

  export class Tile {
    constructor(x: any, z: any, level?: any);
    x: any;
    z: any;
    level: any;
    static from(tile: any): any;
    distanceTo(other: any): any;
    translate(dx: any, dz: any): any;
    equals(other: any): any;
    toString(): any;
  }

  export const Trade: {
    active(): any;
    onOfferScreen(): any;
    onConfirmScreen(): any;
    partner(): any;
    myOffer(): any;
    theirOffer(): any;
    request(playerName: any): any;
    offerAll(name: any, pick: any): any;
    offer(name: any, n: any, pick: any): any;
    removeAll(): any;
    accept(): any;
    decline(): any;
  };

  export const Traversal: {
    walkTo: any;
    walkResilient(tile: any, opts?: any): Promise<any>;
    preload(): any;
    remaining(): any;
    teleportsEnabled(): any;
    requestRepath(_reason: any): any;
    get pureWalk(): any;
    get withTeles(): any;
  };

  export class TreeBot extends LoopingBot {
    root(): any;
  }

  export const VARROCK_ANVIL_BANK: any;

  export const VARROCK_ANVIL_STAND: any;

  export const WALK_DESTINATIONS: any;

  export const WALK_OPTIONS: any;

  export const WHIRLPOOL_IDS: any;

  export const WOODCUTTING_LOCATIONS: any;

  export const WOODCUTTING_LOCATION_OPTIONS: any;

  export function acquireKeepNames(...args: any): any;

  export const apiVersion: number;

  export function axeReq(): any;

  export function axeShopOffers(...args: any): any;

  export function bankDistance(...args: any): any;

  export function bankHasBetterGatherTool(): any;

  export function bankUnlocked(bank: any): any;

  export function bestAffordableShopTier(...args: any): any;

  export function bestAxe(level: any, available: any): any;

  export function bestFromTiers(level: any, tiers: any, available: any): any;

  export function bestHeldToolNames(...args: any): any;

  export function bestOwnedTier(...args: any): any;

  export function bestPickaxe(level: any, available: any): any;

  export function bestSmithableAxe(...args: any): any;

  export function boothFields(...args: any): any;

  export function buyPlansCost(...args: any): any;

  export function canFundPlan(...args: any): any;

  export function canWieldTool(name: any, attack: any): any;

  export function coinsToWithdraw(...args: any): any;

  export function depositAllExcept(keep: any): any;

  export function depositMatcher(own: any, includeCommon: any): any;

  export function entNpcOnTile(npcs: any, tile: any): any;

  export const events: {
    off(...args: any): any;
    on(...args: any): any;
  };

  export function exactTool(name: any): any;

  export function fishingGearShopCart(...args: any): any;

  export function fishingRestockPlan(...args: any): any;

  export function fishingShopCost(...args: any): any;

  export function fishingVendorFor(...args: any): any;

  export function gearKeepNames(...args: any): any;

  export function gearLabel(...args: any): any;

  export function hasAll(...args: any): any;

  export function hasAllTools(reqs: any, skillLevel: any, count: any): any;

  export function hasFishingGear(...args: any): any;

  export function hasToolReq(req: any, skillLevel: any, count: any): any;

  export function held(...args: any): any;

  export function isCowFieldLootTile(...args: any): any;

  export function isEntNpcId(id: any): any;

  export function isFishingBaitPiece(...args: any): any;

  export function locationOptions(...args: any): any;

  export function matchesCommonBankLoot(name: any, id?: any): any;

  export function miningLocationLabel(...args: any): any;

  export function missingFishingGear(...args: any): any;

  export function missingToolLabels(): any;

  export function nearestBank(here: any): any;

  export function nearestCowLocation(tile: any): any;

  export function nearestUsableBank(here: any, usable: any): any;

  export function needsTollCoins(...args: any): any;

  export function parseBankStrategy(label: any): any;

  export function parseToolAcquireMode(...args: any): any;

  export function pickaxeReq(): any;

  export function pickaxeShopOffers(...args: any): any;

  export function planAxeAcquire(...args: any): any;

  export function planBrokenToolRepair(...args: any): any;

  export function planFishingGearAcquire(...args: any): any;

  export function planFishingGearBuys(...args: any): any;

  export function planGatherToolAcquire(...args: any): any;

  export function planPickaxeAcquire(...args: any): any;

  export const reader: {
    worldTile(): any;
    serverTile(): any;
    selfSlot(): any;
    inventorySize(): any;
    inventory(): any;
    bankSideItems(): any;
    sceneState(): any;
    ingame(): any;
    npcs(): any;
    locs(): any;
    players(): any;
    groundItems(): any;
    equipment(): any;
    modals(): any;
    chatContinueComId(): any;
    chatOptions(): any;
    chatModalTexts(): any;
    activeSideTab(): any;
    localPlayerName(): any;
    combatLevel(): any;
    selfChat(): any;
    hintTile(): any;
    npcBox(index: any): any;
    retaliateControls(): any;
    inCombat(): any;
    selfAnim(): any;
    selfTarget(): any;
    selfFaceEntity(): any;
    energy(): any;
    varp(index: any): any;
    stat(i: any): any;
    skillCount(): any;
    sideTabInterface(tab: any): any;
    selectButtonLabelsByVarp(_root: any, _varp: any): any;
    selectButtonByVarp(_root: any, _varp: any, mode: any): any;
    targetButtonByBase(_root: any, label: any): any;
    bankComId(): any;
    ifText(comId: any): any;
    countDialogOpen(): any;
    makeProducts(): any;
    toLocal(x: any, z: any): any;
  };

  export function registerScript(...args: any): any;

  export function resolveBankOpenRoute(...args: any): any;

  export function resolveCowLocation(setting: any): any;

  export function resolveDestination(...args: any): any;

  export function resolveFishMethod(...args: any): any;

  export function resolveFishingLocation(...args: any): any;

  export function resolveGatheringLocation(...args: any): any;

  export function resolveMiningLocation(...args: any): any;

  export function resolveRockIds(_names: any): any;

  export function resolveWoodcuttingLocation(...args: any): any;

  export function sameMapSquare(...args: any): any;

  export function shopableMissingFishingGear(...args: any): any;

  export function shouldBankNow(): any;

  export function shouldBootstrapTollCoins(...args: any): any;

  export function spotMatchesMethod(...args: any): any;

  export function surplusHeldToolNames(...args: any): any;

  export function tinderboxReq(): any;

  export function toolAttackLevel(...args: any): any;

  export function toolKeepNames(reqs: any): any;

  export function toolKitLabel(): any;

  export function toolRestockPlan(reqs: any, skillLevel: any, invCount: any, bankCount: any): any;

  export function toolsNeedingEquip(...args: any): any;

  export function withBaitTarget(...args: any): any;

  export function withdrawOp(ops: any, which: any): any;

}

declare module '*adapter/ClientAdapter.js' {
  export const reader: {
    worldTile(): any;
    serverTile(): any;
    selfSlot(): any;
    inventorySize(): any;
    inventory(): any;
    bankSideItems(): any;
    sceneState(): any;
    ingame(): any;
    npcs(): any;
    locs(): any;
    players(): any;
    groundItems(): any;
    equipment(): any;
    modals(): any;
    chatContinueComId(): any;
    chatOptions(): any;
    chatModalTexts(): any;
    activeSideTab(): any;
    localPlayerName(): any;
    combatLevel(): any;
    selfChat(): any;
    hintTile(): any;
    npcBox(index: any): any;
    retaliateControls(): any;
    inCombat(): any;
    selfAnim(): any;
    selfTarget(): any;
    selfFaceEntity(): any;
    energy(): any;
    varp(index: any): any;
    stat(i: any): any;
    skillCount(): any;
    sideTabInterface(tab: any): any;
    selectButtonLabelsByVarp(_root: any, _varp: any): any;
    selectButtonByVarp(_root: any, _varp: any, mode: any): any;
    targetButtonByBase(_root: any, label: any): any;
    bankComId(): any;
    ifText(comId: any): any;
    countDialogOpen(): any;
    makeProducts(): any;
    toLocal(x: any, z: any): any;
  };
  export const actions: {
    closeModal(): any;
    ifButton(componentId: any): any;
    clickSideTab(tab: any): any;
    setRetaliate(on: any): any;
    setRun(on: any): any;
    walkTo(lx: any, lz: any): any;
    answerCountDialog(value: any): any;
  };
}

declare module '*api/acquisition/Tools.js' {
  export const TINDERBOX: any;
  export const HAMMER: any;
  export const KNIFE: any;
  export const CHISEL: any;
  export const NEEDLE: any;
  export const AXES: any;
  export const PICKAXES: any;
  export function exactTool(name: any): any;
  export function tinderboxReq(): any;
  export function axeReq(): any;
  export function pickaxeReq(): any;
  export function toolKeepNames(reqs: any): any;
  export function hasToolReq(req: any, skillLevel: any, count: any): any;
  export function hasAllTools(reqs: any, skillLevel: any, count: any): any;
  export function bestAxe(level: any, available: any): any;
  export function bestPickaxe(level: any, available: any): any;
  export function bestFromTiers(level: any, tiers: any, available: any): any;
  export function canWieldTool(name: any, attack: any): any;
  export function toolRestockPlan(reqs: any, skillLevel: any, invCount: any, bankCount: any): any;
  export function missingToolLabels(): any;
  export function toolKitLabel(): any;
  export function bankHasBetterGatherTool(): any;
}

declare module '*api/ai/clues/ClueExecutor.js' {
  export class ClueExecutor {
    static setTeleports(enabled: any): any;
  }
}

declare module '*api/ai/clues/SolveClue.js' {
  export class SolveClue {
    constructor(hostArg: any);
    host: any;
    token: any;
    status: any;
    clueStatus(): any;
    noteDeath(): any;
    ownsEquipment(): any;
    retry(): any;
    enabled(): any;
    hooks(): any;
    validate(): any;
    execute(): Promise<any>;
  }
  export function heldClueLikeId(): any;
  export function walkToBank(tile: any, log: any): any;
}

declare module '*api/ai/clues/bankAccess.js' {
  export function openClueBank(_log: any): any;
}

declare module '*api/ai/clues/cluePaint.js' {
  export function paintClueProgress(p: any, idle?: any): any;
}

declare module '*api/ai/clues/data/cluedb.js' {
  export const CLUE_DB: {
  };
  export const CASKET_IDS: {
  };
}

declare module '*api/ai/clues/data/toolAcquire.js' {
  export const SPADE_NAME: any;
  export const TRIO: any;
}

declare module '*api/ai/clues/duelTravel.js' {
  export const DUEL_CLUE_ID: any;
  export const DUEL_CLUE_TILE: any;
  export function crossesClueDuel(dest: any): any;
  export function walkAcrossClueDuel(dest: any, radius: any, log: any): Promise<any>;
}

declare module '*api/ai/quests/defs/murder/areas.js' {
  export const MURDER_NAME: any;
  export const MURDER_OBJ: {
    POT: any;
    POT_FLOUR: any;
    FLYPAPER: any;
    DAGGER: any;
    DAGGER_DUST: any;
    UNKNOWN_PRINT: any;
    KILLERS_PRINT: any;
    THREAD_RED: any;
    THREAD_GREEN: any;
    THREAD_BLUE: any;
  };
  export const MURDER_LOC: {
    WINDOW: any;
    FLOUR_BARREL: any;
    SACKS: any;
  };
  export const MURDER_TILE: {
    BANK: any;
    GUARD: any;
    STUDY: any;
    FLOUR_BARREL: any;
    SACKS: any;
    SALESMAN: any;
    ARHEIN: any;
  };
}

declare module '*api/ai/quests/engine/QuestEngine.js' {
  export const QuestEngine: any;
}

declare module '*api/ai/quests/exec/primitives.js' {
  export function pickPreferred(options: any, prefer: any): any;
  export function pickByLine(): any;
  export function isUnderground(t: any): any;
  export function needsHop(): any;
  export function walkWithHops(dest: any, radius: any, hops: any, log: any): Promise<any>;
  export function gotoNpc(stop: any): Promise<any>;
  export function driveDialog(prefer: any, log: any, gapMs: any): Promise<any>;
  export function openDialogue(npcName: any, log: any): Promise<any>;
  export function talkThrough(npcName: any, prefer: any, log: any, gapMs: any): Promise<any>;
  export function talkStrict(npcName: any, prefer: any, log: any): any;
  export function talkChoosingBy(): Promise<any>;
  export function talkOp(actions: any): any;
}

declare module '*api/bank/Bank.js' {
  export function withdrawOp(ops: any, which: any): any;
  export const Bank: {
    isOpen(): any;
    loaded(): any;
    ready(): any;
    items(): any;
    count(name: any): any;
    deposit(name: any): any;
    depositInventory(): Promise<any>;
    depositAllMatching(predicate: any, log: any): Promise<any>;
    depositAllExcept(keep: any): Promise<any>;
    withdraw(name: any, amount: any): any;
    setNoteMode(on: any): Promise<any>;
    close(): Promise<any>;
    withdrawById(id: any, op?: any): any;
    withdrawX(name: any, count: any): any;
    withdrawXById(id: any, count: any, landsAsId?: any): any;
    openBooth(stand: any, boothName: any, op: any, _log: any): Promise<any>;
    openNearest(boothName: any, op: any, log: any): Promise<any>;
    openNearestWorld(): Promise<any>;
    waitReady(timeoutMs: any, log: any): Promise<any>;
    snapshotReady(): any;
    snapshotGeneration(): any;
    waitSnapshotAfter(generation: any, timeoutMs: any): Promise<any>;
    countById(id: any): any;
    withdrawLoad(name: any): Promise<any>;
    openNearestAccess(access: any, log: any): Promise<any>;
    openNpcAccess(access: any, log: any): Promise<any>;
  };
}

declare module '*api/bank/BankLocations.js' {
  export const USE_MAGE_BANK: any;
  export const USE_ZANARIS_BANK: any;
  export const BANK_LOCATIONS: any;
  export function approachOf(bank: any): any;
  export function bankUnlocked(bank: any): any;
  export function nearestBank(here: any): any;
  export function nearestBanks(here: any): any;
  export function nearestUsableBank(here: any, usable: any): any;
  export function nearestBankReachable(here: any, _navigator: any): Promise<any>;
}

declare module '*api/bank/Banking.js' {
  export const COMMON_BANK_LOOT: any;
  export const RANDOM_EVENT_CASKET_ID: any;
  export function matchesCommonBankLoot(name: any, id?: any): any;
  export function depositMatcher(own: any, includeCommon: any): any;
  export function depositAllExcept(keep: any): any;
  export const PERIODIC_BANK_SETTINGS: {
    bankStrategy: any;
    bankEveryItems: any;
    bankEveryMinutes: any;
    bankCommonJunk: any;
  };
  export function parseBankStrategy(label: any): any;
  export const Banking: {
    open(opts?: any): Promise<any>;
    bankNearest(opts?: any): Promise<any>;
  };
}

declare module '*api/bank/bankOps.js' {
  export function withdrawOp(ops: any, which: any): any;
}

declare module '*api/bank/bankQuestJunk.js' {
  export const QUEST_JUNK: any;
  export function findQuestJunk(): any;
}

declare module '*api/bank/bankRules.js' {
  export function shouldBankNow(): any;
  export function isDisposableGatherJunk(): any;
  export function parseBankStrategy(label: any): any;
  export const PERIODIC_BANK_SETTINGS: {
    bankStrategy: any;
    bankEveryItems: any;
    bankEveryMinutes: any;
    bankCommonJunk: any;
  };
  export const COMMON_BANK_LOOT: any;
  export function matchesCommonBankLoot(name: any, id?: any): any;
  export function depositMatcher(own: any, includeCommon: any): any;
  export function depositAllExcept(keep: any): any;
}

declare module '*api/bank/bankSort.js' {
  export const ARRANGE_SWAP_COM: any;
  export const ARRANGE_INSERT_COM: any;
  export const BANK_INSERT_VARP: any;
  export function sortBank(): Promise<any>;
}

declare module '*api/bank/bankSortRules.js' {
  export const CATEGORY_ORDER: any;
  export function categoryOf(): any;
  export function isUnmatched(): any;
}

declare module '*api/bot/Bot.js' {
  export class LoopingBot {
    loopDelay: any;
    loopCadence: any;
    onStart(): any;
    onStop(): any;
    onPause(): any;
    onResume(): any;
    onPaint(): any;
    loop(): any;
    recoveryAnchor(): any;
    grindTargets(): any;
    ignoredRandoms(): any;
    on(event: any, cb: any): any;
    log(message: any): any;
    get settings(): any;
  }
  export class TaskBot extends LoopingBot {
    constructor();
    add(...tasks: any): any;
    loop(): Promise<any>;
  }
  export class TreeBot extends LoopingBot {
    root(): any;
  }
  export class AbstractBot {
    loopDelay: any;
    loopCadence: any;
    onStart(): any;
    onStop(): any;
    onPause(): any;
    onResume(): any;
    onPaint(): any;
    loop(): any;
    recoveryAnchor(): any;
    grindTargets(): any;
    ignoredRandoms(): any;
    on(event: any, cb: any): any;
    log(message: any): any;
    get settings(): any;
  }
}

declare module '*api/chatbox/gameMessages.js' {
  export const CANT_REACH: any;
  export const WRONG_SIDE: any;
  export const GameMessages: {
    mark(): any;
    since(mark: any): any;
    sawSince(mark: any, pattern: any): any;
  };
}

declare module '*api/combat/CombatStyle.js' {
  export const COMBAT_STYLE_OPTIONS: any;
  export const RANGE_STYLE_OPTIONS: any;
  export function parseCombatStyle(name: any): any;
  export function tryParseCombatStyle(name: any): any;
  export function resolveSplitCombatSettings(rawCombatStyle: any, rawMeleeStyle: any): any;
  export function parseRangeStyle(name: any): any;
  export function describeCombatStyle(resolution: any): any;
}

declare module '*api/combat/CombatStyleLogic.js' {
  export const ATTACKSTYLE_MAGIC_VARP: any;
  export const AUTOCAST_ARMED: any;
  export function runesPerCast(spellName: any, wielded: any): any;
  export function spellButtonCom(spellName: any): any;
  export function castsAvailable(spellName: any, wielded: any, held: any): any;
  export function runeWithdrawList(spellName: any, wielded: any, casts: any): any;
  export const SPELL_DB: {
    "Wind Strike": any;
    "Water Strike": any;
    "Earth Strike": any;
    "Fire Strike": any;
    "Wind Bolt": any;
    "Water Bolt": any;
    "Earth Bolt": any;
    "Fire Bolt": any;
    "Wind Blast": any;
    "Water Blast": any;
    "Earth Blast": any;
    "Fire Blast": any;
    "Wind Wave": any;
    "Water Wave": any;
    "Earth Wave": any;
    "Fire Wave": any;
  };
}

declare module '*api/combat/Special.js' {
  export const SA_MAX_ENERGY: any;
  export const Special: {
    energy(): any;
    armed(): any;
    wielded(): any;
    cost(weaponName: any): any;
    ready(weaponName: any): any;
    barComponent(): any;
    arm(): Promise<any>;
  };
}

declare module '*api/combat/boostPotions.js' {
  export const BOOST_POTIONS: any;
  export const SUPER_ATTACK: any;
  export const SUPER_STRENGTH: any;
  export const EMPTY_VIAL: any;
  export const BOOST_FLOOR: any;
  export function boostFaded(base: any, effective: any, floor: any): any;
  export function plannedPotions(carry: any): any;
  export function potionToSip(s: any): any;
}

declare module '*api/combat/eatTiming.js' {
  export const URGENT_HP_FRACTION: any;
  export function shouldHoldEat(input: any): any;
  export class AttackClock {
    observe(anim: any, tick: any): any;
    attackedThisTick(tick: any): any;
    reset(): any;
  }
}

declare module '*api/combat/equipment.js' {
  export const BOWS: any;
  export const CROSSBOWS: any;
  export const DARTS: any;
  export const ARROWS: any;
  export const BOLTS: any;
  export const MELEE_WEAPONS: any;
  export const STAFFS: any;
}

declare module '*api/combat/fightUpkeep.js' {
  export function swingStartedThisTick(): any;
  export function buryOneInFight(boneName: any): Promise<any>;
}

declare module '*api/combat/food.js' {
  export const FOOD_OPTIONS: any;
  export const MIN_EAT_HP: any;
  export function foodHealAmount(foodName: any): any;
  export function foodForms(foodName: any): any;
  export function isFoodItem(name: any, foodName: any): any;
  export function foodCount(items: any, foodName: any): any;
  export function eatAtHpThreshold(maxHp: any, heal: any, minHp: any): any;
  export function shouldEatToUseFood(opts: any): any;
  export function shouldEatFood(foodName: any, opts: any): any;
}

declare module '*api/combat/hunting/combat.js' {
  class HuntTask {
    constructor(family: any, host: any, site: any);
    family: any;
    host: any;
    site: any;
    hooks: any;
    token: any;
    validate(): any;
    run(): Promise<any>;
  }
  export function siteArgs(site: any, extra: any): any;
  export function hooksOf(host: any, site: any, leave: any): any;
  export function anchorFor(site: any, style: any, index: any): any;
  export class Fight extends HuntTask {
    constructor(host: any, site: any);
    execute(): Promise<any>;
    reset(): any;
    interruptWatch(): any;
    blocksLoot(): any;
  }
  export class Retreat extends HuntTask {
    constructor(host: any, site: any);
    execute(): Promise<any>;
  }
  export class HoldSafespot extends HuntTask {
    constructor(host: any, site: any);
    execute(): Promise<any>;
  }
  export class WalkToSpot extends HuntTask {
    constructor(host: any, site: any);
    execute(): Promise<any>;
  }
  export class EnterLair extends HuntTask {
    constructor(host: any, site: any);
    execute(): Promise<any>;
  }
  export function cell(host: any, site: any): any;
  export function leaveLair(host: any, site: any): any;
  export function bankRoutine(host: any, site: any, opts: any): any;
}

declare module '*api/combat/hunting/guarded.js' {
  export const LOOT_GUARD: any;
  export function guarded(drop: any, bodies: any, radius?: any): any;
}

declare module '*api/combat/hunting/logic.js' {
  export const SAFESPOT_BLIND_MS: any;
  export const PROTECT_FROM_MELEE: any;
  export const PRAYER_SIP_FLOOR: any;
  export const PRAYER_SIP_FRACTION: any;
  export const LOOT_REACH: any;
  export const LOOT_REACH_OPEN: any;
  export const SHIELD_ABSORBS: any;
  export const POTION_PROTECTS: any;
  export const ANTIFIRE_TICKS: any;
  export const ANTIFIRE_MARGIN_TICKS: any;
  export function nextSafespot(s: any): any;
  export function hurtOnSpot(s: any): any;
  export function retreatDue(s: any): any;
  export function lootHalts(s: any): any;
  export function holdDue(s: any): any;
  export function nearestSpot(from: any, spots: any): any;
  export function bodyOrigin(tile: any, size: any): any;
  export function noteSighting(prev: any, tile: any, now: any): any;
  export function settled(s: any, now: any, ms: any): any;
  export function retreatAim(a: any): any;
  export function chaseMode(style: any, fireAtRange: any): any;
  export function prayerFor(style: any, fireAtRange: any): any;
  export function prayerSipDue(points: any, max: any): any;
  export function lootReach(fireAtRange: any): any;
  export function attackRangeFor(style: any): any;
  export function engageRangeFor(style: any): any;
  export function gapTo(from: any, tile: any, size: any): any;
  export function shieldGate(style: any, fireAtRange: any, hasShield: any): any;
  export function styleGate(style: any, fireAtRange: any): any;
  export function antifireDue(s: any): any;
  export function antifireLapsed(sawShield: any, sawPotion: any): any;
  export function nextApproachIndex(stops: any, here: any): any;
  export function isClueObj(id: any): any;
  export function keyStatus(held: any, banked: any): any;
  export function wantsDrop(item: any, f: any): any;
  export function siteTileOf(schema: any, bag: any, key: any, site: any): any;
  export function keepDoses(potionDoses: any, antipoisonDoses: any, carriesAntipoison: any): any;
}

declare module '*api/combat/hunting/sites.js' {
  export function inBox(b: any): any;
  export const DRAGON_SITES: any;
  export const TAVERLEY_BLUE: any;
  export const TAVERLEY_BLACK: any;
  export const HEROES_BLUE: any;
  export const GUTANOTH_BLUE: any;
  export const BRIMHAVEN_IRON: any;
  export const BRIMHAVEN_STEEL: any;
  export const STAND_SITE_KEYS: any;
  export const MAX_STANDS: any;
  export const SITE_OPTIONS: any;
  export function needsShield(s: any, style: any): any;
  export function huntNames(s: any): any;
  export function standFor(s: any, n: any): any;
  export function siteFor(key: any): any;
}

declare module '*api/combat/hunting/supply.js' {
  export const SHIELD: any;
  export const POISONED: any;
  export const COINS: any;
  export const ANTIPOISON_LABEL: any;
  export const ANTIPOISON_DOSES: any;
  export const PRAYER_LABEL: any;
  export const PRAYER_DOSES: any;
  export const ANTIFIRE_LABEL: any;
  export const ANTIFIRE_DOSES: any;
  export function antipoisonPlan(want: any): any;
  export function prayerPlan(want: any): any;
  export function antifirePlan(want: any): any;
  export function doseToDrink(count: any, doses?: any): any;
  export function escapeRunesFor(teleportId: any): any;
  export function inCell(): any;
  export function enterLair(h: any, site: any): Promise<any>;
  export function feePrepaid(site: any): any;
  export function acquireKey(h: any, site: any): Promise<any>;
  export function leaveCell(h: any): Promise<any>;
  export function teleportOut(h: any, site: any): Promise<any>;
  export function waitFed(cond: any, ms: any): Promise<any>;
  export function bankRoutine(h: any, site: any, opts: any): Promise<any>;
  export function walkApproach(): any;
  export function withdrawTo(): any;
  export function leaveLair(host: any, site: any): any;
}

declare module '*api/combat/keepList.js' {
  export function combatKeepNames(o: any): any;
}

declare module '*api/combat/meleeWeapons.js' {
  export function bestMeleeWeapon(available: any, pick: any): any;
  export function knownMeleeWeapon(names: any): any;
}

declare module '*api/combat/ranged.js' {
  export const RANGED_WEAPONS: any;
  export const ROCK_CRAB_RANGED_WEAPONS: any;
  export function rangeLoadoutOf(weapon: any, ammo: any): any;
  export function rockCrabRangeLoadout(...args: any): any;
  export function rangeSupplyEmpty(equipped: any, carried: any, ground: any): any;
}

declare module '*api/combat/rangedSettings.js' {
  export const CUSTOM_RANGED_SETTINGS: {
    customBow: any;
    customAmmo: any;
  };
  export function rangedItem(settings: any, key: any, fallback: any): any;
}

declare module '*api/cooking/CookLocations.js' {
  export const COOK_LOCATION_OPTIONS: any;
  export function cookLocation(name: any): any;
  export function resolveCookLocation(setting: any, from: any, unlocked: any): any;
  export const CUSTOM_LOCATION: any;
  export const COOK_LOCATIONS: any;
}

declare module '*api/duel/ClueDuel.js' {
  export const CLUE_DUEL_LOBBY: any;
  export const CLUE_DUEL_OPTIONS: any;
  export function clueDuelName(name: any): any;
  export function leaveClueDuel(log: any): Promise<any>;
  export class ClueDuelHandshake {
    constructor(partner: any, initiator: any, log: any);
    partner: any;
    initiator: any;
    log: any;
    tick(): Promise<any>;
  }
  export class ClueDuelHelper {
    constructor(partner: any, log: any);
    partner: any;
    log: any;
    validate(): any;
    execute(): Promise<any>;
  }
}

declare module '*api/duel/Duel.js' {
  export const DUEL_SELECT_MODAL: any;
  export const DUEL_CONFIRM_MODAL: any;
  export const DUEL_WIN_MODAL: any;
  export const DUEL_FIGHT_ARENAS: any;
  export function parseDuelPartnerHeader(header: any): any;
  export function fightArenaAt(tile: any): any;
  export const Duel: {
    offerOpen(): any;
    confirmOpen(): any;
    winOpen(): any;
    active(): any;
    partner(): any;
    waitingForOther(): any;
    challenge(player: any): any;
    fight(player: any): any;
    accept(): any;
    cancel(): any;
    closeWin(): any;
  };
}

declare module '*api/equipment/Equipment.js' {
  export const Equipment: {
    items(): any;
    contains(name: any): any;
    equip(name: any): Promise<any>;
    unequip(name: any): Promise<any>;
  };
}

declare module '*api/execution/EventSignal.js' {
  export const EventSignal: {
    pending(): any;
    setInterrupt(callback: any): any;
    ignoredRandoms(): any;
  };
}

declare module '*api/execution/Execution.js' {
  export const Execution: {
    delay(ms: any): Promise<any>;
    delayTicks(n: any): Promise<any>;
    delayUntil(cond: any, timeoutMs?: any): any;
    delayUntilTicks(cond: any, maxTicks: any): any;
    noteProgress(): any;
  };
  export function parkMachine(handle: any): any;
}

declare module '*api/firemaking/Firemaking.js' {
  export const TINDERBOX: any;
  export const CANT_LIGHT: any;
  export const FIRE_START_TICKS: any;
  export const FIRE_LIGHT_TICKS: any;
  export const BURN_WEST: {
    dx: any;
    dz: any;
  };
  export const BURN_DIRS: any;
  export const FIRE_SPOTS: any;
  export const FIRE_SPOT_OPTIONS: any;
  export function localFirePlot(origin: any, half: any): any;
  export const LOG_LEVELS: any;
  export function tileKey(t: any): any;
  export class NoLightTiles {
    add(tile: any): any;
    has(tile: any): any;
    get size(): any;
    merge(occupied: any): any;
    clear(): any;
  }
  export function inFirePlot(t: any, plot: any): any;
  export function burnLaneWant(logCount: any): any;
  export function isBurnWest(dir: any): any;
  export function fireReactionTicks(): any;
  export function runInDir(from: any, plot: any, dir: any, occupied: any, walkable: any, canStep: any, cap: any): any;
  export function findBurnLane(plot: any, here: any, occupied: any, want?: any, _walkable?: any, _canStep?: any, directions?: any): any;
  export function lightFire(logName: any): Promise<any>;
}

declare module '*api/firemaking/LightFire.js' {
  export function lightFire(logName: any): Promise<any>;
}

declare module '*api/game/Game.js' {
  export const Game: {
    ingame(): any;
    tile(): any;
    tick(): any;
    inCombat(): any;
    animating(): any;
    runEnabled(): any;
    autoRetaliate(): any;
    autoRetaliateOn(): any;
    myName(): any;
    combatMode(): any;
    combatStyles(): any;
    hasCombatStyle(style: any): any;
    combatStyleResolution(style: any): any;
    setCombatMode(mode: any): any;
    setCombatStyle(style: any): any;
    setAutoRetaliate(on: any): any;
    openSideTab(tab: any): Promise<any>;
    castOnItem(spell: any, item: any): Promise<any>;
    castOnLoc(spell: any, loc: any): Promise<any>;
    teleport(name: any): Promise<any>;
    energy(): any;
    weight(): any;
    cameraYaw(): any;
    cameraPitch(): any;
    setCameraYaw(yaw: any): any;
    combatStyleMode(): any;
    sceneReady(): any;
    sceneState(): any;
    attackedByPlayer(): any;
    castOnNpc(): Promise<any>;
  };
}

declare module '*api/grounditems/GroundItems.js' {
  export class GroundItem {
    constructor(row: any);
    snap: any;
    get name(): any;
    get id(): any;
    get count(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    interact(action: any): any;
  }
  export const GroundItems: {
    query(): any;
  };
}

declare module '*api/inventory/Inventory.js' {
  export const Inventory: {
    count(name: any): any;
    countById(id: any): any;
    first(name: any): any;
    items(): any;
    contains(name: any): any;
    used(): any;
    isFull(): any;
    free(): any;
  };
}

declare module '*api/inventory/packRules.js' {
  export function matchesAny(name: any, patterns: any): any;
  export function countMatching(items: any, patterns: any): any;
  export function slotsMatching(items: any, patterns: any): any;
  export function shouldBank(lootSlots: any, bankAt: any, invFull: any): any;
  export function shouldRestock(foodCount: any, threshold: any): any;
  export function shouldEat(hp: any, maxHp: any, heal: any, foodCount: any): any;
  export function shouldPanic(hpFrac: any, gate: any, foodCount: any): any;
}

declare module '*api/loadout/loadoutPlan.js' {
  export function foodOf(loadout: any, fallback: any): any;
  export function gearOf(loadout: any): any;
  export function suppliesOf(loadout: any): any;
  export function weaponOf(loadout: any, fallback?: any): any;
  export function scriptFood(bag: any, fallback: any): any;
  export function scriptFoods(bag: any, fallback: any): any;
}

declare module '*api/loadout/loadoutSetting.js' {
  export const LOADOUT_SETTING: {
    type: any;
    default: any;
    options: any;
    optionsFrom: any;
    label: any;
    help: any;
  };
  export function selectedLoadout(bag: any): any;
}

declare module '*api/locs/Locs.js' {
  export class Loc {
    constructor(row: any);
    snap: any;
    get name(): any;
    get id(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    interact(action: any): any;
  }
  export const Locs: {
    query(): any;
  };
}

declare module '*api/magic/Autocast.js' {
  export const Autocast: {
    armed(): any;
    staffTabAttached(): any;
    arm(spellName: any, log: any): Promise<any>;
  };
}

declare module '*api/market/MarketMaker.js' {
  export const MarketMaker: any;
}

declare module '*api/market/catalog.js' {
  export function liveCatalog(): any;
  export function tradeable(id: any): any;
  export function clientName(_cat: any, id: any): any;
  export function displayName(_cat: any, id: any): any;
  export function notedId(_cat: any, id: any): any;
  export function unnotedId(_cat: any, id: any): any;
}

declare module '*api/model/Loc.js' {
  export class Loc {
    constructor(row: any);
    snap: any;
    get name(): any;
    get id(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    interact(action: any): any;
  }
}

declare module '*api/model/Npc.js' {
  export class Npc {
    constructor(row: any);
    snap: any;
    get name(): any;
    get id(): any;
    get index(): any;
    get inCombat(): any;
    get health(): any;
    get level(): any;
    get size(): any;
    networkOrigin(): any;
    networkTile(): any;
    targetsMe(): any;
    targetsAnotherPlayer(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    valid(): any;
    interact(action: any): any;
  }
}

declare module '*api/model/Player.js' {
  export class Player {
    constructor(row: any);
    snap: any;
    get name(): any;
    get index(): any;
    get inCombat(): any;
    get combatLevel(): any;
    targetsMe(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    interact(action: any): any;
  }
}

declare module '*api/npcs/Npcs.js' {
  export class Npc {
    constructor(row: any);
    snap: any;
    get name(): any;
    get id(): any;
    get index(): any;
    get inCombat(): any;
    get health(): any;
    get level(): any;
    get size(): any;
    networkOrigin(): any;
    networkTile(): any;
    targetsMe(): any;
    targetsAnotherPlayer(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    valid(): any;
    interact(action: any): any;
  }
  export const Npcs: {
    query(): any;
    all(): any;
    nearest(count?: any): any;
  };
  export function talkOp(actions: any): any;
}

declare module '*api/players/Players.js' {
  export class Player {
    constructor(row: any);
    snap: any;
    get name(): any;
    get index(): any;
    get inCombat(): any;
    get combatLevel(): any;
    targetsMe(): any;
    tile(): any;
    distance(): any;
    actions(): any;
    interact(action: any): any;
  }
  export const Players: {
    query(): any;
    all(): any;
  };
}

declare module '*api/prayer/Prayer.js' {
  export const PROTECT_FROM_MAGIC: any;
  export const Prayer: {
    points(): any;
    max(): any;
    full(): any;
    known(name: any): any;
    available(name: any): any;
    active(name: any): any;
    set(name: any, on: any): Promise<any>;
    clear(): Promise<any>;
  };
}

declare module '*api/query/Query.js' {
  export function matchesEntityName(actual: any, configured: any): any;
  export class EntityQuery {
    constructor(supplySnaps: any, wrap: any);
    supplySnaps: any;
    wrap: any;
    snapFilters: any;
    entityFilters: any;
    static fromSnapshots(supply: any, wrap: any): any;
    name(...names: any): any;
    action(action: any): any;
    within(dist: any): any;
    withinOf(origin: any, dist: any): any;
    where(pred: any): any;
    forEachMatch(seen: any): any;
    results(): any;
    nearest(): any;
    first(): any;
    exists(): any;
    count(): any;
    inside(_area: any): any;
    nearestPreferLocal(_preferRadius: any): any;
  }
  export default EntityQuery;
}

declare module '*api/shop/BuyoutLogic.js' {
  export function buyoutPlan(rec: any, stock: any, coins: any, chosen: any): any;
}

declare module '*api/shop/Shop.js' {
  export const Shop: {
    isOpen(): any;
    stock(): any;
    player(): any;
    open(npcName: any): any;
    buy(name: any, qty: any): any;
    sell(name: any, qty: any, pick: any): any;
    sellAll(name: any, pick: any): any;
    close(): any;
    buyById(): any;
  };
}

declare module '*api/shop/types.js' {
  export {};
}

declare module '*api/skills/Skills.js' {
  export const Skills: {
    index(name: any): any;
    xp(name: any): any;
    level(name: any): any;
    effective(name: any): any;
    hpFraction(): any;
  };
}

declare module '*api/sustain/Sustain.js' {
  export const Sustain: {
    hook: any;
    running: any;
    set(hook: any): any;
    run(): Promise<any>;
  };
}

declare module '*api/tasks/Anchor.js' {
  export const HOME_ARRIVE_RADIUS: any;
  export function shouldWalkHomeToGatherAnchor(_distToAnchor: any, _arriveRadius: any): any;
  export function shouldSoftHomeFromGatherMiss(_distToAnchor: any, _leash: any): any;
  export function beyondLeash(bot: any, here?: any, slack?: any): any;
  export function tileWithinLeash(bot: any, tile: any, slack?: any): any;
  export function resolveRunAnchor(here: any, locationSpot: any): any;
  export function createReturnToAnchorTask(bot: any, opts?: any): any;
}

declare module '*api/tasks/ContinueDialog.js' {
  export class ContinueDialog {
    constructor(onContinue: any);
    onContinue: any;
    validate(): any;
    execute(): Promise<any>;
  }
}

declare module '*api/tasks/DeathRecovery.js' {
  export class DeathRecovery {
    constructor(_bot: any, opts: any);
    opts: any;
    validate(): any;
    execute(): Promise<any>;
  }
}

declare module '*api/tasks/PeriodicBank.js' {
  export class PeriodicBank {
    constructor(opts: any);
    opts: any;
    validate(): any;
    execute(): Promise<any>;
  }
}

declare module '*api/thieving/CakeStall.js' {
  export function carriedCakes(): any;
  export function needsCakeRestock(target: any): any;
  export function stealCakes(opts?: any): Promise<any>;
}

declare module '*api/thieving/cakeStallData.js' {
  export const STALL_TILE: any;
  export const STAND: any;
  export const STAND_ALT: any;
  export const FLEE_TILE: any;
  export const STALL_NAME: any;
  export const STALL_OP: any;
  export const CAKE_ITEMS: any;
  export const LOCKOUT_TICKS: any;
  export const RESET_AFTER_REFUSALS: any;
  export function classifySteal(): any;
  export function shouldReset(consecutiveRefusals: any): any;
}

declare module '*api/thieving/stealRules.js' {
  export const THIEVER_BANKING_OPTIONS: any;
  export const STUN_COMBAT_TICKS: any;
  export function nextWithdrawChunk(need: any): any;
  export function withdrawTo(name: any, target: any, count: any): Promise<any>;
  export function closeBankAndConfirmCount(expected: any, count: any): Promise<any>;
  export function autoFoodBanking(mode: any): any;
  export function foodMatches(name: any, keyword: any): any;
  export function countFood(items: any, keyword: any): any;
  export function shouldRestockFood(enabled: any, foodCount: any, restockAt: any, bankablePackFull: any): any;
  export function safeToSteal(hpFraction: any, eatAt: any, foodCount: any): any;
  export function canStealNow(foodCount: any, hp: any, minEatHp: any, suicide: any): any;
}

declare module '*api/thieving/targets.js' {
  export function targetSpot(target: any): any;
  export function requiredThieving(target: any): any;
  export const HOSTILE_NAMES: any;
  export function isHostileAttacker(c: any, maxDistance: any): any;
  export function chooseTarget(candidatesNearestFirst: any, reachable: any): any;
  export const PICKPOCKET_TARGET_NAMES: any;
  export const ARDOUGNE_PICKPOCKET_TARGETS: any;
}

declare module '*api/trade/PartnerTrade.js' {
  export const DEFAULT_TRADE_RANGE: any;
  export const MULE_MODE_OPTIONS: any;
  export function parsePartnerList(raw: any): any;
  export function namesMatch(a: any, b: any): any;
  export function isConfiguredPartner(name: any, partners: any): any;
  export function countOfferByName(items: any, itemName: any): any;
  export function decideReceiverOfferScreen(opts: any): any;
  export function decideGiverOfferScreen(myOfferSlots: any): any;
  export function parseMuleMode(raw: any): any;
  export function muleGathererHandoffActive(mode: any, partners: any, powerMode: any): any;
  export function muleReceiverActive(mode: any, partners: any): any;
  export function muleCookerActive(mode: any, partners: any): any;
  export function muleSupplierActive(mode: any, partners: any, powerMode: any): any;
  export function muleNonGathererActive(mode: any, partners: any): any;
  export function countOfferMatching(): any;
}

declare module '*api/trade/Trade.js' {
  export const Trade: {
    active(): any;
    onOfferScreen(): any;
    onConfirmScreen(): any;
    partner(): any;
    myOffer(): any;
    theirOffer(): any;
    request(playerName: any): any;
    offerAll(name: any, pick: any): any;
    offer(name: any, n: any, pick: any): any;
    removeAll(): any;
    accept(): any;
    decline(): any;
  };
}

declare module '*api/trade/drivePartnerTrade.js' {
  export function driveActivePartnerTrade(opts: any): Promise<any>;
}

declare module '*api/ui/dialogue/ChatDialog.js' {
  export const ChatDialog: {
    isOpen(): any;
    canContinue(): any;
    options(): any;
    texts(): any;
    isMakeMenu(): any;
    makeProducts(): any;
    make(match: any): Promise<any>;
    makeX(match: any, count: any): Promise<any>;
    continue(): Promise<any>;
    chooseOption(match: any): Promise<any>;
    isMainMakePanel(): any;
    mainMakeProducts(): any;
    makeFromPanel(match: any, op: any): Promise<any>;
    makeFromPanelMax(match: any): Promise<any>;
    makeOne(): Promise<any>;
  };
}

declare module '*api/ui/questlog/Quests.js' {
  export const Quests: {
    all(): any;
    status(name: any): any;
    journal(): any;
    points(): any;
  };
}

declare module '*api/ui/widgets/Modals.js' {
  export const Modals: {
    main(): any;
    isOpen(): any;
    close(): any;
    closeIfOpen(): any;
  };
}

declare module '*api/walking/DirectNavigator.js' {
  export const DirectNavigator: {
    walk(dest: any): any;
    walkTo(dest: any, radius?: any, timeoutMs?: any): Promise<any>;
  };
}

declare module '*api/walking/Reach.js' {
  export const Reach: {
    entityOp(opts: any): Promise<any>;
    npcDialog(opts: any): Promise<any>;
  };
}

declare module '*api/walking/Traversal.js' {
  export const Traversal: {
    walkTo: any;
    walkResilient(tile: any, opts?: any): Promise<any>;
    preload(): any;
    remaining(): any;
    teleportsEnabled(): any;
    requestRepath(_reason: any): any;
    get pureWalk(): any;
    get withTeles(): any;
  };
}

declare module '*data/cookLocations.js' {
  export const CUSTOM_LOCATION: any;
  export const MAX_SURFACE_CHEB: any;
  export const COOK_LOCATIONS: any;
  export function findCookLocation(locs: any, name: any): any;
  export function buildCookLocations(): any;
}

declare module '*data/cowKillerLocations.js' {
  export const COW_LOCATIONS: any;
  export const COW_LOCATION_OPTIONS: any;
  export const AL_KHARID_BANK: any;
  export function resolveCowLocation(setting: any): any;
  export function nearestCowLocation(tile: any): any;
}

declare module '*data/dropdb.js' {
  export const DROP_DB: any;
}

declare module '*data/herbs.js' {
  export const HERBS: any;
  export const HERB_OPTIONS: any;
}

declare module '*data/itemdb.js' {
  export const ITEM_DB: any;
}

declare module '*data/miningRocks.js' {
  export const ROCK_OPTIONS: any;
  export const ROCK_TYPES: any;
  export const QUEST_ROCK_TYPES: any;
  export const GAS_ROCK_IDS: any;
  export const GAS_ROCK_TICKS: any;
  export const BROKEN_PICKAXE: any;
  export function resolveRockIds(_names: any): any;
}

declare module '*data/pickpocketTargets.js' {
  export const PICKPOCKET_TARGETS: any;
  export const PICKPOCKET_TARGET_NAMES: any;
  export const ARDOUGNE_PICKPOCKET_TARGETS: any;
}

declare module '*data/runeCraftLocations.js' {
  export const RUNES: any;
  export const RUNE_OPTIONS: any;
  export const DEFAULT_RUNE: any;
}

declare module '*data/shopdb.js' {
  export const SHOP_DB: any;
}

declare module '*data/spelldb.js' {
  export const SPELL_DB: {
    "Wind Strike": any;
    "Water Strike": any;
    "Earth Strike": any;
    "Fire Strike": any;
    "Wind Bolt": any;
    "Water Bolt": any;
    "Earth Bolt": any;
    "Fire Bolt": any;
    "Wind Blast": any;
    "Water Blast": any;
    "Earth Blast": any;
    "Fire Blast": any;
    "Wind Wave": any;
    "Water Wave": any;
    "Earth Wave": any;
    "Fire Wave": any;
  };
  export const STAFF_RUNES: any;
}

declare module '*data/woodcuttingLocations.js' {
  export const ENT_NPC_IDS: any;
  export const ENT_LIFE_TICKS: any;
  export function isEntNpcId(id: any): any;
  export function entNpcOnTile(npcs: any, tile: any): any;
}

declare module '*event/webwalk/DirectNavigator.js' {
  export const DirectNavigator: {
    walk(dest: any): any;
    walkTo(dest: any, radius?: any, timeoutMs?: any): Promise<any>;
  };
}

declare module '*event/webwalk/Navigator.js' {
  export const Navigator: {
    start(): any;
    isReady(): any;
    findPath(from: any, to: any, opts?: any): Promise<any>;
  };
  export default Navigator;
}

declare module '*event/webwalk/geometry/Reachability.js' {
  export const Reachability: {
    walkable(target: any): any;
    canReach(target: any, opts?: any): any;
    lineOfSight(from: any, to: any, size: any): any;
    canStep(from: any, to: any): any;
  };
}

declare module '*event/webwalk/walkOpening.js' {
  export function openOp(actions: any): any;
  export function towardDest(_from: any, _here: any, _toward: any): any;
  export function isOpenableObstacle(_name: any, _actions: any, _obstacles: any): any;
  export function walkOpening(dest: any, radius: any, obstacles: any, log: any): Promise<any>;
}

declare module '*geometry/Tile.js' {
  export class Tile {
    constructor(x: any, z: any, level?: any);
    x: any;
    z: any;
    level: any;
    static from(tile: any): any;
    distanceTo(other: any): any;
    translate(dx: any, dz: any): any;
    equals(other: any): any;
    toString(): any;
  }
  export default Tile;
  export function tileFromPosted(value: any): any;
}

declare module '*input/Input.js' {
  export const Input: {
    walk(lx: any, lz: any): any;
    interactNpc(index: any, op: any): any;
    interactPlayer(index: any, op: any): any;
    interactLoc(lx: any, lz: any, _typecode: any, op: any): any;
    takeObj(lx: any, lz: any, _objId: any, op: any): any;
    heldOp(objId: any, _slot: any, _comId: any, op: any): any;
    invButton(objId: any, slot: any, comId: any, op: any): any;
  };
}

declare module '*paint/Paint.js' {
  export const Paint: {
    begin(ctx: any, opts: any): any;
  };
}

declare module '*paint/jive.js' {
  export const JIVE_ACCENT: any;
  export const JIVE_BYLINE: any;
  export const COMBAT_SKILLS: any;
  export function scriptFrame(ctx: any, opts: any): any;
  export function jiveFrame(ctx: any, opts: any): any;
  export class XpTracker {
    constructor(skills: any, read: any);
    skills: any;
    read: any;
    id: any;
    begin(): any;
    progress(): any;
    gains(): any;
  }
  export function paintLevels(p: any, gains: any, mins: any, reserve: any, empty: any): any;
}

declare module '*paint/levelProgress.js' {
  export function xpAtLevel(level: any): any;
  export function levelProgress(level: any, xp: any): any;
  export function etaHours(remaining: any, xpPerHour: any): any;
  export function levelRow(g: any, mins: any): any;
}

declare module '*paint/paintLogic.js' {
  export function fmtDuration(minutes: any): any;
  export function fmtXpHr(gained: any, mins: any): any;
  export function paintSkillShort(skill: any): any;
}

declare module '*runtime/BotHost.js' {
  export const BotHost: {
    get tickCount(): any;
  };
}

declare module '*runtime/RecoveryHints.js' {
  export const RecoveryHints: {
    get pendingRecovery(): any;
    set pendingRecovery(value: any);
    get anchor(): any;
    set anchor(value: any);
    takeAnchor(): any;
    clear(): any;
  };
}

declare module '*runtime/RunManager.js' {
  export const RunManager: {
    override(policy: any): any;
  };
}

declare module '*runtime/ScriptRunner.js' {
  export const ScriptRunner: {
    stop(reason: any): any;
    paintControls(p: any): any;
  };
}

declare module '*runtime/Settings.js' {
  export class SettingsBag {
    constructor(values?: any);
    values: any;
    bool(key: any, fallback?: any): any;
    num(key: any, fallback?: any): any;
    str(key: any, fallback?: any): any;
    list(key: any, fallback?: any): any;
    tile(key: any, fallback?: any): any;
  }
  export const SettingsStore: {
    resolve(_name: any, schema: any): any;
    displayString(_name: any, key: any, def: any): any;
    saved(_name: any, key: any): any;
    globalBag(): any;
  };
}

declare module '*runtime/Supervisor.js' {
  export const Supervisor: {
    noteProgress(): any;
  };
}

declare module '*shim/_kernel.js' {
  export function host(): any;
  export function snap(): any;
  export function notImpl(name: any, reason: any): any;
  export function queue(req: any): any;
  export function runMachine(family: any, args: any, hooks: any): Promise<any>;
  export function machineNow(family: any, args: any): any;
  export function proxy(ns: any, members: any): any;
  export function notImplValue(ns: any): any;
  export function distanceTo(a: any, b: any): any;
  export function planarDistanceTo(a: any, b: any): any;
  export function arrived(dest: any, radius: any): any;
  export function presentOps(actions: any): any;
  export function opIndex(actions: any, action: any): any;
  export function entitySnapView(row: any): any;
  export function optionalText(value: any): any;
}

