// Authored TypeScript declarations for the JS API v1 compat surface.
// Names, members, arity and Promise-returning wrappers are gated against
// the live shim (crates/script/src/compat_dts.rs). Types are written from
// the shim implementation and the host natives. Do not regenerate.
// O-SCRIPT-API: add a typed module .d.ts under compat-js/ and export it from
// the @rs2b0t/api barrel below.
//
// Public barrel exports only typed re-exports (plus unknown stubs for catalog
// ABI names with no shim). Check this file with skipLibCheck: false:
//   npx -p typescript@5.8.3 --yes tsc --noEmit -p crates/script/tests/compat_dts_probe
// that command must report zero diagnostics in this file.

declare module '@rs2b0t/api' {
  export function defineBot(manifest: { name: string; create: () => unknown; [key: string]: unknown }): unknown;

  export { AbstractBot } from '*api/bot/Bot.js';
  export { AL_KHARID_BANK } from '*data/cowKillerLocations.js';
  export { ARDOUGNE_PICKPOCKET_TARGETS } from '*api/thieving/targets.js';
  export { axeReq } from '*api/acquisition/Tools.js';
  export { AXES } from '*api/acquisition/Tools.js';
  export { Bank } from '*api/bank/Bank.js';
  export { BANK_LOCATIONS } from '*api/bank/BankLocations.js';
  export { bankHasBetterGatherTool } from '*api/acquisition/Tools.js';
  export { Banking } from '*api/bank/Banking.js';
  export { bankUnlocked } from '*api/bank/BankLocations.js';
  export { bestAxe } from '*api/acquisition/Tools.js';
  export { bestFromTiers } from '*api/acquisition/Tools.js';
  export { bestPickaxe } from '*api/acquisition/Tools.js';
  export { BROKEN_PICKAXE } from '*data/miningRocks.js';
  export { canWieldTool } from '*api/acquisition/Tools.js';
  export { ChatDialog } from '*api/ui/dialogue/ChatDialog.js';
  export { CHISEL } from '*api/acquisition/Tools.js';
  export { COINS } from '*api/combat/hunting/supply.js';
  export { COMMON_BANK_LOOT } from '*api/bank/Banking.js';
  export { COW_LOCATION_OPTIONS } from '*data/cowKillerLocations.js';
  export { COW_LOCATIONS } from '*data/cowKillerLocations.js';
  export { DEFAULT_RUNE } from '*data/runeCraftLocations.js';
  export { depositAllExcept } from '*api/bank/Banking.js';
  export { depositMatcher } from '*api/bank/Banking.js';
  export { DirectNavigator } from '*api/walking/DirectNavigator.js';
  export { ENT_LIFE_TICKS } from '*data/woodcuttingLocations.js';
  export { ENT_NPC_IDS } from '*data/woodcuttingLocations.js';
  export { EntityQuery } from '*api/query/Query.js';
  export { entNpcOnTile } from '*data/woodcuttingLocations.js';
  export { Equipment } from '*api/equipment/Equipment.js';
  export { exactTool } from '*api/acquisition/Tools.js';
  export { Execution } from '*api/execution/Execution.js';
  export { Game } from '*api/game/Game.js';
  export { GAS_ROCK_IDS } from '*data/miningRocks.js';
  export { GAS_ROCK_TICKS } from '*data/miningRocks.js';
  export { GroundItem } from '*api/grounditems/GroundItems.js';
  export { GroundItems } from '*api/grounditems/GroundItems.js';
  export { HAMMER } from '*api/acquisition/Tools.js';
  export { hasAllTools } from '*api/acquisition/Tools.js';
  export { hasToolReq } from '*api/acquisition/Tools.js';
  export { Inventory } from '*api/inventory/Inventory.js';
  export { isEntNpcId } from '*data/woodcuttingLocations.js';
  export { KNIFE } from '*api/acquisition/Tools.js';
  export { Loc } from '*api/locs/Locs.js';
  export { Locs } from '*api/locs/Locs.js';
  export { LoopingBot } from '*api/bot/Bot.js';
  export { matchesCommonBankLoot } from '*api/bank/Banking.js';
  export { missingToolLabels } from '*api/acquisition/Tools.js';
  export { nearestBank } from '*api/bank/BankLocations.js';
  export { nearestCowLocation } from '*data/cowKillerLocations.js';
  export { nearestUsableBank } from '*api/bank/BankLocations.js';
  export { NEEDLE } from '*api/acquisition/Tools.js';
  export { Npc } from '*api/model/Npc.js';
  export { Npcs } from '*api/npcs/Npcs.js';
  export { parseBankStrategy } from '*api/bank/Banking.js';
  export { PERIODIC_BANK_SETTINGS } from '*api/bank/Banking.js';
  export { pickaxeReq } from '*api/acquisition/Tools.js';
  export { PICKAXES } from '*api/acquisition/Tools.js';
  export { PICKPOCKET_TARGET_NAMES } from '*api/thieving/targets.js';
  export { PICKPOCKET_TARGETS } from '*data/pickpocketTargets.js';
  export { Player } from '*api/model/Player.js';
  export { Players } from '*api/players/Players.js';
  export { Quests } from '*api/ui/questlog/Quests.js';
  export { RANDOM_EVENT_CASKET_ID } from '*api/bank/Banking.js';
  export { reader } from '*adapter/ClientAdapter.js';
  export { resolveCowLocation } from '*data/cowKillerLocations.js';
  export { resolveRockIds } from '*data/miningRocks.js';
  export { ROCK_OPTIONS } from '*data/miningRocks.js';
  export { ROCK_TYPES } from '*data/miningRocks.js';
  export { RUNE_OPTIONS } from '*data/runeCraftLocations.js';
  export { RUNES } from '*data/runeCraftLocations.js';
  export { Shop } from '*api/shop/Shop.js';
  export { shouldBankNow } from '*api/bank/bankRules.js';
  export { Skills } from '*api/skills/Skills.js';
  export { Special } from '*api/combat/Special.js';
  export { TaskBot } from '*api/bot/Bot.js';
  export { Tile } from '*geometry/Tile.js';
  export { TINDERBOX } from '*api/acquisition/Tools.js';
  export { tinderboxReq } from '*api/acquisition/Tools.js';
  export { toolKeepNames } from '*api/acquisition/Tools.js';
  export { toolKitLabel } from '*api/acquisition/Tools.js';
  export { toolRestockPlan } from '*api/acquisition/Tools.js';
  export { Trade } from '*api/trade/Trade.js';
  export { Traversal } from '*api/walking/Traversal.js';
  export { TreeBot } from '*api/bot/Bot.js';
  export { withdrawOp } from '*api/bank/Bank.js';

  export type { WorldTile, PaintContext, Task, SettingsView } from '*api/bot/Bot.js';
  export type { InvItem } from '*api/inventory/Inventory.js';
  export type { SettingsSchema } from '*runtime/Settings.js';
  export type { MeleeCombatStyle } from '*api/combat/CombatStyle.js';

  export function acquireKeepNames(...args: unknown[]): unknown;
  export const AcquireTask: unknown;
  export const ALL_FISHING_GEAR_NAMES: unknown;
  export const apiVersion: unknown;
  export const Area: unknown;
  export const AXE_BAR_FOR: unknown;
  export const AXE_SHOP_COSTS: unknown;
  export const AXE_SMITH_LEVEL: unknown;
  export function axeShopOffers(...args: unknown[]): unknown;
  export function bankDistance(...args: unknown[]): unknown;
  export function bestAffordableShopTier(...args: unknown[]): unknown;
  export function bestHeldToolNames(...args: unknown[]): unknown;
  export function bestOwnedTier(...args: unknown[]): unknown;
  export function bestSmithableAxe(...args: unknown[]): unknown;
  export const BOB_VENDOR: unknown;
  export function boothFields(...args: unknown[]): unknown;
  export const BranchTask: unknown;
  export const BROKEN_AXE: unknown;
  export function buyPlansCost(...args: unknown[]): unknown;
  export function canFundPlan(...args: unknown[]): unknown;
  export function coinsToWithdraw(...args: unknown[]): unknown;
  export const DEFAULT_BOOTH_NAME: unknown;
  export const DEFAULT_BOOTH_OP: unknown;
  export const events: unknown;
  export const FISHING_LOCATION_OPTIONS: unknown;
  export const FISHING_LOCATIONS: unknown;
  export const FISHING_METHOD_OPTIONS: unknown;
  export const FISHING_METHODS: unknown;
  export const FISHING_SHOP_COSTS: unknown;
  export function fishingGearShopCart(...args: unknown[]): unknown;
  export function fishingRestockPlan(...args: unknown[]): unknown;
  export function fishingShopCost(...args: unknown[]): unknown;
  export function fishingVendorFor(...args: unknown[]): unknown;
  export const FORGETFUL_BANK_ODDS: unknown;
  export const FORGETFUL_BANK_SETTING: unknown;
  export function gearKeepNames(...args: unknown[]): unknown;
  export function gearLabel(...args: unknown[]): unknown;
  export const GERRANT_ONLY_FISHING: unknown;
  export const GERRANT_VENDOR: unknown;
  export const HARRY_VENDOR: unknown;
  export function hasAll(...args: unknown[]): unknown;
  export function hasFishingGear(...args: unknown[]): unknown;
  export function held(...args: unknown[]): unknown;
  export function isCowFieldLootTile(...args: unknown[]): unknown;
  export function isFishingBaitPiece(...args: unknown[]): unknown;
  export const LeafTask: unknown;
  export function locationOptions(...args: unknown[]): unknown;
  export const MAP_SQUARE: unknown;
  export const MINING_LOCATION_OPTION_LABELS: unknown;
  export const MINING_LOCATION_OPTIONS: unknown;
  export const MINING_LOCATIONS: unknown;
  export function miningLocationLabel(...args: unknown[]): unknown;
  export function missingFishingGear(...args: unknown[]): unknown;
  export const NAV_PURE_WALK: unknown;
  export const NAV_WITH_TELES: unknown;
  export const NEARBY_BANK_RADIUS: unknown;
  export function needsTollCoins(...args: unknown[]): unknown;
  export const NURMOF_VENDOR: unknown;
  export function parseToolAcquireMode(...args: unknown[]): unknown;
  export const PICKAXE_SHOP_COSTS: unknown;
  export function pickaxeShopOffers(...args: unknown[]): unknown;
  export function planAxeAcquire(...args: unknown[]): unknown;
  export function planBrokenToolRepair(...args: unknown[]): unknown;
  export function planFishingGearAcquire(...args: unknown[]): unknown;
  export function planFishingGearBuys(...args: unknown[]): unknown;
  export function planGatherToolAcquire(...args: unknown[]): unknown;
  export function planPickaxeAcquire(...args: unknown[]): unknown;
  export function registerScript(...args: unknown[]): unknown;
  export function resolveBankOpenRoute(...args: unknown[]): unknown;
  export function resolveDestination(...args: unknown[]): unknown;
  export function resolveFishingLocation(...args: unknown[]): unknown;
  export function resolveFishMethod(...args: unknown[]): unknown;
  export function resolveGatheringLocation(...args: unknown[]): unknown;
  export function resolveMiningLocation(...args: unknown[]): unknown;
  export function resolveWoodcuttingLocation(...args: unknown[]): unknown;
  export function sameMapSquare(...args: unknown[]): unknown;
  export function shopableMissingFishingGear(...args: unknown[]): unknown;
  export function shouldBootstrapTollCoins(...args: unknown[]): unknown;
  export function spotMatchesMethod(...args: unknown[]): unknown;
  export function surplusHeldToolNames(...args: unknown[]): unknown;
  export const TOLL_COIN_TARGET: unknown;
  export const TOOL_ACQUIRE_OPTIONS: unknown;
  export const TOOL_ACQUIRE_SETTING: unknown;
  export function toolAttackLevel(...args: unknown[]): unknown;
  export function toolsNeedingEquip(...args: unknown[]): unknown;
  export const VARROCK_ANVIL_BANK: unknown;
  export const VARROCK_ANVIL_STAND: unknown;
  export const WALK_DESTINATIONS: unknown;
  export const WALK_OPTIONS: unknown;
  export const WHIRLPOOL_IDS: unknown;
  export function withBaitTarget(...args: unknown[]): unknown;
  export const WOODCUTTING_LOCATION_OPTIONS: unknown;
  export const WOODCUTTING_LOCATIONS: unknown;
}

declare module '*adapter/ClientAdapter.js' {
  export type InvItemSnapshot = { slot: number; id: number; name: string | null; count: number; ops?: string[]; comId?: number };

  export interface WorldTile { x: number; z: number; level: number }
  export type ChatLine = { seq?: number; text: string; [key: string]: unknown };
  export interface SnapshotItem {
    slot: number;
    id: number;
    name: string | null;
    count: number;
    ops: string[];
    comId: number;
  }
  export const reader: {
    worldTile(): WorldTile | null;
    serverTile(): WorldTile | null;
    selfSlot(): number;
    inventorySize(): number;
    inventory(): SnapshotItem[];
    bankSideItems(): SnapshotItem[];
    sceneState(): number;
    ingame(): boolean;
    npcs(): unknown[];
    locs(): unknown[];
    players(): unknown[];
    groundItems(): unknown[];
    equipment(): unknown[];
    modals(): { main: number; chat: number; side: number };
    chatContinueComId(): number;
    chatOptions(): Array<{ text: string; comId: number }>;
    chatModalTexts(): string[];
    activeSideTab(): number;
    localPlayerName(): string | null;
    combatLevel(): number;
    selfChat(): string | null;
    hintTile(): { x: number; z: number } | null;
    npcBox(index: number): Array<{ x: number; y: number }> | null;
    retaliateControls(): { onComId: number; offComId: number } | null;
    inCombat(): boolean;
    selfAnim(): number;
    selfTarget(): { kind: number; index: number };
    selfFaceEntity(): number;
    energy(): number;
    varp(index: number): number;
    stat(i: number): { name: string; xp: number; base: number; effective: number };
    skillCount(): number;
    sideTabInterface(tab: number): number;
    selectButtonLabelsByVarp(_root: unknown, _varp: unknown): Array<{ mode: number; label: string }>;
    selectButtonByVarp(_root: unknown, _varp: unknown, mode: number): number;
    targetButtonByBase(_root: unknown, label: string): number;
    bankComId(): number;
    ifText(comId: number): string | null;
    countDialogOpen(): boolean;
    makeProducts(): Array<{ name: string; buttons: Array<{ qty: unknown; comId: unknown }> }>;
    toLocal(x: number, z: number): { lx: number; lz: number } | null;
  };
  export const actions: {
    closeModal(): boolean;
    ifButton(componentId: number): boolean;
    clickSideTab(tab: number): boolean;
    setRetaliate(on: boolean): boolean;
    setRun(on: boolean): boolean;
    walkTo(lx: number, lz: number): boolean;
    answerCountDialog(value: number): boolean;
  };
}

declare module '*api/acquisition/Tools.js' {
  export type ToolReq = { name?: string; [key: string]: unknown };

  export const TINDERBOX: any;
  export const HAMMER: any;
  export const KNIFE: any;
  export const CHISEL: any;
  export const NEEDLE: any;
  export const AXES: any;
  export const PICKAXES: any;
  export function exactTool(name: string): unknown;
  export function tinderboxReq(): unknown;
  export function axeReq(): unknown;
  export function pickaxeReq(): unknown;
  export function toolKeepNames(reqs: unknown): unknown;
  export function hasToolReq(req: unknown, skillLevel: unknown, count: number | ((name: string) => number)): boolean;
  export function hasAllTools(reqs: unknown, skillLevel: unknown, count: number | ((name: string) => number)): boolean;
  export function bestAxe(level: number, available: unknown): unknown;
  export function bestPickaxe(level: number, available: unknown): unknown;
  export function bestFromTiers(level: number, tiers: unknown, available: unknown): unknown;
  export function canWieldTool(name: string, attack: unknown): boolean;
  export function toolRestockPlan(reqs: unknown, skillLevel: unknown, invCount: unknown, bankCount: unknown): unknown;
  export function missingToolLabels(): unknown;
  export function toolKitLabel(): unknown;
  export function bankHasBetterGatherTool(): unknown;
}

declare module '*api/ai/clues/ClueExecutor.js' {
  export class ClueExecutor {
    static setTeleports(enabled: unknown): unknown;
  }
}

declare module '*api/ai/clues/SolveClue.js' {
  export class SolveClue {
    constructor(hostArg: unknown);
    host: unknown;
    token: unknown;
    status: unknown;
    clueStatus(): unknown;
    noteDeath(): unknown;
    ownsEquipment(): unknown;
    retry(): unknown;
    enabled(): unknown;
    hooks(): unknown;
    validate(): boolean;
    execute(): Promise<void>;
  }
  export function heldClueLikeId(): unknown;
  export function walkToBank(tile: unknown, log: (msg: string) => void): unknown;
}

declare module '*api/ai/clues/bankAccess.js' {
  export function openClueBank(_log: (msg: string) => void): unknown;
}

declare module '*api/ai/clues/cluePaint.js' {
  export function paintClueProgress(p: unknown, idle?: unknown): unknown;
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
  export function crossesClueDuel(dest: unknown): unknown;
  export function walkAcrossClueDuel(dest: unknown, radius: number, log: (msg: string) => void): Promise<boolean>;
}

declare module '*api/ai/quests/defs/murder/areas.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export const MURDER_NAME: string;
  export const MURDER_OBJ: {
    POT: number;
    POT_FLOUR: number;
    FLYPAPER: number;
    DAGGER: number;
    DAGGER_DUST: number;
    UNKNOWN_PRINT: number;
    KILLERS_PRINT: number;
    THREAD_RED: number;
    THREAD_GREEN: number;
    THREAD_BLUE: number;
  };
  export const MURDER_LOC: {
    WINDOW: string;
    FLOUR_BARREL: string;
    SACKS: string;
  };
  export const MURDER_TILE: {
    BANK: WorldTile;
    GUARD: WorldTile;
    STUDY: WorldTile;
    FLOUR_BARREL: WorldTile;
    SACKS: WorldTile;
    SALESMAN: WorldTile;
    ARHEIN: WorldTile;
  };
}

declare module '*api/ai/quests/engine/QuestEngine.js' {
  export const QuestEngine: any;
}

declare module '*api/ai/quests/exec/primitives.js' {
  export function pickPreferred(options: unknown, prefer: unknown): unknown;
  export function pickByLine(): unknown;
  export function isUnderground(t: unknown): boolean;
  export function needsHop(): unknown;
  export function walkWithHops(dest: unknown, radius: number, hops: unknown, log: (msg: string) => void): Promise<boolean>;
  export function gotoNpc(stop: unknown): Promise<boolean>;
  export function driveDialog(prefer: unknown, log: (msg: string) => void, gapMs: unknown): Promise<boolean>;
  export function openDialogue(npcName: unknown, log: (msg: string) => void): Promise<boolean>;
  export function talkThrough(npcName: unknown, prefer: unknown, log: (msg: string) => void, gapMs: unknown): Promise<boolean>;
  export function talkStrict(npcName: unknown, prefer: unknown, log: (msg: string) => void): Promise<unknown>;
  export function talkChoosingBy(): Promise<boolean>;
  export function talkOp(actions: unknown): unknown;
}

declare module '*api/bank/Bank.js' {
  export type BackpackItem = { name: string | null; count: number; id: number; slot?: number };

  export interface WorldTile { x: number; z: number; level: number }
  export interface BankItem {
    name: string | null;
    count: number;
    id: number;
    slot: number;
    comId: number;
    ops: string[];
    noted: boolean;
    cert: number;
  }
  export interface BankAccess {
    name?: string;
    op?: string;
    openFirst?: { name: string; op: string };
  }
  export interface NpcBankAccess {
    name: string;
    op: string;
    choose?: string;
  }
  export function withdrawOp(ops: string[], which: unknown): string | null;
  export const Bank: {
    isOpen(): boolean;
    loaded(): boolean;
    ready(): boolean;
    items(): BankItem[];
    count(name: string): number;
    deposit(name: string): Promise<boolean>;
    depositInventory(): Promise<void>;
    depositAllMatching(predicate: (name: string, id: number) => boolean, log?: (msg: string) => void): Promise<void>;
    depositAllExcept(keep: Iterable<string>): Promise<void>;
    withdraw(name: string, amount?: number | string): Promise<boolean>;
    setNoteMode(on: boolean): Promise<boolean>;
    close(): Promise<boolean>;
    withdrawById(id: number, op?: string): Promise<boolean>;
    withdrawX(name: string, count: number): Promise<boolean>;
    withdrawXById(id: number, count: number, landsAsId?: number): Promise<boolean>;
    openBooth(stand?: WorldTile | null, boothName?: string, op?: string, _log?: (msg: string) => void): Promise<boolean>;
    openNearest(boothName?: string, op?: string, log?: (msg: string) => void): Promise<boolean>;
    openNearestWorld(): Promise<boolean>;
    waitReady(timeoutMs?: number, log?: (msg: string) => void): Promise<boolean>;
    snapshotReady(): boolean;
    snapshotGeneration(): number;
    waitSnapshotAfter(generation: number, timeoutMs?: number): Promise<boolean>;
    countById(id: number): number;
    withdrawLoad(name: string): Promise<boolean>;
    openNearestAccess(access?: BankAccess, log?: (msg: string) => void): Promise<boolean>;
    openNpcAccess(access: NpcBankAccess, log?: (msg: string) => void): Promise<boolean>;
  };
}

declare module '*api/bank/BankLocations.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export type BankAccess = {
    name: string;
    op: string;
    openFirst?: { name: string; op: string };
  };
  export type NpcBankAccess = {
    name: string;
    op: string;
    choose?: string;
  };
  export type BankLocation = {
    name?: string;
    tile?: Tile;
    approach?: Tile;
    access?: BankAccess;
    npcAccess?: NpcBankAccess;
    [key: string]: unknown;
  };
  export const USE_MAGE_BANK: string;
  export const USE_ZANARIS_BANK: string;
  export const BANK_LOCATIONS: BankLocation[];
  export function approachOf(bank: BankLocation | unknown): Tile | WorldTile | undefined;
  export function bankUnlocked(bank: BankLocation | unknown): boolean;
  export function nearestBank(here: WorldTile | unknown): BankLocation | null;
  export function nearestBanks(here: WorldTile | unknown): BankLocation[];
  export function nearestUsableBank(here: WorldTile | unknown, usable: unknown): BankLocation | null;
  export function nearestBankReachable(here: WorldTile | unknown, _navigator: unknown): Promise<BankLocation | null>;
}

declare module '*api/bank/Banking.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export type BankDestination = {
    name?: string;
    tile?: WorldTile;
    [key: string]: unknown;
  };
  export const COMMON_BANK_LOOT: string[];
  export const RANDOM_EVENT_CASKET_ID: number;
  export function matchesCommonBankLoot(name: string, id?: number): boolean;
  export function depositMatcher(own: (name: string, id?: number) => boolean, includeCommon: unknown): (name: string, id?: number) => boolean;
  export function depositAllExcept(keep: Iterable<string> | null | undefined): (name: string) => boolean;
  export const PERIODIC_BANK_SETTINGS: {
    bankStrategy: unknown;
    bankEveryItems: unknown;
    bankEveryMinutes: unknown;
    bankCommonJunk: unknown;
  };
  export function parseBankStrategy(label: string): string;
  export const Banking: {
    open(opts?: unknown): Promise<boolean>;
    bankNearest(opts?: unknown): Promise<boolean>;
  };
}

declare module '*api/bank/bankOps.js' {
  export function withdrawOp(ops: unknown, which: string): unknown;
}

declare module '*api/bank/bankQuestJunk.js' {
  export type QuestJunkFinding = { quest?: string; items?: unknown; [key: string]: unknown };

  export const QUEST_JUNK: any;
  export function findQuestJunk(): unknown;
}

declare module '*api/bank/bankRules.js' {
  export function shouldBankNow(): unknown;
  export function isDisposableGatherJunk(): boolean;
  export function parseBankStrategy(label: string): unknown;
  export const PERIODIC_BANK_SETTINGS: {
    bankStrategy: unknown;
    bankEveryItems: unknown;
    bankEveryMinutes: unknown;
    bankCommonJunk: unknown;
  };
  export const COMMON_BANK_LOOT: any;
  export function matchesCommonBankLoot(name: string, id?: number): unknown;
  export function depositMatcher(own: unknown, includeCommon: unknown): unknown;
  export function depositAllExcept(keep: unknown): unknown;
}

declare module '*api/bank/bankSort.js' {
  export type BankSortResult = { moved?: number; [key: string]: unknown };
  export type BankCategory = string;

  export const ARRANGE_SWAP_COM: any;
  export const ARRANGE_INSERT_COM: any;
  export const BANK_INSERT_VARP: any;
  export function sortBank(opts?: unknown): Promise<BankSortResult>;
}

declare module '*api/bank/bankSortRules.js' {
  export const CATEGORY_ORDER: any;
  export function categoryOf(): unknown;
  export function isUnmatched(): boolean;
}

declare module '*api/bot/Bot.js' {
  export interface SettingsView {
    str(name: string, fallback?: string): string;
    num(name: string, fallback?: number): number;
    bool(name: string, fallback?: boolean): boolean;
    tile(name: string, fallback?: WorldTile | null): WorldTile | null;
    list(name: string, fallback?: string[]): string[];
  }
  export interface WorldTile { x: number; z: number; level: number }
  export type PaintContext = CanvasRenderingContext2D;
  export interface ChatMessage {
    text: string;
    [key: string]: unknown;
  }
  export interface Task {
    validate(): boolean | Promise<boolean>;
    execute(): void | Promise<void>;
  }
  export type LoopCadence = number | { kind: string; ticks?: number } | null;
  export type LoopResult = void | number | Promise<void | number>;
  export class LoopingBot {
    loopDelay: number;
    loopCadence: LoopCadence;
    onStart(): void | Promise<void>;
    onStop(): void | Promise<void>;
    onPause(): void;
    onResume(): void;
    onPaint(ctx?: CanvasRenderingContext2D): void;
    loop(): LoopResult;
    recoveryAnchor(): WorldTile | null;
    grindTargets(): unknown[];
    ignoredRandoms(): unknown[];
    on(event: 'chat.message', cb: (payload: ChatMessage) => unknown): void;
    on(event: string, cb: (payload: unknown) => unknown): void;
    log(message: unknown): void;
    get settings(): SettingsView;
  }
  export class TaskBot extends LoopingBot {
    constructor();
    add(...tasks: Task[]): void;
    loop(): Promise<void | number>;
  }
  export class TreeBot extends LoopingBot {
    root(): unknown;
  }
  export class AbstractBot {
    loopDelay: number;
    loopCadence: LoopCadence;
    onStart(): void | Promise<void>;
    onStop(): void | Promise<void>;
    onPause(): void;
    onResume(): void;
    onPaint(ctx?: CanvasRenderingContext2D): void;
    loop(): LoopResult;
    recoveryAnchor(): WorldTile | null;
    grindTargets(): unknown[];
    ignoredRandoms(): unknown[];
    on(event: 'chat.message', cb: (payload: ChatMessage) => unknown): void;
    on(event: string, cb: (payload: unknown) => unknown): void;
    log(message: unknown): void;
    get settings(): SettingsView;
  }
}

declare module '*api/chatbox/gameMessages.js' {
  export type ChatLine = { seq: number; text: string };
  export const CANT_REACH: RegExp;
  export const WRONG_SIDE: RegExp;
  export const GameMessages: {
    mark(): number;
    since(mark: number): ChatLine[];
    sawSince(mark: number, pattern: RegExp | { test(text: string): boolean }): boolean;
  };
}

declare module '*api/combat/CombatStyle.js' {
  export type MeleeCombatStyle = "attack" | "strength" | "controlled" | "defence";
  export type RangeStyle = "accurate" | "rapid" | "longrange";
  export interface CombatStyleResolution {
    requested: string;
    effective: string;
    mode: number;
    label: string;
  }
  export const COMBAT_STYLE_OPTIONS: MeleeCombatStyle[];
  export const RANGE_STYLE_OPTIONS: RangeStyle[];
  export function parseCombatStyle(name: string): MeleeCombatStyle;
  export function tryParseCombatStyle(name: string): MeleeCombatStyle | null;
  export function resolveSplitCombatSettings(rawCombatStyle: string, rawMeleeStyle?: string): {
    kind: "melee" | "mage" | "range";
    meleeStyle: MeleeCombatStyle;
    legacyMigrated: MeleeCombatStyle | null;
  };
  export function parseRangeStyle(name: string): number;
  export function describeCombatStyle(resolution: CombatStyleResolution | unknown): string;
}

declare module '*api/combat/CombatStyleLogic.js' {
  export const ATTACKSTYLE_MAGIC_VARP: any;
  export const AUTOCAST_ARMED: any;
  export function runesPerCast(spellName: unknown, wielded: unknown): unknown;
  export function spellButtonCom(spellName: unknown): unknown;
  export function castsAvailable(spellName: unknown, wielded: unknown, held: unknown): unknown;
  export function runeWithdrawList(spellName: unknown, wielded: unknown, casts: unknown): unknown;
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
    energy(): number;
    armed(): unknown;
    wielded(): unknown;
    cost(weaponName: unknown): unknown;
    ready(weaponName: unknown): boolean;
    barComponent(): unknown;
    arm(): Promise<boolean>;
  };
}

declare module '*api/combat/boostPotions.js' {
  export type PotionPlan = { name?: string; [key: string]: unknown };

  export const BOOST_POTIONS: any;
  export const SUPER_ATTACK: any;
  export const SUPER_STRENGTH: any;
  export const EMPTY_VIAL: any;
  export const BOOST_FLOOR: any;
  export function boostFaded(base: unknown, effective: unknown, floor: unknown): unknown;
  export function plannedPotions(carry: unknown): unknown;
  export function potionToSip(s: unknown): unknown;
}

declare module '*api/combat/eatTiming.js' {
  export const URGENT_HP_FRACTION: any;
  export function shouldHoldEat(input: unknown): unknown;
  export class AttackClock {
    observe(anim: unknown, tick: unknown): unknown;
    attackedThisTick(tick: unknown): unknown;
    reset(): unknown;
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
  export function swingStartedThisTick(): unknown;
  export function buryOneInFight(boneName: unknown): Promise<boolean>;
}

declare module '*api/combat/food.js' {
  export const FOOD_OPTIONS: string[];
  export const MIN_EAT_HP: number;
  export function foodHealAmount(foodName: string): number;
  export function foodForms(foodName: string): string[];
  export function isFoodItem(name: string, foodName: string): boolean;
  export function foodCount(items: unknown, foodName: string): number;
  export function eatAtHpThreshold(maxHp: number, heal: number, minHp: unknown): number;
  export function shouldEatToUseFood(opts: unknown): boolean;
  export function shouldEatFood(foodName: string, opts?: unknown): boolean;
}

declare module '*api/combat/hunting/combat.js' {
  export type CombatHost = { log?: (msg: string) => void; [key: string]: unknown };

  class HuntTask {
    constructor(family: string, host: unknown, site: unknown);
    family: unknown;
    host: unknown;
    site: unknown;
    hooks: unknown;
    token: unknown;
    validate(): boolean;
    run(): Promise<boolean>;
  }
  export function siteArgs(site: unknown, extra: unknown): unknown;
  export function hooksOf(host: unknown, site: unknown, leave: unknown): unknown;
  export function anchorFor(site: unknown, style: unknown, index: number): unknown;
  export class Fight extends HuntTask {
    constructor(host: unknown, site: unknown);
    execute(): Promise<void>;
    reset(): unknown;
    interruptWatch(): unknown;
    blocksLoot(): unknown;
  }
  export class Retreat extends HuntTask {
    constructor(host: unknown, site: unknown);
    execute(): Promise<void>;
  }
  export class HoldSafespot extends HuntTask {
    constructor(host: unknown, site: unknown);
    execute(): Promise<void>;
  }
  export class WalkToSpot extends HuntTask {
    constructor(host: unknown, site: unknown);
    execute(): Promise<void>;
  }
  export class EnterLair extends HuntTask {
    constructor(host: unknown, site: unknown);
    execute(): Promise<void>;
  }
  export function cell(host: unknown, site: unknown): Promise<unknown>;
  export function leaveLair(host: unknown, site: unknown): Promise<unknown>;
  export function bankRoutine(host: unknown, site: unknown, opts: unknown): Promise<unknown>;
}

declare module '*api/combat/hunting/guarded.js' {
  export const LOOT_GUARD: any;
  export function guarded(drop: unknown, bodies: unknown, radius?: number): unknown;
}

declare module '*api/combat/hunting/logic.js' {
  export type Style = string;
  export type KeyState = { [key: string]: unknown };

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
  export function nextSafespot(s: unknown): unknown;
  export function hurtOnSpot(s: unknown): unknown;
  export function retreatDue(s: unknown): unknown;
  export function lootHalts(s: unknown): unknown;
  export function holdDue(s: unknown): unknown;
  export function nearestSpot(from: unknown, spots: unknown): unknown;
  export function bodyOrigin(tile: unknown, size: unknown): unknown;
  export function noteSighting(prev: unknown, tile: unknown, now: unknown): unknown;
  export function settled(s: unknown, now: unknown, ms: number): unknown;
  export function retreatAim(a: unknown): unknown;
  export function chaseMode(style: unknown, fireAtRange: unknown): unknown;
  export function prayerFor(style: unknown, fireAtRange: unknown): unknown;
  export function prayerSipDue(points: unknown, max: unknown): unknown;
  export function lootReach(fireAtRange: unknown): unknown;
  export function attackRangeFor(style: unknown): unknown;
  export function engageRangeFor(style: unknown): unknown;
  export function gapTo(from: unknown, tile: unknown, size: unknown): unknown;
  export function shieldGate(style: unknown, fireAtRange: unknown, hasShield: unknown): unknown;
  export function styleGate(style: unknown, fireAtRange: unknown): unknown;
  export function antifireDue(s: unknown): unknown;
  export function antifireLapsed(sawShield: unknown, sawPotion: unknown): unknown;
  export function nextApproachIndex(stops: unknown, here: unknown): unknown;
  export function isClueObj(id: number): boolean;
  export function keyStatus(held: unknown, banked: unknown): unknown;
  export function wantsDrop(item: string, f: unknown): unknown;
  export function siteTileOf(schema: unknown, bag: unknown, key: string, site: unknown): unknown;
  export function keepDoses(potionDoses: unknown, antipoisonDoses: unknown, carriesAntipoison: unknown): unknown;
}

declare module '*api/combat/hunting/sites.js' {
  export type DragonSite = { name?: string; key?: string; [key: string]: unknown };

  export function inBox(b: unknown): unknown;
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
  export function needsShield(s: unknown, style: unknown): unknown;
  export function huntNames(s: unknown): unknown;
  export function standFor(s: unknown, n: number): unknown;
  export function siteFor(key: string): unknown;
}

declare module '*api/combat/hunting/supply.js' {
  export type BankOpts = { [key: string]: unknown };

  export const SHIELD: any;
  export const POISONED: any;
  export const COINS: any;
  export const ANTIPOISON_LABEL: any;
  export const ANTIPOISON_DOSES: any;
  export const PRAYER_LABEL: any;
  export const PRAYER_DOSES: any;
  export const ANTIFIRE_LABEL: any;
  export const ANTIFIRE_DOSES: any;
  export function antipoisonPlan(want: unknown): unknown;
  export function prayerPlan(want: unknown): unknown;
  export function antifirePlan(want: unknown): unknown;
  export function doseToDrink(count: number, doses?: unknown): unknown;
  export function escapeRunesFor(teleportId: unknown): unknown;
  export function inCell(): unknown;
  export function enterLair(h: unknown, site: unknown): Promise<boolean>;
  export function feePrepaid(site: unknown): unknown;
  export function acquireKey(h: unknown, site: unknown): Promise<boolean>;
  export function leaveCell(h: unknown): Promise<boolean>;
  export function teleportOut(h: unknown, site: unknown): Promise<boolean>;
  export function waitFed(cond: () => boolean, ms: number): Promise<boolean>;
  export function bankRoutine(h: unknown, site: unknown, opts: unknown): Promise<boolean>;
  export function walkApproach(): unknown;
  export function withdrawTo(): unknown;
  export function leaveLair(host: unknown, site: unknown): Promise<unknown>;
}

declare module '*api/combat/keepList.js' {
  export function combatKeepNames(o: unknown): unknown;
}

declare module '*api/combat/meleeWeapons.js' {
  export function bestMeleeWeapon(available: unknown, pick: unknown): unknown;
  export function knownMeleeWeapon(names: unknown): unknown;
}

declare module '*api/combat/ranged.js' {
  export const RANGED_WEAPONS: any;
  export const ROCK_CRAB_RANGED_WEAPONS: any;
  export function rangeLoadoutOf(weapon: unknown, ammo: unknown): unknown;
  export function rockCrabRangeLoadout(...args: unknown[]): unknown;
  export function rangeSupplyEmpty(equipped: unknown, carried: unknown, ground: unknown): unknown;
}

declare module '*api/combat/rangedSettings.js' {
  export const CUSTOM_RANGED_SETTINGS: {
    customBow: unknown;
    customAmmo: unknown;
  };
  export function rangedItem(settings: unknown, key: string, fallback: unknown): unknown;
}

declare module '*api/cooking/CookLocations.js' {
  export type CookLocation = { name: string; [key: string]: unknown };
  export const COOK_LOCATION_OPTIONS: string[];
  export function cookLocation(name: string): CookLocation | null;
  export function resolveCookLocation(setting: unknown, from: unknown, unlocked?: (loc: CookLocation) => boolean): CookLocation | null;
  export const CUSTOM_LOCATION: string;
  export const COOK_LOCATIONS: CookLocation[];
}

declare module '*api/duel/ClueDuel.js' {
  export const CLUE_DUEL_LOBBY: any;
  export const CLUE_DUEL_OPTIONS: any;
  export function clueDuelName(name: string): unknown;
  export function leaveClueDuel(log: (msg: string) => void): Promise<boolean>;
  export class ClueDuelHandshake {
    constructor(partner: unknown, initiator: unknown, log: (msg: string) => void);
    partner: unknown;
    initiator: unknown;
    log: unknown;
    tick(): Promise<boolean>;
  }
  export class ClueDuelHelper {
    constructor(partner: unknown, log: (msg: string) => void);
    partner: unknown;
    log: unknown;
    validate(): boolean;
    execute(): Promise<void>;
  }
}

declare module '*api/duel/Duel.js' {
  export type Rect = { x: number; y: number; w?: number; h?: number; [key: string]: unknown };

  export const DUEL_SELECT_MODAL: any;
  export const DUEL_CONFIRM_MODAL: any;
  export const DUEL_WIN_MODAL: any;
  export const DUEL_FIGHT_ARENAS: any;
  export function parseDuelPartnerHeader(header: unknown): unknown;
  export function fightArenaAt(tile: unknown): unknown;
  export const Duel: {
    offerOpen(): unknown;
    confirmOpen(): unknown;
    winOpen(): unknown;
    active(): unknown;
    partner(): unknown;
    waitingForOther(): unknown;
    challenge(player: unknown): unknown;
    fight(player: unknown): unknown;
    accept(): Promise<boolean>;
    cancel(): Promise<boolean>;
    closeWin(): Promise<boolean>;
  };
}

declare module '*api/equipment/Equipment.js' {
  export interface EquipmentItem {
    name: string;
    count: number;
    id: number;
  }
  export const Equipment: {
    items(): EquipmentItem[];
    contains(name: string): boolean;
    equip(name: string): Promise<boolean>;
    unequip(name: string): Promise<boolean>;
  };
}

declare module '*api/execution/EventSignal.js' {
  export const EventSignal: {
    pending(): boolean;
    setInterrupt(callback: (() => boolean) | null | undefined): void;
    ignoredRandoms(): unknown[];
  };
}

declare module '*api/execution/Execution.js' {
  export const Execution: {
    delay(ms: number): Promise<void>;
    delayTicks(n: number): Promise<void>;
    delayUntil(cond: () => boolean, timeoutMs?: number): Promise<boolean>;
    delayUntilTicks(cond: () => boolean, maxTicks: number): Promise<boolean>;
    noteProgress(): void;
  };
  export function parkMachine(handle: unknown): Promise<unknown>;
}

declare module '*api/firemaking/Firemaking.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export type BurnDir = { dx: number; dz: number };
  export type FirePlot = { origin?: WorldTile; [key: string]: unknown };
  export const TINDERBOX: any;
  export const CANT_LIGHT: any;
  export const FIRE_START_TICKS: any;
  export const FIRE_LIGHT_TICKS: any;
  export const BURN_WEST: { dx: number; dz: number };
  export const BURN_DIRS: BurnDir[];
  export const FIRE_SPOTS: any;
  export const FIRE_SPOT_OPTIONS: any;
  export function localFirePlot(origin: unknown, half: number): FirePlot;
  export const LOG_LEVELS: any;
  export function tileKey(t: unknown): string;
  export class NoLightTiles {
    add(tile: unknown): unknown;
    has(tile: unknown): boolean;
    get size(): number;
    merge(occupied: unknown): unknown;
    clear(): void;
  }
  export function inFirePlot(t: unknown, plot: unknown): boolean;
  export function burnLaneWant(logCount: unknown): unknown;
  export function isBurnWest(dir: unknown): boolean;
  export function fireReactionTicks(): number;
  export function runInDir(from: unknown, plot: unknown, dir: unknown, occupied: unknown, walkable: unknown, canStep: unknown, cap: number): unknown;
  export function findBurnLane(plot: unknown, here: unknown, occupied: unknown, want?: unknown, _walkable?: unknown, _canStep?: unknown, directions?: unknown): unknown;
  export function lightFire(logName: string): Promise<string>;
}

declare module '*api/firemaking/LightFire.js' {
  export function lightFire(logName: string): Promise<string>;
}

declare module '*api/game/Game.js' {
  export interface WorldTile {
    x: number;
    z: number;
    level: number;
  }
  export interface CombatStyleResolution {
    requested: string;
    effective: string;
    mode: number;
    label: string;
  }
  export const Game: {
    ingame(): boolean;
    tile(): WorldTile | null;
    tick(): number;
    inCombat(): boolean;
    animating(): boolean;
    runEnabled(): boolean;
    autoRetaliate(): boolean;
    autoRetaliateOn(): boolean;
    myName(): string | null;
    combatMode(): number;
    combatStyles(): Array<{ mode: number; label: string }> | null;
    hasCombatStyle(style: string): boolean;
    combatStyleResolution(style: string): CombatStyleResolution | null;
    setCombatMode(mode: number): boolean;
    setCombatStyle(style: string | number): boolean;
    setAutoRetaliate(on: boolean): boolean;
    openSideTab(tab: number): Promise<boolean>;
    castOnItem(spell: string, item: string | { name?: string } | null): Promise<boolean>;
    castOnLoc(spell: string, loc: { name?: string; tile?: () => WorldTile } | WorldTile | null): Promise<boolean>;
    teleport(name: string): Promise<boolean>;
    energy(): number;
    weight(): number;
    cameraYaw(): number;
    cameraPitch(): number;
    setCameraYaw(yaw: number): boolean;
    combatStyleMode(): number;
    sceneReady(): boolean;
    sceneState(): number;
    attackedByPlayer(): boolean;
    castOnNpc(): Promise<boolean>;
  };
}

declare module '*api/grounditems/GroundItems.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export class GroundItem {
    constructor(row: unknown);
    snap: unknown;
    get name(): string | null;
    get id(): number;
    get count(): number;
    tile(): Tile;
    distance(): number;
    actions(): string[];
    interact(action: string): boolean;
  }
  export interface GroundItemQuery {
    name(...names: string[]): GroundItemQuery;
    action(action: string): GroundItemQuery;
    within(dist: number): GroundItemQuery;
    withinOf(origin: WorldTile, dist: number): GroundItemQuery;
    where(pred: (entity: GroundItem) => boolean): GroundItemQuery;
    forEachMatch(seen: (entity: GroundItem) => boolean): void;
    results(): GroundItem[];
    nearest(): GroundItem | null;
    first(): GroundItem | null;
    exists(): boolean;
    count(): number;
    inside(_area?: unknown): GroundItemQuery;
    nearestPreferLocal(_preferRadius?: number): GroundItem;
  }
  export const GroundItems: {
    query(): GroundItemQuery;
  };
}

declare module '*api/inventory/Inventory.js' {
  export interface InvItem {
    name: string;
    count: number;
    id: number;
    slot: number;
    noted: boolean;
    cert: number;
    interact(action: string): boolean;
    actions(): string[];
    useOn(target: unknown): boolean;
  }
  export const Inventory: {
    count(name: string): number;
    countById(id: number): number;
    first(name: string): InvItem | null;
    items(): InvItem[];
    contains(name: string): boolean;
    used(): number;
    isFull(): boolean;
    free(): number;
  };
}

declare module '*api/inventory/packRules.js' {
  export function matchesAny(name: string, patterns: unknown): unknown;
  export function countMatching(items: unknown, patterns: unknown): number;
  export function slotsMatching(items: unknown, patterns: unknown): unknown;
  export function shouldBank(lootSlots: unknown, bankAt: number, invFull: unknown): unknown;
  export function shouldRestock(foodCount: number, threshold: number): unknown;
  export function shouldEat(hp: number, maxHp: number, heal: number, foodCount: number): unknown;
  export function shouldPanic(hpFrac: number, gate: number, foodCount: number): unknown;
}

declare module '*api/loadout/loadoutPlan.js' {
  export function foodOf(loadout: unknown, fallback: unknown): unknown;
  export function gearOf(loadout: unknown): unknown;
  export function suppliesOf(loadout: unknown): unknown;
  export function weaponOf(loadout: unknown, fallback?: unknown): unknown;
  export function scriptFood(bag: unknown, fallback: unknown): unknown;
  export function scriptFoods(bag: unknown, fallback: unknown): unknown;
}

declare module '*api/loadout/loadoutSetting.js' {
  export const LOADOUT_SETTING: {
    type: unknown;
    default: unknown;
    options: unknown;
    optionsFrom: unknown;
    label: unknown;
    help: unknown;
  };
  export function selectedLoadout(bag: unknown): unknown;
}

declare module '*api/locs/Locs.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export class Loc {
    constructor(row: unknown);
    snap: { name?: string | null; id?: number; x?: number; z?: number; level?: number; distance?: number; actions?: unknown };
    get name(): string | null;
    get id(): number;
    tile(): Tile;
    distance(): number;
    actions(): string[];
    interact(action: string): boolean;
  }
  export interface LocQuery {
    name(...names: string[]): LocQuery;
    action(action: string): LocQuery;
    within(dist: number): LocQuery;
    withinOf(origin: WorldTile, dist: number): LocQuery;
    where(pred: (entity: Loc) => boolean): LocQuery;
    forEachMatch(seen: (entity: Loc) => boolean): void;
    results(): Loc[];
    nearest(): Loc | null;
    first(): Loc | null;
    exists(): boolean;
    count(): number;
    inside(_area?: unknown): LocQuery;
    nearestPreferLocal(_preferRadius?: number): Loc;
  }
  export const Locs: {
    query(): LocQuery;
  };
}

declare module '*api/magic/Autocast.js' {
  export const Autocast: {
    armed(): unknown;
    staffTabAttached(): unknown;
    arm(spellName: unknown, log: (msg: string) => void): Promise<boolean>;
  };
}

declare module '*api/market/MarketMaker.js' {
  export const MarketMaker: any;
}

declare module '*api/market/catalog.js' {
  export interface Catalog {
    notedOf: Map<unknown, unknown>;
    unnotedOf: Map<unknown, unknown>;
    readonly byId: unknown;
    readonly items: unknown;
    readonly aliases: unknown;
  }
  export function liveCatalog(): Catalog;
  export function tradeable(id: number): boolean;
  export function clientName(_cat: Catalog | unknown, id: number): string | undefined;
  export function displayName(_cat: Catalog | unknown, id: number): string;
  export function notedId(_cat: Catalog | unknown, id: number): number | null;
  export function unnotedId(_cat: Catalog | unknown, id: number): number;
}

declare module '*api/model/Loc.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export class Loc {
    constructor(row: unknown);
    snap: { name?: string | null; id?: number; x?: number; z?: number; level?: number; distance?: number; actions?: unknown };
    get name(): string | null;
    get id(): number;
    tile(): Tile;
    distance(): number;
    actions(): string[];
    interact(action: string): boolean;
  }
}

declare module '*api/model/Npc.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export class Npc {
    constructor(row: unknown);
    snap: unknown;
    get name(): string | null;
    get id(): number;
    get index(): number;
    get inCombat(): boolean;
    get health(): number | null;
    get level(): number;
    get size(): number;
    networkOrigin(): Tile;
    networkTile(): Tile;
    targetsMe(): boolean;
    targetsAnotherPlayer(): boolean;
    tile(): Tile;
    distance(): number;
    actions(): string[];
    valid(): boolean;
    interact(action: string): boolean;
  }
}

declare module '*api/model/Player.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export class Player {
    constructor(row: unknown);
    snap: unknown;
    get name(): string | null;
    get index(): number;
    get inCombat(): boolean;
    get combatLevel(): number;
    targetsMe(): boolean;
    tile(): Tile;
    distance(): number;
    actions(): string[];
    interact(action: string): boolean;
  }
}

declare module '*api/npcs/Npcs.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export class Npc {
    constructor(row: unknown);
    snap: unknown;
    get name(): string | null;
    get id(): number;
    get index(): number;
    get inCombat(): boolean;
    get health(): number | null;
    get level(): number;
    get size(): number;
    networkOrigin(): Tile;
    networkTile(): Tile;
    targetsMe(): boolean;
    targetsAnotherPlayer(): boolean;
    tile(): Tile;
    distance(): number;
    actions(): string[];
    valid(): boolean;
    interact(action: string): boolean;
  }
  export interface NpcQuery {
    name(...names: string[]): NpcQuery;
    action(action: string): NpcQuery;
    within(dist: number): NpcQuery;
    withinOf(origin: WorldTile, dist: number): NpcQuery;
    where(pred: (entity: Npc) => boolean): NpcQuery;
    forEachMatch(seen: (entity: Npc) => boolean): void;
    results(): Npc[];
    nearest(): Npc | null;
    first(): Npc | null;
    exists(): boolean;
    count(): number;
    inside(_area?: unknown): NpcQuery;
    nearestPreferLocal(_preferRadius?: number): Npc;
  }
  export const Npcs: {
    query(): NpcQuery;
    all(): Npc[];
    nearest(count?: number): Npc[];
  };
  export function talkOp(actions: string[] | null | undefined): string | null;
}

declare module '*api/players/Players.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export class Player {
    constructor(row: unknown);
    snap: unknown;
    get name(): string | null;
    get index(): number;
    get inCombat(): boolean;
    get combatLevel(): number;
    targetsMe(): boolean;
    tile(): Tile;
    distance(): number;
    actions(): string[];
    interact(action: string): boolean;
  }
  export interface PlayerQuery {
    name(...names: string[]): PlayerQuery;
    action(action: string): PlayerQuery;
    within(dist: number): PlayerQuery;
    withinOf(origin: WorldTile, dist: number): PlayerQuery;
    where(pred: (entity: Player) => boolean): PlayerQuery;
    forEachMatch(seen: (entity: Player) => boolean): void;
    results(): Player[];
    nearest(): Player | null;
    first(): Player | null;
    exists(): boolean;
    count(): number;
    inside(_area?: unknown): PlayerQuery;
    nearestPreferLocal(_preferRadius?: number): Player;
  }
  export const Players: {
    query(): PlayerQuery;
    all(): Player[];
  };
}

declare module '*api/prayer/Prayer.js' {
  export const PROTECT_FROM_MAGIC: string;
  export const Prayer: {
    points(): number;
    max(): number;
    full(): boolean;
    known(name: string): boolean;
    available(name: string): boolean;
    active(name: string): boolean;
    set(name: string, on: boolean): Promise<boolean>;
    clear(): Promise<void>;
  };
}

declare module '*api/query/Query.js' {
  export function matchesEntityName(actual: unknown, configured: unknown): boolean;
  export class EntityQuery<T = unknown> {
    constructor(supplySnaps: () => unknown[], wrap: (snap: unknown) => T);
    supplySnaps: () => unknown[];
    wrap: (snap: unknown) => T;
    snapFilters: Array<(s: unknown) => boolean>;
    entityFilters: Array<(e: T) => boolean>;
    static fromSnapshots<U>(supply: () => unknown[], wrap: (snap: unknown) => U): EntityQuery<U>;
    name(...names: string[]): this;
    action(action: string): this;
    within(dist: number): this;
    withinOf(origin: { x: number; z: number; level?: number }, dist: number): this;
    where(pred: (entity: T) => boolean): this;
    forEachMatch(seen: (entity: T) => boolean): void;
    results(): T[];
    nearest(): T | null;
    first(): T | null;
    exists(): boolean;
    count(): number;
    inside(_area?: unknown): this;
    nearestPreferLocal(_preferRadius?: number): T;
  }
  export default EntityQuery;
}

declare module '*api/shop/BuyoutLogic.js' {
  export function buyoutPlan(rec: unknown, stock: unknown, coins: unknown, chosen: unknown): unknown;
}

declare module '*api/shop/Shop.js' {
  export const Shop: {
    isOpen(): boolean;
    stock(): Array<{ name: string | null; count: number; slot: number }>;
    player(): Array<{ name: string | null; count: number; slot: number }>;
    open(npcName?: string): Promise<boolean>;
    buy(name: string, qty?: number): Promise<number>;
    sell(name: string, qty?: number, pick?: (row: unknown) => boolean): Promise<number>;
    sellAll(name: string, pick?: (row: unknown) => boolean): Promise<number>;
    close(): Promise<void>;
    buyById(): never;
  };
}

declare module '*api/shop/types.js' {
  export type ShopRecord = { name?: string; [key: string]: unknown };

  export {};
}

declare module '*api/skills/Skills.js' {
  export const Skills: {
    index(name: string): number;
    xp(name: string): number;
    level(name: string): number;
    effective(name: string): number;
    hpFraction(): number;
  };
}

declare module '*api/sustain/Sustain.js' {
  export const Sustain: {
    hook: unknown;
    running: unknown;
    set(hook: unknown): unknown;
    run(): Promise<boolean>;
  };
}

declare module '*api/tasks/Anchor.js' {
  export const HOME_ARRIVE_RADIUS: number;
  export function shouldWalkHomeToGatherAnchor(_distToAnchor: unknown, _arriveRadius: unknown): unknown;
  export function shouldSoftHomeFromGatherMiss(_distToAnchor: unknown, _leash: unknown): unknown;
  export function beyondLeash(bot: unknown, here?: unknown, slack?: unknown): unknown;
  export function tileWithinLeash(bot: unknown, tile: unknown, slack?: unknown): unknown;
  export function resolveRunAnchor(here: unknown, locationSpot: unknown): unknown;
  export function createReturnToAnchorTask(bot: unknown, opts?: unknown): unknown;
}

declare module '*api/tasks/ContinueDialog.js' {
  export class ContinueDialog {
    constructor(onContinue?: () => void);
    onContinue?: () => void;
    validate(): boolean;
    execute(): Promise<void>;
  }
}

declare module '*api/tasks/DeathRecovery.js' {
  export class DeathRecovery {
    constructor(_bot: unknown, opts: unknown);
    opts: unknown;
    validate(): boolean;
    execute(): Promise<void>;
  }
}

declare module '*api/tasks/PeriodicBank.js' {
  export class PeriodicBank {
    constructor(opts: unknown);
    opts: unknown;
    validate(): boolean;
    execute(): Promise<void>;
  }
}

declare module '*api/thieving/CakeStall.js' {
  export function carriedCakes(): number;
  export function needsCakeRestock(target: unknown): boolean;
  export function stealCakes(opts?: unknown): Promise<string>;
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
  export function classifySteal(): unknown;
  export function shouldReset(consecutiveRefusals: unknown): unknown;
}

declare module '*api/thieving/stealRules.js' {
  export const THIEVER_BANKING_OPTIONS: any;
  export const STUN_COMBAT_TICKS: any;
  export function nextWithdrawChunk(need: unknown): unknown;
  export function withdrawTo(name: string, target: unknown, count: number): Promise<boolean>;
  export function closeBankAndConfirmCount(expected: unknown, count: number): Promise<boolean>;
  export function autoFoodBanking(mode: number): unknown;
  export function foodMatches(name: string, keyword: unknown): unknown;
  export function countFood(items: unknown, keyword: unknown): number;
  export function shouldRestockFood(enabled: unknown, foodCount: number, restockAt: unknown, bankablePackFull: unknown): unknown;
  export function safeToSteal(hpFraction: unknown, eatAt: unknown, foodCount: number): unknown;
  export function canStealNow(foodCount: number, hp: number, minEatHp: unknown, suicide: unknown): boolean;
}

declare module '*api/thieving/targets.js' {
  import Tile from '*geometry/Tile.js';
  export function targetSpot(target: unknown): { anchor: Tile; leash: number };
  export function requiredThieving(target: unknown): number;
  export const HOSTILE_NAMES: readonly string[];
  export function isHostileAttacker(c: unknown, maxDistance: unknown): boolean;
  export function chooseTarget(candidatesNearestFirst: unknown, reachable: unknown): unknown;
  export const PICKPOCKET_TARGET_NAMES: unknown;
  export const ARDOUGNE_PICKPOCKET_TARGETS: unknown;
}

declare module '*api/trade/PartnerTrade.js' {
  export const DEFAULT_TRADE_RANGE: any;
  export const MULE_MODE_OPTIONS: any;
  export function parsePartnerList(raw: unknown): unknown;
  export function namesMatch(a: unknown, b: unknown): unknown;
  export function isConfiguredPartner(name: string, partners: unknown): boolean;
  export function countOfferByName(items: unknown, itemName: unknown): number;
  export function decideReceiverOfferScreen(opts: unknown): unknown;
  export function decideGiverOfferScreen(myOfferSlots: unknown): unknown;
  export function parseMuleMode(raw: unknown): unknown;
  export function muleGathererHandoffActive(mode: number, partners: unknown, powerMode: unknown): unknown;
  export function muleReceiverActive(mode: number, partners: unknown): unknown;
  export function muleCookerActive(mode: number, partners: unknown): unknown;
  export function muleSupplierActive(mode: number, partners: unknown, powerMode: unknown): unknown;
  export function muleNonGathererActive(mode: number, partners: unknown): unknown;
  export function countOfferMatching(): number;
}

declare module '*api/trade/Trade.js' {
  export const Trade: {
    active(): unknown;
    onOfferScreen(): unknown;
    onConfirmScreen(): unknown;
    partner(): unknown;
    myOffer(): unknown;
    theirOffer(): unknown;
    request(playerName: unknown): Promise<boolean>;
    offerAll(name: string, pick: unknown): Promise<boolean>;
    offer(name: string, n: number, pick: unknown): Promise<boolean>;
    removeAll(): Promise<boolean>;
    accept(): Promise<boolean>;
    decline(): Promise<boolean>;
  };
}

declare module '*api/trade/drivePartnerTrade.js' {
  export function driveActivePartnerTrade(opts: unknown): Promise<boolean>;
}

declare module '*api/ui/dialogue/ChatDialog.js' {
  export const ChatDialog: {
    isOpen(): boolean;
    canContinue(): boolean;
    options(): string[];
    texts(): string[];
    isMakeMenu(): boolean;
    makeProducts(): string[];
    make(match?: string | null): Promise<boolean>;
    makeX(match?: string | null, count?: number): Promise<boolean>;
    continue(): Promise<boolean>;
    chooseOption(match?: string | null): Promise<boolean>;
    isMainMakePanel(): boolean;
    mainMakeProducts(): string[];
    makeFromPanel(match?: string | null, op?: string | null): Promise<boolean>;
    makeFromPanelMax(match?: string | null): Promise<boolean>;
    makeOne(): Promise<boolean>;
  };
}

declare module '*api/ui/questlog/Quests.js' {
  export type QuestStatus = string;
  export type QuestRow = { name: string; status: QuestStatus };

  export const Quests: {
    all(): QuestRow[];
    status(name: string): QuestStatus;
    journal(): unknown;
    points(): number;
  };
}

declare module '*api/ui/widgets/Modals.js' {
  export const Modals: {
    main(): unknown;
    isOpen(): boolean;
    close(): Promise<boolean>;
    closeIfOpen(): Promise<boolean>;
  };
}

declare module '*api/walking/DirectNavigator.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export const DirectNavigator: {
    walk(dest: WorldTile): boolean;
    walkTo(dest: WorldTile, radius?: number, timeoutMs?: number): Promise<boolean>;
  };
}

declare module '*api/walking/Reach.js' {
  export const Reach: {
    entityOp(opts: unknown): Promise<string>;
    npcDialog(opts: unknown): Promise<string>;
  };
}

declare module '*api/walking/Traversal.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export interface WalkOptions {
    radius?: number;
    timeoutMs?: number;
    attempts?: number;
    sceneRadius?: number;
    useTeleportCatalog?: boolean;
    policy?: {
      useTeleports?: boolean;
      distanceBeforeTeleport?: number;
      useShips?: boolean;
      useShortcuts?: boolean;
      allowTeleportIds?: unknown;
      denyTeleportIds?: unknown;
    };
    avoidZones?: unknown[];
    pathFollow?: unknown;
    forceRepath?: boolean;
    log?: (message: string) => void;
  }
  export const Traversal: {
    walkTo(tile: WorldTile, opts?: WalkOptions): Promise<boolean>;
    walkResilient(tile: WorldTile, opts?: WalkOptions): Promise<boolean>;
    preload(): void;
    remaining(): number;
    teleportsEnabled(): boolean;
    requestRepath(_reason: string): void;
    get pureWalk(): { useTeleportCatalog: boolean; policy: { useTeleports: boolean } };
    get withTeles(): { useTeleportCatalog: boolean; policy: { useTeleports: boolean } };
  };
}

declare module '*data/cookLocations.js' {
  export type CookLocation = { name: string; [key: string]: unknown };

  export const CUSTOM_LOCATION: any;
  export const MAX_SURFACE_CHEB: any;
  export const COOK_LOCATIONS: any;
  export function findCookLocation(locs: unknown, name: string): unknown;
  export function buildCookLocations(): unknown;
}

declare module '*data/cowKillerLocations.js' {
  export type CowLocation = { name: string; anchor: { x: number; z: number; level: number }; usesAlKharidToll?: boolean; [key: string]: unknown };

  export const COW_LOCATIONS: any;
  export const COW_LOCATION_OPTIONS: any;
  export const AL_KHARID_BANK: any;
  export function resolveCowLocation(setting: unknown): unknown;
  export function nearestCowLocation(tile: unknown): unknown;
}

declare module '*data/dropdb.js' {
  export const DROP_DB: any;
}

declare module '*data/herbs.js' {
  export type HerbDef = { name?: string; [key: string]: unknown };

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
  export function resolveRockIds(_names: unknown): unknown;
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
  export function isEntNpcId(id: number): boolean;
  export function entNpcOnTile(npcs: unknown, tile: unknown): unknown;
}

declare module '*event/webwalk/DirectNavigator.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export const DirectNavigator: {
    walk(dest: WorldTile): boolean;
    walkTo(dest: WorldTile, radius?: number, timeoutMs?: number): Promise<boolean>;
  };
}

declare module '*event/webwalk/Navigator.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export interface InspectHop {
    kind: string;
    locId: number;
    locName: string;
    action: string;
    option: number;
    from: WorldTile;
    to: WorldTile;
    ticks: number;
  }
  export interface InspectPath {
    ok: boolean;
    reason: string;
    bankPlanned: boolean;
    ticks: number;
    hops: InspectHop[];
    request_id: number;
  }
  export const Navigator: {
    start(): unknown;
    isReady(): boolean;
    findPath(from: unknown, to: unknown, opts?: unknown): Promise<InspectPath | null>;
  };
  export default Navigator;
}

declare module '*event/webwalk/geometry/Reachability.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export type ReachTarget = WorldTile | { tile(): WorldTile | null | undefined };
  export const Reachability: {
    walkable(target: ReachTarget): boolean;
    canReach(target: ReachTarget, opts?: unknown): boolean;
    lineOfSight(from: ReachTarget, to: ReachTarget, size?: number): boolean;
    canStep(from: ReachTarget, to: ReachTarget): boolean;
  };
}

declare module '*event/webwalk/walkOpening.js' {
  export type WalkOpeningOptions = { radius?: number; [key: string]: unknown };

  export function openOp(actions: unknown): unknown;
  export function towardDest(_from: unknown, _here: unknown, _toward: unknown): unknown;
  export function isOpenableObstacle(_name: string, _actions: unknown, _obstacles: unknown): boolean;
  export function walkOpening(dest: unknown, radius?: number, obstacles?: unknown, log?: (msg: string) => void): Promise<boolean>;
}

declare module '*geometry/Tile.js' {
  export interface WorldTile {
    x: number;
    z: number;
    level: number;
  }
  export class Tile implements WorldTile {
    constructor(x: number, z: number, level?: number);
    x: number;
    z: number;
    level: number;
    static from(tile: WorldTile): Tile;
    distanceTo(other: WorldTile): number;
    translate(dx: number, dz: number): Tile;
    equals(other: WorldTile): boolean;
    toString(): string;
  }
  export default Tile;
  export function tileFromPosted(value: unknown): Tile | null;
}

declare module '*input/Input.js' {
  export const Input: {
    walk(lx: number, lz: number): unknown;
    interactNpc(index: number, op: string): unknown;
    interactPlayer(index: number, op: string): unknown;
    interactLoc(lx: number, lz: number, _typecode: unknown, op: string): unknown;
    takeObj(lx: number, lz: number, _objId: unknown, op: string): unknown;
    heldOp(objId: unknown, _slot: number, _comId: number, op: string): unknown;
    invButton(objId: unknown, slot: number, comId: number, op: string): unknown;
  };
}

declare module '*paint/Paint.js' {
  export type PaintLine = string | { text?: string; [key: string]: unknown };
  export interface PaintFrame {
    title(text: string): PaintFrame;
    row(...cols: Array<string | number>): PaintFrame;
    gap(px?: number): PaintFrame;
    text(line: string, _color?: string): PaintFrame;
    bar(label: string, fraction: number, _color?: string): PaintFrame;
    tabs(id: string, names: string[]): string;
    cells(cols: Array<string | { text: string }>): PaintFrame;
    strip(id: string, names: string[], status?: string, brand?: string, resolved?: string): string;
    rail(id: string, names: string[], resolved?: string): string;
    footer(text: string): PaintFrame;
    rowsLeft(): number;
    statGrid(rows: unknown, columns?: unknown): PaintFrame;
    select(id: string, label: string, options: unknown[], current?: unknown): unknown;
    buttons(items: Array<{ id: string; label: string } | null> | null): string | null;
    end(): void;
  }
  export const Paint: {
    begin(ctx: CanvasRenderingContext2D | null, opts?: { dock?: string; accent?: string }): PaintFrame;
  };
}

declare module '*paint/jive.js' {
  export const JIVE_ACCENT: string;
  export const JIVE_BYLINE: string;
  export const COMBAT_SKILLS: string[];
  export interface SkillGain {
    skill: string;
    gained?: number;
    [key: string]: unknown;
  }
  export interface JivePaintFrame {
    title(text: string): JivePaintFrame;
    row(...cols: Array<string | number>): JivePaintFrame;
    gap(px?: number): JivePaintFrame;
    text(line: string): JivePaintFrame;
    bar(label: string, fraction: number, _color?: string): JivePaintFrame;
    tabs(id: string, names: string[]): string;
    cells(cols: Array<string | { text: string }>): JivePaintFrame;
    strip(id: string, names: string[], status?: string, brand?: string, resolved?: string): string;
    rail(id: string, names: string[], resolved?: string): string;
    footer(text: string): JivePaintFrame;
    rowsLeft(): number;
    statGrid(rows: unknown, columns?: unknown): JivePaintFrame;
    select(id: string, label: string, options: unknown[], current?: unknown): unknown;
    buttons(items: Array<{ id: string; label: string } | null> | null): string | null;
    end(): void;
  }
  export interface JiveFrame {
    frame: JivePaintFrame;
    page: string;
    section: string;
  }
  export function scriptFrame(ctx: CanvasRenderingContext2D | null, opts?: unknown): JiveFrame;
  export function jiveFrame(ctx: CanvasRenderingContext2D | null, opts?: unknown): JiveFrame;
  export class XpTracker {
    constructor(skills: string[], read: { xp(skill: string): number; level(skill: string): number });
    skills: string[];
    read: { xp(skill: string): number; level(skill: string): number };
    id: unknown;
    begin(): void;
    progress(): SkillGain[];
    gains(): SkillGain[];
  }
  export function paintLevels(p: { rowsLeft?: () => number; [key: string]: unknown }, gains: SkillGain[], mins: number, reserve: number, empty?: unknown): void;
}

declare module '*paint/levelProgress.js' {
  export interface SkillGain {
    skill: string;
    gained?: number;
    [key: string]: unknown;
  }
  export interface LevelProgress {
    level: number;
    fraction: number;
    remaining: number;
  }
  export function xpAtLevel(level: number): number;
  export function levelProgress(level: number, xp: number): LevelProgress;
  export function etaHours(remaining: number, xpPerHour: number): number | null;
  export function levelRow(g: unknown, mins: number): string;
}

declare module '*paint/paintLogic.js' {
  export function fmtDuration(minutes: number): string;
  export function fmtXpHr(gained: number, mins: number): string;
  export function paintSkillShort(skill: string): string;
}

declare module '*runtime/BotHost.js' {
  export const BotHost: {
    get tickCount(): number;
    addTickListener(cb: () => void): () => boolean;
  };
}

declare module '*runtime/RecoveryHints.js' {
  import Tile from '*geometry/Tile.js';
  export interface WorldTile { x: number; z: number; level: number }
  export const RecoveryHints: {
    get pendingRecovery(): unknown;
    set pendingRecovery(value: number);
    get anchor(): Tile | null;
    set anchor(value: WorldTile | null);
    takeAnchor(): Tile | null;
    clear(): unknown;
  };
}

declare module '*runtime/RunManager.js' {
  export const RunManager: {
    override(policy: unknown): unknown;
  };
}

declare module '*runtime/ScriptRunner.js' {
  export const ScriptRunner: {
    stop(reason?: string): void;
    paintControls(p: { gap(px?: number): unknown; row(...cols: Array<string | number>): unknown }): void;
  };
}

declare module '*runtime/Settings.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export interface SettingDef {
    type?: unknown;
    default?: unknown;
    label?: unknown;
    help?: unknown;
    min?: unknown;
    max?: unknown;
    options?: unknown;
    optionsFrom?: unknown;
    group?: unknown;
    showIf?: unknown;
    [key: string]: unknown;
  }
  export type SettingsSchema = Record<string, SettingDef>;
  export class SettingsBag {
    constructor(values?: Record<string, unknown>);
    values: Record<string, unknown>;
    bool(key: string, fallback?: boolean): boolean;
    num(key: string, fallback?: number): number;
    str(key: string, fallback?: string): string;
    list(key: string, fallback?: string[]): string[];
    tile(key: string, fallback?: WorldTile | null): WorldTile | null;
  }
  export const SettingsStore: {
    resolve(_name: string, schema: SettingsSchema): Record<string, unknown>;
    displayString(_name: string, key: string, def?: SettingDef): string;
    saved(_name: string, key: string): string | undefined;
    globalBag(): SettingsBag;
  };
}

declare module '*runtime/Supervisor.js' {
  export const Supervisor: {
    noteProgress(): unknown;
  };
}

declare module '*shim/_kernel.js' {
  export function host(): unknown;
  export function snap(): unknown;
  export function notImpl(name: string, reason: string): unknown;
  export function queue(req: unknown): unknown;
  export function runMachine(family: string, args: unknown, hooks: unknown): Promise<boolean>;
  export function machineNow(family: string, args: unknown): unknown;
  export function proxy(ns: unknown, members: unknown): unknown;
  export function notImplValue(ns: unknown): unknown;
  export function distanceTo(a: unknown, b: unknown): number;
  export function planarDistanceTo(a: unknown, b: unknown): unknown;
  export function arrived(dest: unknown, radius: number): unknown;
  export function presentOps(actions: unknown): unknown;
  export function opIndex(actions: unknown, action: string): number;
  export function entitySnapView(row: unknown): unknown;
  export function optionalText(value: number): unknown;
}

