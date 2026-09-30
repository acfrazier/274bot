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

// Runtime-wins allowlist (frozen typed source differs; the live shim is authority):
// - Game.tile(): WorldTile | null posted snapshot, not a Tile instance.
// - Game.setCombatStyle / hasCombatStyle: any style string matchCombatRow resolves.
// - Npc.health: posted snap.health may be null.
// - Entity .interact: shim is synchronous boolean (not frozen Promise).
// - Entity .snap / constructors: posted rows, not frozen snapshot classes.
// - LoopingBot.settings: prelude SettingsView, not frozen SettingsBag.
// - LoopingBot.recoveryAnchor: posted WorldTile, not Tile.
// - GameMessages.since: ChatLine {seq,text} (frozen GameMessage is the same row).
// - Tools.ToolReq / AXES / PICKAXES: simplified {name,id} runtime records.
// - Bank async, Game.combatStyleMode, Game.castOnNpc, Shop.buyById: known Rust divergences.
// - Banking.bankNearest(opts?): shim defaults opts to {}.
// - rockCrabRangeLoadout(weapon?, ammo?): shim is rest `...args`.
// - stealCakes(opts?): shim defaults opts to {}.
// - Quests.journal(name?): shim journal() is notImpl with zero params.
// - ClueDuelHandshake constructor includes initiator (shim + frozen); previously omitted.
// - scriptFood/selectedLoadout/rangedItem take a `.str()` bag so LoopingBot.settings (SettingsView) and frozen SettingsBag both typecheck.
// - EatTimingInput.attackedThisTick is a boolean field (shim compares === true), not a method.
// - Reach.locOp is absent from the shim (known Rust divergence); leftover catalog diagnostic is unchanged.
// - CLUE_DB / CASKET_IDS are empty objects in the live shim.
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
  export interface ToolTier {
      name: string;
      /** Skill level required to *use* the tool (mining / woodcutting). */
      level: number;
      // Mining level for using the tool from inventory; wielding has a separate Attack requirement.
  
      /** Attack level required to wield the tool. */
      attackLevel?: number;
  }
  export interface ToolRestockStep {
      name: string;
      qty: number;
      equip: boolean;
  }
  export type ToolReq = { name?: string; [key: string]: unknown };

  export const TINDERBOX: "Tinderbox";
  export const HAMMER: "Hammer";
  export const KNIFE: "Knife";
  export const CHISEL: "Chisel";
  export const NEEDLE: "Needle";
  export const AXES: any;
  export const PICKAXES: any;
  export function exactTool(name: string): unknown;
  export function tinderboxReq(): unknown;
  export function axeReq(): unknown;
  export function pickaxeReq(): unknown;
  export function toolKeepNames(reqs: readonly ToolReq[]): string[];
  export function hasToolReq(req: unknown, skillLevel: unknown, count: number | ((name: string) => number)): boolean;
  export function hasAllTools(reqs: unknown, skillLevel: unknown, count: number | ((name: string) => number)): boolean;
  export function bestAxe(woodcuttingLevel: number, available: (name: string) => boolean): string | null;
  export function bestPickaxe(miningLevel: number, available: (name: string) => boolean): string | null;
  export function bestFromTiers(level: number, tiers: readonly ToolTier[], available: (name: string) => boolean): string | null;
  export function canWieldTool(name: string, attackLevel: number): boolean;
  export function toolRestockPlan(reqs: readonly ToolReq[], skillLevel: (skill: string) => number, invCount: (name: string) => number, bankCount: (name: string) => number): ToolRestockStep[];
  export function missingToolLabels(): unknown;
  export function toolKitLabel(): unknown;
  export function bankHasBetterGatherTool(): unknown;
}

declare module '*api/ai/clues/ClueExecutor.js' {
  export interface NavPoint { x: number; z: number; level?: number }
  export interface ClueProgress {
      clueId: number;
      name: string;
      step: string;
      leg: number;
      attempt: number;
      startedAt: number;
      /** Where this leg is headed, a dig/search coord, or a talk NPC's anchor. */
      target: NavPoint | null;
      /** Distance to `target` when the leg began, so travel can be shown as a bar. */
      startDist: number;
  }
  export type GuardianStop = 'supplies-needed' | 'dead' | 'guardian-lost';
  export type ClueOutcome = 'done' | 'abandon' | 'yield' | GuardianStop;
  export class ClueExecutor {
    static setTeleports(enabled: unknown): unknown;
  }
}

declare module '*api/ai/clues/SolveClue.js' {
  export interface NavPoint { x: number; z: number; level?: number }
  export interface SolveClueHost {
      log(m: string): void;
      setStatus(s: string): void;
      isFood(name: string): boolean;
      foodName(): string;
      foodWithdraw(): number;
      weaponName?(): string;
      enabled?(): boolean;
      /** Travel to the initial bank with host upkeep intact; false blocks the trail. */
      prepareInitialBank?(): Promise<boolean>;
      /** Hard-clue dig guardians are fought under Protect from Magic. */
      restorePrayer?(): boolean;
      /** Route trail legs through the teleport catalog and stock the runes. */
      useTeleports?(): boolean;
  }
  export class SolveClue {
    constructor(host: SolveClueHost);
    host: unknown;
    token: unknown;
    status: unknown;
    clueStatus(): string;
    noteDeath(): void;
    ownsEquipment(): boolean;
    retry(): void;
    enabled(): unknown;
    hooks(): unknown;
    validate(): boolean;
    execute(): Promise<void>;
  }
  export function heldClueLikeId(): number | null;
  export function walkToBank(tile: NavPoint, log: (m: string) => void): Promise<boolean>;
}

declare module '*api/ai/clues/bankAccess.js' {
  export function openClueBank(log?: (message: string) => void): Promise<boolean>;
}

declare module '*api/ai/clues/cluePaint.js' {
  export type PaintFrame = { row(...cols: string[]): PaintFrame; end(): void };
  export function paintClueProgress(p: PaintFrame, idle?: string): void;
}

declare module '*api/ai/clues/data/cluedb.js' {
  export const CLUE_DB: {
  };
  export const CASKET_IDS: {
  };
}

declare module '*api/ai/clues/data/toolAcquire.js' {
  import Tile from '*geometry/Tile.js';

  export type CoordTool = 'sextant' | 'watch' | 'chart';
  export interface HeldTrio {
      sextant: boolean;
      watch: boolean;
      chart: boolean;
  }
  export interface ShopSource {
      npc: string;
      stand: Tile;
      /** Rough coins per unit, so a caller can say why it could not buy. */
      cost: number;
  }
  export const SPADE_NAME: "Spade";
  export const TRIO: readonly [ "Sextant", "Watch", "Chart" ];
}

declare module '*api/ai/clues/duelTravel.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export const DUEL_CLUE_ID: 3554;
  export const DUEL_CLUE_TILE: any;
  export function crossesClueDuel(dest: WorldTile): boolean;
  export function walkAcrossClueDuel(dest: WorldTile, radius: number, log: (message: string) => void): Promise<boolean>;
}

declare module '*api/ai/quests/defs/murder/areas.js' {
  import Tile from '*geometry/Tile.js';

  export interface LocStop {
      id: number;
      name: string;
      near: Tile;
  }
  export interface Suspect {
      stop: import('*api/ai/quests/exec/primitives.js').NpcStop;
      barrel: LocStop;
      /** The silver keepsake in their barrel, its floured form, and the print lifted off it. */
      silver: number;
      dust: number;
      print: number;
      /** The thread colour their clothes are made of. */
      thread: number;
      /** What they claim they bought the poison for. */
      poison: LocStop;
  }
  export interface WorldTile { x: number; z: number; level: number }
  export const MURDER_NAME: "Murder Mystery";
  export const MURDER_OBJ: {
    readonly POT: 1931;
    readonly POT_FLOUR: 1933;
    readonly FLYPAPER: 1811;
    readonly DAGGER: 1813;
    readonly DAGGER_DUST: 1814;
    readonly UNKNOWN_PRINT: 1822;
    readonly KILLERS_PRINT: 1815;
    readonly THREAD_RED: 1808;
    readonly THREAD_GREEN: 1809;
    readonly THREAD_BLUE: 1810;
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
  /** What the engine needs from whatever script is driving it. */
  export interface QuestHost {
      log(msg: string): void;
      verbose(): boolean;
      foodItem(): string | null;
      pickedIds(): Set<string>;
      skipPending(): boolean;
      consumeSkip(): boolean;
      consumeDeath(): boolean;
      noteState(rows: unknown[], runningId: string | null, stepDesc: string, noProgress: number, parked: number): void;
      /** Why: the engine only reports it's out of work; stopping the run is the script's call. */
      finish(reason: string): void;
  }
  export const QuestEngine: any;
}

declare module '*api/ai/quests/exec/primitives.js' {
  import Tile from '*geometry/Tile.js';

  export interface LineRule {
      /** Lower-case fragment of the NPC's spoken line. */
      whenLine: string;
      /** Option to choose when that fragment is present. */
      choose: string;
  }
  export interface LadderHop {
      stand: Tile;
      locName: string;
      op: string;
      arrive: Tile;
      open?: string;
      /** Long-walk dest when `stand` is behind a door the baked graph can't pin. */
      walk?: Tile;
  }
  export interface NpcStop {
      npc: string;
      anchor: Tile;
      leash: number;
      prefer: string[];
      approach?: Tile[];
      // Why: a scene that walks an npc somewhere and animates it goes quiet for longer than a page turn, and the default tolerance ends the drive mid-scene.
      /** How long a lull may last before the conversation counts as over. */
      gapMs?: number;
  }
  export function pickPreferred(options: string[], prefer: string[]): string | null;
  export function pickByLine(): unknown;
  export function isUnderground(t: { z: number; }): boolean;
  export function needsHop(): unknown;
  export function walkWithHops(dest: Tile, radius: number, hops: LadderHop[], log: (m: string) => void): Promise<boolean>;
  export function gotoNpc(stop: unknown): Promise<boolean>;
  export function driveDialog(prefer: string[], log: (m: string) => void, gapMs?: number): Promise<boolean>;
  export function openDialogue(npcName: string, log: (m: string) => void): Promise<boolean>;
  export function talkThrough(npcName: string, prefer: string[], log: (m: string) => void, gapMs?: number): Promise<boolean>;
  export function talkStrict(npcName: string, prefer: string[], log: (m: string) => void): Promise<boolean>;
  export function talkChoosingBy(): Promise<boolean>;
  export function talkOp(actions: string[]): string | null;
}

declare module '*api/bank/Bank.js' {
  export type InvItemSnapshot = { slot: number; id: number; name: string | null; count: number };
  export type BackpackItem = Pick<InvItemSnapshot, 'slot' | 'id' | 'name' | 'count'>;

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
  export function withdrawOp(ops: readonly (string | null)[], amount: "all" | "10" | "5" | "1" | "x" | "any"): string | null;
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
  export interface BankRequirement {
      skill?: { name: string; level: number };
      quest?: string;
      // Why: Gundai's cellar is reached by slashing into ~level 55 Wilderness, so an Ardougne script must never pick it up for being a few tiles closer.
      // Why: off by default, so a Wilderness bot opts in.
  
      /** A Global setting that must be true before this bank is offered at all. */
      setting?: string;
  }
  export interface BankObjectAccess {
      name: string;
      op: string;
      openFirst?: {
          name: string;
          op: string;
      };
  }
  export interface BankNpcAccess {
      name: string;
      op: string;
      /** The dialogue option that opens the bank, when the banker surfaces one. */
      choose?: string;
  }
  /** Walk cost in run-tiles from `from` to a bank, or null when the bank is unreachable. */
  export type BankPathCost = (from: WorldTile, to: Tile) => number | null;
  /** Async walk cost: the nav pack lives in the worker, so a live caller pays per {@link Navigator.findPath} instead of a local {@link PathFinder}. */
  export type BankPathCostAsync = (from: WorldTile, to: Tile) => Promise<number | null>;
  export interface BankPathStamp {
      x: number;
      z: number;
      level: number;
  }
  /** The bits of the {@link Navigator} {@link findPath} that the walk-cost wrapper needs. */
  export type NavigatorLike = {
      findPath(
          from: BankPathStamp,
          to: BankPathStamp,
          opts?: unknown
      ): Promise<{
          ok: boolean;
          cost?: number;
          reason?: string;
      }>;
  };
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
  /**
 * A bank, its stand tile, and how to open it.
 * @see docs/reference/api-items.md#bank
 */
export interface BankLocation {
    name: string;
    tile: Tile;
    requires?: BankRequirement;
    access?: BankObjectAccess;
    npcAccess?: BankNpcAccess;
    // Why: callers still walk to `tile`, since the nav graph knows how to get there.
    // Why: ranking is straight-line, and Gundai's cellar at z=4714 scores ~800 tiles from the ladder it's reached by, so it would never be chosen.

    /** Where the surface route to this bank starts, for distance ranking only. */
    approach?: Tile;
}
  export const USE_MAGE_BANK: "useMageBank";
  export const USE_ZANARIS_BANK: "useZanarisBank";
  export const BANK_LOCATIONS: BankLocation[];
  export function approachOf(bank: BankLocation): Tile;
  export function bankUnlocked(bank: BankLocation): boolean;
  export function nearestBank(from: WorldTile): BankLocation | null;
  export function nearestBanks(from: WorldTile): BankLocation[];
  export function nearestUsableBank(from: WorldTile, usable: (bank: BankLocation) => boolean): BankLocation | null;
  export function nearestBankReachable(here: WorldTile, navigator: NavigatorLike): Promise<BankLocation | null>;
}

declare module '*api/bank/Banking.js' {
  import type { BankObjectAccess, BankNpcAccess } from '*api/bank/BankLocations.js';
  /** Re-export as a type alias so the drift parser does not treat it as a value export. */
  export type BankStrategy = import('*api/bank/bankRules.js').BankStrategy;
  export type BankTriggerState = import('*api/bank/bankRules.js').BankTriggerState;
  export interface OpenBankOpts {
      /**
       * Preset bank stand (location table). Used when no bank is already nearby.
       * Nearby booth / local nearestBank always wins when {@link preferNearby} is on.
       */
      stand?: WorldTile | null;
      boothName?: string;
      boothOp?: string;
      /**
       * Openable obstacles on the way to a preset stand (doors/gates).
       * Empty = plain walkResilient to the stand.
       */
      obstacles?: string[];
      /** Optional forced destination when no booth is in scene and stand is unset. */
      destination?: BankDestination;
      /**
       * Prefer a bank already underfoot or in the local scene over a distant preset stand; default true.
       * Why: starting next to Draynor must not web-walk to Edgeville because the camp table says Edgeville.
       */
      preferNearby?: boolean;
      /** Chebyshev / booth distance for nearby snap. Default {@link NEARBY_BANK_RADIUS}. */
      nearbyRadius?: number;
      log?: (msg: string) => void;
  }
  export type BankOpenRoute = 'already-open' | 'scene-booth' | 'local-bank' | 'preset-stand' | 'nearest-fallback';
  export interface WorldTile { x: number; z: number; level: number }
  export interface BankDestination {
    name: string;
    tile: WorldTile;
    access?: BankObjectAccess;
    /** Set when the banker is an npc (Gundai). */
    npcAccess?: BankNpcAccess;
}
  export const COMMON_BANK_LOOT: string[];
  export const RANDOM_EVENT_CASKET_ID: 405;
  export function matchesCommonBankLoot(name: string, id?: number): boolean;
  export function depositMatcher(own: (name: string) => boolean, includeCommon: boolean): (name: string, id?: number | undefined) => boolean;
  export function depositAllExcept(keep: Iterable<string> | null | undefined): (name: string) => boolean;
  export const PERIODIC_BANK_SETTINGS: {
    bankStrategy: unknown;
    bankEveryItems: unknown;
    bankEveryMinutes: unknown;
    bankCommonJunk: unknown;
  };
  export function parseBankStrategy(label: string): BankStrategy;
  export const Banking: {
    open(opts?: OpenBankOpts): Promise<boolean>;
    // Runtime wins: shim `bankNearest(opts = {})` — opts is optional even though frozen requires it.
    bankNearest(opts?: { deposit?: (name: string) => boolean; commonJunk?: boolean; destination?: BankDestination; returnTo?: WorldTile; boothName?: string; boothOp?: string; afterDeposit?: () => void | Promise<void>; log?: (msg: string) => void; }): Promise<boolean>;
  };
}

declare module '*api/bank/bankOps.js' {
  export function withdrawOp(ops: readonly (string | null)[], amount: 'all' | '10' | '5' | '1' | 'x' | 'any'): string | null;
}

declare module '*api/bank/bankQuestJunk.js' {
  export interface QuestJunkEntry {
      id: number;
      name: string;
      quest: string;
  }
  export interface QuestJunkFinding {
    id: number;
    name: string;
    quest: string;
    status: import('*api/ui/questlog/Quests.js').QuestStatus;
    droppable: boolean;
}

  export const QUEST_JUNK: readonly QuestJunkEntry[];
  export function findQuestJunk(): unknown;
}

declare module '*api/bank/bankRules.js' {
  /**
   * When a bot should break off and bank.
   * @see docs/reference/api-items.md#bank
   */
  export type BankStrategy = 'off' | 'items' | 'time' | 'either';
  export interface BankTriggerState {
      lootCount: number;
      minutesSinceLastBank: number;
      itemsThreshold: number;
      minutesThreshold: number;
  }
  export function shouldBankNow(): unknown;
  export function isDisposableGatherJunk(): boolean;
  export function parseBankStrategy(label: string): BankStrategy;
  export const PERIODIC_BANK_SETTINGS: {
    bankStrategy: unknown;
    bankEveryItems: unknown;
    bankEveryMinutes: unknown;
    bankCommonJunk: unknown;
  };
  export const COMMON_BANK_LOOT: string[];
  export function matchesCommonBankLoot(name: string, id?: number): boolean;
  export function depositMatcher(own: (name: string) => boolean, includeCommon: boolean): (name: string, id?: number) => boolean;
  export function depositAllExcept(keep: Iterable<string>): (name: string) => boolean;
}

declare module '*api/bank/bankSort.js' {
  export interface SortBankOptions {
      log?: (msg: string) => void;
      categoryOverrides?: ReadonlyMap<number, BankCategory>;
  }
  export type BankSortMode = 'insert' | 'swap' | string;
  export interface BankSortResult {
    sorted: boolean;
    moves: number;
    mode: BankSortMode | null;
    unmatched: number[];
    reason: string;
}
  export type BankCategory = string;

  export const ARRANGE_SWAP_COM: 8130;
  export const ARRANGE_INSERT_COM: 8131;
  export const BANK_INSERT_VARP: 304;
  export function sortBank(opts?: unknown): Promise<BankSortResult>;
}

declare module '*api/bank/bankSortRules.js' {
  export type BankCategory =
      | 'coins' | 'runes' | 'staves' | 'ammunition' | 'weapons' | 'armour' | 'rangedArmour'
      | 'food' | 'potions' | 'jewellery' | 'herbs' | 'oresBarsGems' | 'logs' | 'supplies'
      | 'tools' | 'teleports' | 'questLive' | 'questObsolete' | 'junk';
  export interface SortableItem {
      slot: number;
      id: number;
      name: string | null;
      cost: number;
  }
  export interface CategoryRule {
      category: BankCategory;
      ids?: readonly number[];
      match?: (name: string) => boolean;
  }
  export const CATEGORY_ORDER: readonly BankCategory[];
  export function categoryOf(): unknown;
  export function isUnmatched(): boolean;
}

declare module '*api/bot/Bot.js' {
  export interface BranchTask { validate(): boolean; success(): unknown; failure(): unknown }
  export interface LeafTask { execute(): unknown }
  export type TreeNode = BranchTask | LeafTask;
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
    loop(): Promise<number | void>;
    recoveryAnchor(): WorldTile | null;
    grindTargets(): string[];
    ignoredRandoms(): string[];
    on(event: 'chat.message', cb: (payload: ChatMessage) => unknown): void;
    on(event: string, cb: (payload: unknown) => unknown): void;
    log(msg: string): void;
    get settings(): SettingsView;
  }
  export class TaskBot extends LoopingBot {
    constructor();
    add(...tasks: Task[]): void;
    loop(): Promise<void | number>;
  }
  export class TreeBot extends LoopingBot {
    root(): TreeNode;
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
  export interface GameMessage {
      seq: number;
      text: string;
  }
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
  export interface CombatModeLabel {
      mode: number;
      label: string;
  }
  export interface CombatStyleBackend {
      offeredModes(): readonly CombatModeLabel[] | null;
      currentMode(): number;
      selectMode(mode: number): boolean;
  }
  export type CombatKind = 'melee' | 'mage' | 'range';
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
  export function resolveSplitCombatSettings(rawCombatStyle: string, rawMeleeStyle?: string): { kind: CombatKind; meleeStyle: MeleeCombatStyle; legacyMigrated: MeleeCombatStyle | null; };
  export function parseRangeStyle(name: string): number;
  export function describeCombatStyle(resolution: CombatStyleResolution): string;
}

declare module '*api/combat/CombatStyleLogic.js' {
  export const ATTACKSTYLE_MAGIC_VARP: 108;
  export const AUTOCAST_ARMED: 3;
  export function runesPerCast(spellName: string, wielded: string[]): { rune: string; count: number; }[] | null;
  export function spellButtonCom(spellName: string): number;
  export function castsAvailable(spellName: string, wielded: string[], held: (rune: string) => number): number;
  export function runeWithdrawList(spellName: string, wielded: string[], casts: number): { rune: string; count: number; }[];
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
  export const SA_MAX_ENERGY: 1000;
  export const Special: {
    energy(): number;
    armed(): boolean;
    wielded(): string;
    cost(weaponName: string): number | null;
    ready(weaponName: string): boolean;
    barComponent(): number;
    arm(): Promise<boolean>;
  };
}

declare module '*api/combat/boostPotions.js' {
  export type CarryEntry = { name: string; count?: number };
  export interface BoostPotion {
      /** The skill the dose lifts. */
      skill: string;
      /** Paint label, kept to 3 characters so a boost row still fits 3 columns. */
      short: string;
      /** The dose form drawn when the loadout names none. */
      flask: string;
      /** Every dose form, so a part-used flask still counts as one in the pack. */
      doses: readonly string[];
  }
  export interface SipState {
      plans: readonly PotionPlan[];
      /** Flasks of that potion held, counting every dose form. */
      held: (plan: PotionPlan) => number;
      levels: (skill: string) => { base: number; effective: number };
  }
  export interface PotionPlan {
    potion: BoostPotion;
    /** The dose form the bank run draws. */
    flask: string;
    /** Flasks to carry per trip. */
    want: number;
}

  export const BOOST_POTIONS: readonly BoostPotion[];
  export const SUPER_ATTACK: BoostPotion;
  export const SUPER_STRENGTH: BoostPotion;
  export const EMPTY_VIAL: "Vial";
  export const BOOST_FLOOR: 0.1;
  export function boostFaded(base: number, effective: number, floor?: number): boolean;
  export function plannedPotions(carry: readonly CarryEntry[]): PotionPlan[];
  export function potionToSip(s: SipState): PotionPlan | null;
}

declare module '*api/combat/eatTiming.js' {
  export interface EatTimingInput {
      /** True only on the tick the attack animation began. */
      attackedThisTick: boolean;
      /** 0 to 1. */
      hpFraction: number;
      /** At or below this fraction, eat now whatever the tick. */
      urgentAt: number;
  }
  export const URGENT_HP_FRACTION: 0.35;
  export function shouldHoldEat(input: EatTimingInput): boolean;
  export class AttackClock {
    observe(anim: number, tick: number): void;
    attackedThisTick(tick: number): boolean;
    reset(): void;
  }
}

declare module '*api/combat/equipment.js' {
  export const BOWS: string[];
  export const CROSSBOWS: string[];
  export const DARTS: string[];
  export const ARROWS: string[];
  export const BOLTS: string[];
  export const MELEE_WEAPONS: string[];
  export const STAFFS: string[];
}

declare module '*api/combat/fightUpkeep.js' {
  export function swingStartedThisTick(): boolean;
  export function buryOneInFight(boneName: string): Promise<boolean>;
}

declare module '*api/combat/food.js' {
  export const FOOD_OPTIONS: string[];
  export const MIN_EAT_HP: 5;
  export function foodHealAmount(foodName: string): number;
  export function foodForms(foodName: string): string[];
  export function isFoodItem(name: string | null | undefined, foodName: string): boolean;
  export function foodCount(items: readonly { name: string | null | undefined; }[], foodName: string): number;
  export function eatAtHpThreshold(maxHp: number, heal: number, minHp?: number): number;
  export function shouldEatToUseFood(opts: { hp: number; maxHp: number; heal: number; foodCount: number; minHp?: number; }): boolean;
  export function shouldEatFood(foodName: string, opts: { hp: number; maxHp: number; foodCount: number; minHp?: number; }): boolean;
}

declare module '*api/combat/hunting/combat.js' {
  import Tile from '*geometry/Tile.js';
  import type { Style } from '*api/combat/hunting/logic.js';
  import type { DragonSite } from '*api/combat/hunting/sites.js';
  import type { JiveHost } from '*api/combat/hunting/supply.js';
  /** Whether the ladder ran, and whether the bot is back on a tile it can fight from. */
  export type Step = 'held' | 'moved' | 'stuck';
  export interface Engagement {
      isOurs: boolean;
      inCombat: boolean;
      targetsMe: boolean;
      targetsAnother: boolean;
  }
  export interface CombatHost extends JiveHost {
    readonly fight?: Fight;
    died: boolean;
    targetIdx: number | null;
    countKill(): void;
    countBurial(): void;
    hpFraction(): number;
    panicHp(): number;
    retreatHp(): number;
    hasFood(): boolean;
    needEat(): boolean;
    eatOnce(): Promise<boolean>;
    shieldReady?(): boolean;
    buryBones(): boolean;
    boneName(): string;
    safespotIndex(): number;
    setSafespotIndex(n: number): void;
    armSpecial?(): Promise<void>;
  }

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
  export function anchorFor(site: DragonSite, style: Style, index: number): Tile;
  export class Fight extends HuntTask {
    constructor(host: unknown, site: unknown);
    execute(): Promise<void>;
    reset(): void;
    interruptWatch(): void;
    blocksLoot(): boolean;
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
  import type { Spot } from '*api/combat/hunting/logic.js';
  export interface Body {
      tile: Spot;
      size: number;
  }
  export const LOOT_GUARD: 4;
  export function guarded(drop: Spot, bodies: readonly Body[], radius?: number): boolean;
}

declare module '*api/combat/hunting/logic.js' {
  import Tile from '*geometry/Tile.js';
  import type { SettingsBag, SettingsSchema } from '*runtime/Settings.js';
  export interface LadderState {
      index: number;
      spots: number;
      /** HP fell while standing on the current safespot. */
      hurt: boolean;
      /** How long no adult has been in line of sight. */
      blindMs: number;
  }
  export interface HurtState {
      rangedThreat: boolean;
      onSpot: boolean;
      /** The hp read on the previous pass, or -1 when the bot was off the tile. */
      lastHp: number;
      hp: number;
  }
  export interface RetreatState {
      inLair: boolean;
      /** Standing on one of the site's safespots right now. */
      onSafespot: boolean;
      hpFrac: number;
      retreatHp: number;
      hasFood: boolean;
      spots: number;
  }
  export interface LootState {
      hpFrac: number;
      panicHp: number;
      retreatHp: number;
  }
  export interface HoldState {
      onSafespot: boolean;
      hasFood: boolean;
  }
  export interface Spot {
      x: number;
      z: number;
  }
  export interface Sighting {
      x: number;
      z: number;
      /** When the body was first seen on this tile. */
      since: number;
      at: number;
  }
  export interface RetreatAim {
      /** The index a failed attempt rotated to, or null when this retreat is a fresh one. */
      rotated: number | null;
      from: Spot;
      spots: readonly Spot[];
  }
  export interface AntifireState {
      inLair: boolean;
      tick: number;
      /** The tick the last dose lapses, 0 when none has been drunk. */
      until: number;
  }
  export interface DropFilter {
      loot: ReadonlySet<string>;
      bankCommon: boolean;
      solveClues: boolean;
      buryBones: boolean;
      boneName: string;
  }
  export type Style = 'melee' | 'mage' | 'range';

  export const SAFESPOT_BLIND_MS: 20000;
  export const PROTECT_FROM_MELEE: "Protect from Melee";
  export const PRAYER_SIP_FLOOR: 8;
  export const PRAYER_SIP_FRACTION: 0.15;
  export const LOOT_REACH: 10;
  export const LOOT_REACH_OPEN: 14;
  export const SHIELD_ABSORBS: RegExp;
  export const POTION_PROTECTS: RegExp;
  export const ANTIFIRE_TICKS: 600;
  export const ANTIFIRE_MARGIN_TICKS: 20;
  export function nextSafespot(s: LadderState): number;
  export function hurtOnSpot(s: HurtState): boolean;
  export function retreatDue(s: RetreatState): boolean;
  export function lootHalts(s: LootState): boolean;
  export function holdDue(s: HoldState): boolean;
  export function nearestSpot(from: Spot, spots: readonly Spot[]): number;
  export function bodyOrigin(tile: Spot, size: number): Spot;
  export function noteSighting(prev: Sighting | undefined, tile: Spot, now: number): Sighting;
  export function settled(s: Sighting | undefined, now: number, ms: number): boolean;
  export function retreatAim(a: RetreatAim): { index: number; next: number; };
  export function chaseMode(style: Style, fireAtRange: boolean): boolean;
  export function prayerFor(style: Style, fireAtRange: boolean): string | null;
  export function prayerSipDue(points: number, max: number): boolean;
  export function lootReach(fireAtRange: boolean): number;
  export function attackRangeFor(style: Style): number;
  export function engageRangeFor(style: Style): number;
  export function gapTo(from: Spot, tile: Spot, size: number): number;
  export function shieldGate(style: Style, fireAtRange: boolean, hasShield: boolean): string | null;
  export function styleGate(style: Style, fireAtRange: boolean): string | null;
  export function antifireDue(s: AntifireState): boolean;
  export function antifireLapsed(sawShield: boolean, sawPotion: boolean): boolean;
  export function nextApproachIndex(stops: readonly Spot[], here: Spot): number;
  export function isClueObj(id: number): boolean;
  export function keyStatus(held: number, banked: number): 'held' | 'bank' | 'fetch';
  export function wantsDrop(item: { id: number; name: string | null; }, f: DropFilter): boolean;
  export function siteTileOf(schema: SettingsSchema, bag: SettingsBag, key: string | undefined, site: Tile): Tile;
  export function keepDoses(potionDoses: readonly string[], antipoisonDoses: readonly string[], carriesAntipoison: boolean): string[];
}

declare module '*api/combat/hunting/sites.js' {
  import Tile from '*geometry/Tile.js';

  export interface AreaPoint {
      x: number;
      z: number;
      level: number;
  }
  export interface DragonGate {
      locId: number;
      op: string;
      /** Where the key is used from. */
      outside: Tile;
      /** Where the gate lands you. */
      inside: Tile;
  }
  /** A way in that is a conversation rather than a door. */
  export interface DragonTalkGate {
      npc: string;
      op: string;
      /** The option that gets you past; the reply teleports you inside. */
      choose: string;
      stand: Tile;
  }
  /** A way out that is a loc op rather than the walk back in reverse. */
  export interface DragonExit {
      locId: number;
      op: string;
      stand: Tile;
  }
  /** A way in that costs coins: an npc paid from `stand`, then a loc op that teleports inside. */
  export interface DragonFeeGate {
      npc: string;
      op: string;
      coins: number;
      stand: Tile;
      entrance: { locId: number; op: string };
      /** The line the payment prints, which is what proves the coins landed. */
      paidLine: RegExp;
      /** The line a second Pay gets when the varbit is already set, since the coins stay put then. */
      prepaidLine: RegExp;
  }
  /** One numbered place to fight from: the tiles the ladder rotates between and the tile melee uses. */
  export interface DragonStand {
      /** Which dragon it looks at, for the log line. */
      label: string;
      tiles: Tile[];
      anchor: Tile;
  }
  export interface Box {
      minX: number;
      maxX: number;
      minZ: number;
      maxZ: number;
      level: number;
  }
  export interface DragonSite {
    key: string;
    label: string;
    target: string;
    bones: string;
    keyItem: { name: string; id: number } | null;
    gate: DragonGate | null;
    approach: Tile[];
    safespots: Tile[];
    meleeAnchor: Tile;
    bank: Tile;
    /** A teleportId from webwalk/teleportCatalog.ts, never a copied rune list. */
    escapeTeleportId: string;
    /** Walk-out target when the teleport will not fire. */
    walkOut: Tile;
    /** The target hits the safespot from range, so hp lost there is not the derivation being wrong. */
    rangedThreat?: boolean;
    // Why: a SettingDef's options are a fixed string[] with no hook onto another key's value, so each site names the loot setting whose chips are its own drop table.
    /** Settings key holding this site's loot chips; `loot` when absent. */
    lootSetting?: string;
    /** Fallback food when the loadout names none. */
    food?: string;
    /** The route in is worth a Superantipoison. */
    antipoison?: boolean;
    /** The way in is a conversation rather than a door. */
    talkGate?: DragonTalkGate;
    /** The way out is a loc op rather than the walk back. */
    exit?: DragonExit;
    /** Numbered stands to choose between; `safespots` and `meleeAnchor` are the first of them. */
    stands?: DragonStand[];
    // Why: a stand is idle while its dragon respawns, and the Enclave puts a greater demon inside cast range of one, so the idle time goes on that rather than on nothing.
    /** Other npcs worth killing from the same stand, taken only when the target is not up. */
    alsoHunt?: string[];
    /** The way in costs coins at an npc before a loc op teleports inside. */
    feeGate?: DragonFeeGate;
    // Why: metal dragons breathe from ten tiles by script, so no tile is fire-proof and the Dragonfire shield goes on whatever the style; the bow has no hand left for it.
    /** The target breathes at range, so every style wears the shield and range is refused. */
    fireAtRange?: boolean;
    /** The trip carries Antifire potions and sips one whenever the last dose lapses. */
    antifire?: boolean;
    /** Coins carried per trip, for the fee and the fares on the way. */
    coins?: number;
    /** The walk in chops vines, so an axe rides in the pack. */
    axe?: boolean;
    // Why: with the breath at 0 the food goes unused and the doses are what the trip burns, so a site can say how much food a trip carries when the panel is left at its default.
    /** Food per trip while the panel's foodWithdraw sits on its schema default. */
    foodPerTrip?: number;
    inArea(t: AreaPoint | null): boolean;
}

  export function inBox(b: Box): (t: AreaPoint | null) => boolean;
  export const DRAGON_SITES: Record<string, DragonSite>;
  export const TAVERLEY_BLUE: DragonSite;
  export const TAVERLEY_BLACK: DragonSite;
  export const HEROES_BLUE: DragonSite;
  export const GUTANOTH_BLUE: DragonSite;
  export const BRIMHAVEN_IRON: DragonSite;
  export const BRIMHAVEN_STEEL: DragonSite;
  export const STAND_SITE_KEYS: string[];
  export const MAX_STANDS: number;
  export const SITE_OPTIONS: string[];
  export function needsShield(site: DragonSite, style: string): boolean;
  export function huntNames(site: DragonSite): string[];
  export function standFor(site: DragonSite, n: number): DragonStand;
  export function siteFor(key: string): DragonSite;
}

declare module '*api/combat/hunting/supply.js' {
  import type { DragonSite } from '*api/combat/hunting/sites.js';
  import type { Style } from '*api/combat/hunting/logic.js';
  import type { PotionPlan } from '*api/combat/boostPotions.js';

  /** What supply and combat need from the bot, so neither imports JiveDragons.ts. */
  export interface JiveHost {
      log(m: string): void;
      /** Suppressed unless the panel asks for it; a host without one logs nothing extra. */
      vlog?(m: string): void;
      setStatus(s: string): void;
      parkFor(reason: string): void;
      countBankTrip(): void;
      style(): Style;
      foodName(): string;
      foodWithdraw(): number;
      weaponName(): string;
      /** Settle which melee weapon the trip carries, given everything the bank, the pack and the body hold. */
      pickWeapon?(available: readonly string[]): void;
      ammoName(): string;
      spellName(): string;
      keepExtra(): string[];
      /** Walk out through the gate rather than casting the escape teleport. */
      leaveByWalk(): boolean;
  }
  export interface FlaskPlan {
      /** The dose form the bank run draws. */
      flask: string;
      /** Every dose form, so a part-used flask still counts as one in the pack. */
      doses: readonly string[];
      /** Flasks to carry per trip. */
      want: number;
  }
  export interface EscapeSpell {
      runes: { rune: string; count: number }[];
      level: number;
      label: string;
  }
  export type KeyState = 'held' | 'bank' | 'fetch';
  export interface BankOpts {
    withdrawFood: boolean;
    /** Casts of spell runes to withdraw. */
    runeCasts?: number;
    /** Spare runes per type, on top of the cast budget. */
    runeBuffer?: number;
    ammo?: number;
    /** Escape casts carried on top of the one needed to leave. */
    escapeStock?: number;
    /** Fraction of max hp to eat back to before walking in. */
    healTo?: number;
    /** Boost flasks to top up, empty for a run that carries none. */
    potions?: PotionPlan[];
    /** Dose families topped up by count, antipoison and the like. */
    flasks?: FlaskPlan[];
    /** Gear worn every trip on top of the weapon and the ammo. */
    wear?: string[];
    /** Items carried in the pack every trip, an axe for the vines. */
    carry?: string[];
    /** How the trip leaves the lair; leaveLair when absent. */
    leave?: (h: JiveHost, site: DragonSite) => Promise<boolean>;
}

  export const SHIELD: "Dragonfire shield";
  export const POISONED: RegExp;
  export const COINS: "Coins";
  export const ANTIPOISON_LABEL: "Superantipoison";
  export const ANTIPOISON_DOSES: readonly string[];
  export const PRAYER_LABEL: "Prayer potion";
  export const PRAYER_DOSES: readonly string[];
  export const ANTIFIRE_LABEL: "Antifire potion";
  export const ANTIFIRE_DOSES: readonly string[];
  export function antipoisonPlan(want: number): FlaskPlan;
  export function prayerPlan(want: number): FlaskPlan;
  export function antifirePlan(want: number): FlaskPlan;
  export function doseToDrink(count: (name: string) => number, doses?: readonly string[]): string | null;
  export function escapeRunesFor(teleportId: string): EscapeSpell;
  export function inCell(): boolean;
  export function enterLair(h: JiveHost, site: DragonSite): Promise<boolean>;
  export function feePrepaid(site: DragonSite): boolean;
  export function acquireKey(h: JiveHost, site: DragonSite): Promise<KeyState>;
  export function leaveCell(h: JiveHost): Promise<boolean>;
  export function teleportOut(h: JiveHost, site: DragonSite): Promise<string | null>;
  export function waitFed(cond: () => boolean, ms: number): Promise<boolean>;
  export function bankRoutine(h: JiveHost, site: DragonSite, opts: BankOpts): Promise<void>;
  export function walkApproach(): unknown;
  export function withdrawTo(): unknown;
  export function leaveLair(h: JiveHost, site: DragonSite): Promise<boolean>;
}

declare module '*api/combat/keepList.js' {
  export interface KeepParams {
      food: string;
      style: 'melee' | 'mage' | 'range';
      spell?: string;
      ammo?: string;
      weapon?: string;
      extra?: string[];
  }
  export function combatKeepNames(o: KeepParams): string[];
}

declare module '*api/combat/meleeWeapons.js' {
  export interface WeaponPick {
      /** The character's Attack level. */
      attack: number;
      /** Rank stab weapons first, for a target soft to stab. */
      preferStab: boolean;
      /** Names a wield already refused, a level or a quest short. */
      unusable?: ReadonlySet<string>;
  }
  export function bestMeleeWeapon(available: readonly string[], pick: WeaponPick): string | null;
  export function knownMeleeWeapon(names: readonly string[]): string | null;
}

declare module '*api/combat/ranged.js' {
  export interface RangeLoadout {
      weapon: string;
      projectile: string;
      thrown: boolean;
  }
  export const RANGED_WEAPONS: string[];
  export const ROCK_CRAB_RANGED_WEAPONS: string[];
  export function rangeLoadoutOf(weapon: string, ammo: string): RangeLoadout;
  // Runtime wins: shim is `rockCrabRangeLoadout(...args)` (rest); frozen requires (weapon, ammo).
  export function rockCrabRangeLoadout(weapon?: string, ammo?: string): RangeLoadout;
  export function rangeSupplyEmpty(equipped: number, carried: number, ground: number): boolean;
}

declare module '*api/combat/rangedSettings.js' {
  /** Runtime bots pass LoopingBot.settings (SettingsView); frozen SettingsBag is structurally compatible via `.str`. */
  export type SettingsStr = { str(name: string, fallback?: string): string };
  export const CUSTOM_RANGED_SETTINGS: {
    customBow: unknown;
    customAmmo: unknown;
  };
  export function rangedItem(settings: SettingsStr, key: 'bow' | 'ammo', fallback: string): string;
}

declare module '*api/cooking/CookLocations.js' {
  export interface WorldTile { x: number; z: number; level: number }
  export type CookLocation = { name: string; [key: string]: unknown };
  export const COOK_LOCATION_OPTIONS: readonly string[];
  export function cookLocation(name: string): CookLocation | null;
  export function resolveCookLocation(setting: string, from: WorldTile, unlocked?: (loc: CookLocation) => boolean): CookLocation | null;
  export const CUSTOM_LOCATION: string;
  export const COOK_LOCATIONS: readonly CookLocation[];
}

declare module '*api/duel/ClueDuel.js' {
  export const CLUE_DUEL_LOBBY: any;
  export const CLUE_DUEL_OPTIONS: 1024;
  export function clueDuelName(name: string | null): string;
  export function leaveClueDuel(log: (msg: string) => void): Promise<boolean>;
  export class ClueDuelHandshake {
    constructor(partner: string, initiator: boolean, log: (message: string) => void);
    readonly partner: string;
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
  import type { Player } from '*api/players/Players.js';
  export interface WorldTile { x: number; z: number; level: number }
  export interface Rect {
    minX: number;
    maxX: number;
    minZ: number;
    maxZ: number;
}

  export const DUEL_SELECT_MODAL: 6575;
  export const DUEL_CONFIRM_MODAL: 6412;
  export const DUEL_WIN_MODAL: 6733;
  export const DUEL_FIGHT_ARENAS: readonly Rect[];
  export function parseDuelPartnerHeader(header: string | null): string | null;
  export function fightArenaAt(tile: WorldTile | null): Rect | null;
  export const Duel: {
    offerOpen(): boolean;
    confirmOpen(): boolean;
    winOpen(): boolean;
    active(): boolean;
    partner(): string | null;
    waitingForOther(): boolean;
    challenge(player: Player): boolean | Promise<boolean>;
    fight(player: Player): boolean | Promise<boolean>;
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
    setInterrupt(p: (() => boolean) | null): void;
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
  import Tile from '*geometry/Tile.js';

  export type BurnMode = 'off' | 'chop-then-burn';
  /** What one light attempt did. */
  export type LightOutcome = 'lit' | 'blocked' | 'stalled';
  export interface WorldTile { x: number; z: number; level: number }
  export type BurnDir = { dx: number; dz: number };
  export interface FirePlot {
    bank: Tile;
    x0: number;
    x1: number;
    z0: number;
    z1: number;
}
  export const TINDERBOX: "Tinderbox";
  export const CANT_LIGHT: RegExp;
  export const FIRE_START_TICKS: 14;
  export const FIRE_LIGHT_TICKS: 150;
  export const BURN_WEST: { dx: number; dz: number };
  export const BURN_DIRS: BurnDir[];
  export const FIRE_SPOTS: Record<string, FirePlot>;
  export const FIRE_SPOT_OPTIONS: string[];
  export function localFirePlot(origin: WorldTile, half?: number): FirePlot;
  export const LOG_LEVELS: Record<string, number>;
  export function tileKey(t: { x: number; z: number; }): string;
  export class NoLightTiles {
    add(tile: { x: number; z: number; }): void;
    has(tile: { x: number; z: number; }): boolean;
    get size(): number;
    merge(occupied: Iterable<string>): Set<string>;
    clear(): void;
  }
  export function inFirePlot(t: WorldTile, plot: FirePlot): boolean;
  export function burnLaneWant(logCount: number): number;
  export function isBurnWest(dir: BurnDir): boolean;
  export function fireReactionTicks(): number;
  export function runInDir(from: WorldTile, plot: FirePlot, dir: BurnDir, occupied: ReadonlySet<string>, walkable: (t: WorldTile) => boolean, canStep: (from: WorldTile, to: WorldTile) => boolean, cap: number): number;
  export function findBurnLane(plot: FirePlot, here: WorldTile, occupied: ReadonlySet<string>, want?: number, walkable?: (t: WorldTile) => boolean, canStep?: (from: WorldTile, to: WorldTile) => boolean, directions?: readonly BurnDir[]): { start: Tile; run: number; dir: BurnDir; } | null;
  export function lightFire(logName: string): Promise<string>;
}

declare module '*api/firemaking/LightFire.js' {
  export type LightOutcome = import('*api/firemaking/Firemaking.js').LightOutcome;
  export function lightFire(logName: string): Promise<LightOutcome>;
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
  export interface CombatModeLabel {
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
    combatStyles(): readonly CombatModeLabel[] | null;
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
    get name(): string | null;
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
  export interface PackItem {
      readonly name: string | null;
      readonly count: number;
  }
  export function matchesAny(name: string | null, patterns: string[]): boolean;
  export function countMatching(items: readonly PackItem[], patterns: string[]): number;
  export function slotsMatching(items: readonly PackItem[], patterns: string[]): number;
  export function shouldBank(lootSlots: number, bankAt: number, invFull: boolean): boolean;
  export function shouldRestock(foodCount: number, threshold: number): boolean;
  export function shouldEat(hp: number, maxHp: number, heal: number, foodCount: number): boolean;
  export function shouldPanic(hpFrac: number, gate: number, foodCount: number): boolean;
}

declare module '*api/loadout/loadoutPlan.js' {
  /** Runtime bots pass LoopingBot.settings (SettingsView); frozen SettingsBag is structurally compatible via `.str`. */
  export type SettingsStr = { str(name: string, fallback?: string): string };
  export type Loadout = { food?: string; [key: string]: unknown };
  export type CarryEntry = { name: string; count?: number };
  export function foodOf(loadout: Loadout | null, fallback: string): string;
  export function gearOf(loadout: Loadout | null): string[];
  export function suppliesOf(loadout: Loadout | null): CarryEntry[];
  export function weaponOf(loadout: Loadout | null, fallback?: string | null): string | null;
  export function scriptFood(bag: SettingsStr, fallback: string): string;
  export function scriptFoods(bag: SettingsStr, fallback: readonly string[]): string[];
}

declare module '*api/loadout/loadoutSetting.js' {
  /** Runtime bots pass LoopingBot.settings (SettingsView); frozen SettingsBag is structurally compatible via `.str`. */
  export type SettingsStr = { str(name: string, fallback?: string): string };
  export type Loadout = { food?: string; [key: string]: unknown };
  export const LOADOUT_SETTING: {
    type: unknown;
    default: unknown;
    options: unknown;
    optionsFrom: unknown;
    label: unknown;
    help: unknown;
  };
  export function selectedLoadout(bag: SettingsStr): Loadout | null;
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
    armed(): boolean;
    staffTabAttached(): boolean;
    arm(spellName: string, log: (m: string) => void): Promise<boolean>;
  };
}

declare module '*api/market/MarketMaker.js' {
  export const MarketMaker: any;
}

declare module '*api/market/catalog.js' {
  export type ObjRecord = { id: number; name: string; [key: string]: unknown };
  export type ItemAlias = { words: string[]; name: string };
  export interface Catalog {
    byId: Map<number, ObjRecord>;
    /** unnoted id -> noted id */
    notedOf: Map<number, number>;
    /** noted id -> unnoted id */
    unnotedOf: Map<number, number>;
    /** unnoted entries only, name-sorted */
    items: ObjRecord[];
    /** id -> the words that separate it from its same-named siblings, and the name to say back */
    aliases: Map<number, ItemAlias>;
}
  export function liveCatalog(): Catalog;
  export function tradeable(id: number): boolean;
  export function clientName(cat: Catalog, id: number): string | undefined;
  export function displayName(cat: Catalog, id: number): string;
  export function notedId(cat: Catalog, id: number): number | null;
  export function unnotedId(cat: Catalog, id: number): number;
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
  export type PlayerSnapshot = {
    name: string | null;
    index: number;
    x?: number;
    z?: number;
    level?: number;
    combatLevel?: number;
    inCombat?: boolean;
  };
  /** PlayerSnapshot + empty ops for snapshot-first EntityQuery. */
  export type PlayerSnapRow = PlayerSnapshot & { ops: readonly (string | null)[] };
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
  export interface PrayerDef {
      com: number;
      varp: number;
      level: number;
  }
  export const PROTECT_FROM_MAGIC: "Protect from Magic";
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
  /**
   * Minimal fields shared by Loc/Npc/GroundItem snapshots and adapted players.
   * Why: name, action and distance filters run before any entity wrapper is allocated, on the snapshot's `ops`.
   */
  export interface WorldTile { x: number; z: number; level: number }
  export interface EntitySnapView {
      name: string | null;
      /** Menu ops; null/hidden slots ignored by {@link EntityQuery.action}. */
      ops: readonly (string | null)[];
      tile: WorldTile;
      distance: number;
  }
  export function matchesEntityName(actual: string | null, configured: string): boolean;
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
  import type { ShopRecord } from '*api/shop/types.js';
  export interface BuyoutItem {
      obj: string;
      name: string;
      units: number;
      estCost: number;
  }
  export function buyoutPlan(rec: ShopRecord, stock: Record<string, number>, coins: number, chosen: ReadonlySet<string>): BuyoutItem[];
}

declare module '*api/shop/Shop.js' {
  export const Shop: {
    isOpen(): boolean;
    stock(): Array<{ name: string | null; count: number; slot: number }>;
    player(): Array<{ name: string | null; count: number; slot: number }>;
    open(npcName?: string): Promise<boolean>;
    buy(name: string, qty?: number): Promise<number>;
    sell(name: string, n: number, pick?: (i: { id: number; count: number; slot: number; }) => boolean): Promise<number>;
    sellAll(name: string, pick?: (i: { id: number; count: number; slot: number; }) => boolean): Promise<number>;
    close(): Promise<void>;
    buyById(): never;
  };
}

declare module '*api/shop/types.js' {
  export interface NavPointLike { x: number; z: number; level: number }
  export interface ShopItemDef {
      obj: string;
      name: string;
      baseline: number;
      restockTicks: number;
      cost: number;
      stackable: boolean;
      members: boolean;
  }
  export type BuyPolicy = { kind: 'buyout' } | { kind: 'floor'; pct: number };
  export interface GateSpec {
      quest?: string;
      skill?: { name: string; level: number };
      qp?: number;
  }
  export interface RouteShop {
      shopId: string;
      keeperNpc: string;
      stand: NavPointLike;
      buys: { obj: string; policy?: BuyPolicy }[];
  }
  export interface RouteCluster {
      id: string;
      bank: { stand: NavPointLike; boothName: string; boothOp: string; banker?: string };
      shops: RouteShop[];
      gates: GateSpec[];
      keep?: string[];
      /** free ground spawn to fetch a keep item from when the bank has none */
      keepFallback?: { item: string; spawn: NavPointLike };
      wield?: string[];
      waypoints?: NavPointLike[];
      setting?: string;
      haulBank?: { stand: NavPointLike; banker: string };
      repeatWhileFull?: boolean;
  }
  export interface Route {
      clusters: RouteCluster[];
      ring: string[];
  }
  export interface Seen { count: number; atMs: number }
  export interface AccountView {
      qp: number;
      quests: Record<string, boolean>;
      skills: Record<string, number>;
  }
  export interface ShopRecord {
    inv: string;
    title: string;
    keepers: string[];
    sell: number;
    buy: number;
    delta: number;
    scope: string;
    allstock: boolean;
    items: ShopItemDef[];
}

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
    hook: (() => Promise<void>) | null;
    running: boolean;
    set(hook: (() => Promise<void>) | null): void;
    run(): Promise<void>;
  };
}

declare module '*api/tasks/Anchor.js' {
  import Tile from '*geometry/Tile.js';
  import type { Task } from '*api/bot/Bot.js';
  export interface WorldTile { x: number; z: number; level: number }

  export interface AnchorHost {
      getAnchor(): Tile;
      leashRadius(): number;
  
      setStatus?(s: string): void;
      log?(msg: string): void;
  }
  export interface ReturnToAnchorOptions {
      slack?: number;
      arriveRadius?: number;
      timeoutMs?: number;
      /** When set and non-empty, final approach opens matching doors/gates via walkOpening. */
      obstacles?: string[];
      /** Past this distance walkResilient (web path) runs before the local approach; omit or <= 0 skips the long-range leg. */
      longRangeTiles?: number;
      suppress?: () => boolean;
      status?: string;
  }
  export const HOME_ARRIVE_RADIUS: 8;
  export function shouldWalkHomeToGatherAnchor(_distToAnchor: unknown, _arriveRadius: unknown): unknown;
  export function shouldSoftHomeFromGatherMiss(_distToAnchor: unknown, _leash: unknown): unknown;
  export function beyondLeash(host: AnchorHost, here?: WorldTile | null, slack?: number): boolean;
  export function tileWithinLeash(host: AnchorHost, tile: WorldTile, slack?: number): boolean;
  export function resolveRunAnchor(here: WorldTile, locationSpot: Tile | null | undefined): Tile;
  export function createReturnToAnchorTask(host: AnchorHost, opts?: ReturnToAnchorOptions): Task;
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
  import type { AbstractBot } from '*api/bot/Bot.js';
  export interface WorldTile { x: number; z: number; level: number }
  export type ItemNeed = { name: string; count?: number };
  export interface DeathRecoveryOptions {
      anchor: WorldTile;
      radius?: number;
      needs?: ItemNeed[];
      onDeath?: () => void;
      onRecovered?: () => void;
      walkBack?: () => Promise<boolean>;
  }
  export class DeathRecovery {
    constructor(bot: AbstractBot, opts: DeathRecoveryOptions);
    opts: unknown;
    validate(): boolean;
    execute(): Promise<void>;
  }
}

declare module '*api/tasks/PeriodicBank.js' {
  import type { BankStrategy } from '*api/bank/bankRules.js';
  import type { BankDestination } from '*api/bank/Banking.js';
  export interface WorldTile { x: number; z: number; level: number }
  export interface PeriodicBankOptions {
      strategy: () => BankStrategy;
      itemsThreshold: () => number;
      minutesThreshold: () => number;
      countLoot: () => number;
      deposit: (name: string) => boolean;
      afterDeposit?: () => void | Promise<void>;
      destination?: () => BankDestination | null;
      commonJunk?: () => boolean;
      returnTo?: () => WorldTile | null;
      setStatus?: (s: string) => void;
      log?: (m: string) => void;
  }
  export class PeriodicBank {
    constructor(opts: PeriodicBankOptions);
    opts: unknown;
    validate(): boolean;
    execute(): Promise<void>;
  }
}

declare module '*api/thieving/CakeStall.js' {
  export type StealCakesResult = 'stocked' | 'combat' | 'aborted' | 'no-progress';
  export interface StealCakesOptions {
      /** Stop once this many stall foods are held. Omit to fill the pack. */
      fillTo?: number;
      abort: () => boolean;
      shouldEat?: () => boolean;
      lockedOutUntil?: () => number;
      setStatus: (s: string) => void;
      log: (m: string) => void;
      onSteal?: () => void;
      onReset?: () => void;
  }
  export function carriedCakes(): number;
  export function needsCakeRestock(target: number): boolean;
  // Runtime wins: shim `stealCakes(opts = {})` — opts is optional.
  export function stealCakes(opts?: StealCakesOptions): Promise<StealCakesResult>;
}

declare module '*api/thieving/cakeStallData.js' {
  import Tile from '*geometry/Tile.js';
  export type StealOutcome = 'success' | 'caught' | 'lockout' | 'refused' | 'timeout';
  export interface StealSignals {
      gained: boolean;
      combat: boolean;
      lockoutSeen: boolean;
      attemptSeen: boolean;
  }
  export const STALL_TILE: Tile;
  export const STAND: Tile;
  export const STAND_ALT: Tile;
  export const FLEE_TILE: Tile;
  export const STALL_NAME: "Baker's stall";
  export const STALL_OP: "Steal from";
  export const CAKE_ITEMS: string[];
  export const LOCKOUT_TICKS: 10;
  export const RESET_AFTER_REFUSALS: 3;
  export function classifySteal(): unknown;
  export function shouldReset(consecutiveRefusals: number): boolean;
}

declare module '*api/thieving/stealRules.js' {
  export interface NamedStack {
      name: string | null;
      count: number;
  }
  export type WithdrawChunk =
      | { kind: 'x'; count: number }
      | { kind: 'op'; op: 'Withdraw-10' | 'Withdraw-5' | 'Withdraw-1' };
  export const THIEVER_BANKING_OPTIONS: string[];
  export const STUN_COMBAT_TICKS: 9;
  export function nextWithdrawChunk(need: number): WithdrawChunk | null;
  export function withdrawTo(name: string, target: number, countInInv?: () => number): Promise<number>;
  export function closeBankAndConfirmCount(expected: number, count: () => number): Promise<boolean>;
  export function autoFoodBanking(mode: string): boolean;
  export function foodMatches(name: string | null, keyword: string): boolean;
  export function countFood(items: NamedStack[], keyword: string): number;
  export function shouldRestockFood(enabled: boolean, foodCount: number, restockAt: number, bankablePackFull: boolean): boolean;
  export function safeToSteal(hpFraction: number, eatAt: number, foodCount: number): boolean;
  export function canStealNow(foodCount: number, hp: number, minEatHp: number, suicide: boolean): boolean;
}

declare module '*api/thieving/targets.js' {
  import Tile from '*geometry/Tile.js';
  export interface TargetSpot {
      anchor: Tile;
      leash: number;
  }
  export interface AttackerCandidate {
      name: string | null;
      inCombat: boolean;
      distance: number;
      actions: string[];
      targetsAnotherPlayer: boolean;
  }
  export function targetSpot(target: string): TargetSpot;
  export function requiredThieving(target: string): number;
  export const HOSTILE_NAMES: readonly string[];
  export function isHostileAttacker(c: AttackerCandidate, maxDistance: number): boolean;
  export function chooseTarget<T>(candidatesNearestFirst: T[], reachable: (t: T) => boolean): { target: T | null; blocked: T | null; };
  export const PICKPOCKET_TARGET_NAMES: unknown;
  export const ARDOUGNE_PICKPOCKET_TARGETS: unknown;
}

declare module '*api/trade/PartnerTrade.js' {
  export type ReceiverOfferDecision =
      | { action: 'wait-header' }
      | { action: 'decline'; reason: string }
      | { action: 'wait-offer' }
      | { action: 'accept' };
  export type GiverOfferDecision = 'offer' | 'accept' | 'wait';
  /** GatheringBot partner roles. */
  export type MuleMode = 'off' | 'gatherer' | 'mule' | 'cooker' | 'supplier';
  export const DEFAULT_TRADE_RANGE: 2;
  export const MULE_MODE_OPTIONS: readonly [ "Off", "Gatherer", "Mule", "Cooker", "Supplier" ];
  export function parsePartnerList(raw: string): string[];
  export function namesMatch(a: string, b: string): boolean;
  export function isConfiguredPartner(name: string | null | undefined, partners: readonly string[]): boolean;
  export function countOfferByName(items: readonly { name: string | null; count: number; }[], itemName: string): number;
  export function decideReceiverOfferScreen(opts: { partnerHeader: string | null; partners: readonly string[]; myOfferSlots: number; theirProductCount: number; }): ReceiverOfferDecision;
  export function decideGiverOfferScreen(myOfferSlots: number): GiverOfferDecision;
  export function parseMuleMode(raw: string): MuleMode;
  export function muleGathererHandoffActive(mode: MuleMode, partners: readonly string[], powerMode: boolean): boolean;
  export function muleReceiverActive(mode: MuleMode, partners: readonly string[]): boolean;
  export function muleCookerActive(mode: MuleMode, partners: readonly string[]): boolean;
  export function muleSupplierActive(mode: MuleMode, partners: readonly string[], powerMode: boolean): boolean;
  export function muleNonGathererActive(mode: MuleMode, partners: readonly string[]): boolean;
  export function countOfferMatching(): number;
}

declare module '*api/trade/Trade.js' {
  export interface TradeItem {
      id: number;
      name: string | null;
      count: number;
  }
  export const Trade: {
    active(): boolean;
    onOfferScreen(): boolean;
    onConfirmScreen(): boolean;
    partner(): string | null;
    myOffer(): TradeItem[];
    theirOffer(): TradeItem[];
    request(playerName: string): Promise<boolean>;
    offerAll(itemName: string, pick?: (i: { count: number; id: number; slot: number; }) => boolean): Promise<boolean>;
    offer(itemName: string, n: number, pick?: (i: { count: number; id: number; slot: number; }) => boolean): Promise<boolean>;
    removeAll(): Promise<boolean>;
    accept(): Promise<boolean>;
    decline(): Promise<void>;
  };
}

declare module '*api/trade/drivePartnerTrade.js' {
  export type PartnerTradeRole = 'giver' | 'receiver';
  export interface DrivePartnerTradeOpts {
      role: PartnerTradeRole;
      partners: readonly string[];
      /** Receiver: which of their offer slots count as product. */
      theirProductMatch: (name: string) => boolean;
      /** Giver: product stack names to Offer-All (case-sensitive display names). */
      productNamesToOffer: () => readonly string[];
      setStatus: (s: string) => void;
      log: (m: string) => void;
      /**
       * Called once when confirm completes and the modal closes.
       * `metricDelta` is after minus before from {@link inventoryMetric}.
       */
      onComplete?: (metricDelta: number) => void;
      /** Called when we decline (stranger, empty haul, safety, receiver gate). */
      onDecline?: (reason: string) => void;
      /**
       * Metric for confirm delta (default {@link Inventory.used}).
       * FlaxRunner uses flax stack counts instead.
       */
      inventoryMetric?: () => number;
      /**
       * When partner header is still null. Default waits 1 tick.
       * Flax declines after ~8 consecutive waits.
       */
      onMissingPartner?: () => 'wait' | 'decline';
      /** Receiver: extra gate once their product is present (e.g. free pack slots). */
      receiverCanAccept?: (
          theirProductCount: number
      ) => boolean | { ok: true } | { ok: false; reason: string };
      /**
       * Giver: when offer is "ready" to accept (default: any own offer slot).
       * Flax uses flax units in offer.
       */
      myOfferReady?: () => boolean;
      // Why: the metric snapshot comes from handshake start, since a giver's offered stack leaves the pack at offer time and a confirm-time baseline reads every trade as delta 0.
      baseline?: () => number;
      /**
       * Giver: decline non-partners / wait on blank header (Flax). Default false
       * so GatheringBot gatherer keeps offering without a partner-header gate.
       */
      verifyGiverPartner?: boolean;
      /** Optional status labels. */
      labels?: {
          accepting?: string;
          offering?: string;
          confirming?: string;
          waitHeader?: string;
          waitOffer?: string;
          declining?: string;
          acceptingOffer?: string;
      };
  }
  export function driveActivePartnerTrade(opts: DrivePartnerTradeOpts): Promise<void>;
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
  /**
 * Coarse quest-list colour. `unknown` means the tab hasn't loaded yet.
 * @see docs/reference/quest-engine.md#quest-state
 */
export type QuestStatus = 'notStarted' | 'inProgress' | 'complete' | 'unknown';
  export type QuestRow = { name: string; status: QuestStatus };

  export const Quests: {
    all(): { name: string; status: QuestStatus; }[];
    status(name: string): QuestStatus;
    // Runtime wins: shim `journal()` is notImpl with no params; frozen requires `name`.
    journal(name?: string): Promise<string[]>;
    points(): number;
  };
}

declare module '*api/ui/widgets/Modals.js' {
  export const Modals: {
    main(): number;
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
  export interface WorldTile { x: number; z: number; level: number }
  export interface ReachEntity {
    interact(action: string): boolean;
    tile?(): WorldTile | null;
  }
  export type ReachStatus = 'done' | 'retry' | 'unreachable';
  export interface ReachLocOpts {
      name: string;
      op: string;
      near: WorldTile;
      within?: number;
      // Why: Display names collide; four searchable crates surround Wydin's quest crate.
  
      /** Exact loc id, when the display name is shared with something else in range. */
      id?: number;
      expect: () => boolean;
      expectMs?: number;
      // Why: Provide other refusal patterns explicitly or each attempt waits the full `expectMs`.
  
      /** Game-message pattern emitted when the loc op cannot run yet. */
      refused?: RegExp;
      log?: (m: string) => void;
  }
  export interface ReachNpcOpts {
      name: string;
      near: WorldTile;
      openMs?: number;
      log?: (m: string) => void;
  }
  export interface ReachEntityOpts<T extends ReachEntity> {
      find: () => T | null;
      op: string;
      expect: () => boolean;
      /** Probe the scene for a shut door instead of waiting on the server's verdict. */
      openWhenUnreachable?: boolean;
      expectMs?: number;
      what?: string;
      log?: (m: string) => void;
  }
  export const Reach: {
    entityOp<T extends ReachEntity>(opts: ReachEntityOpts<T>): Promise<ReachStatus>;
    npcDialog(opts: ReachNpcOpts): Promise<ReachStatus>;
  };
}

declare module '*api/walking/Traversal.js' {
  /**
   * Options for a walk behind the escalation ladder.
   * @see docs/reference/nav-walker.md#when-it-gets-stuck
   */
  export interface WalkResilientOptions {
      radius: number;
      attempts?: number;
      timeoutMs?: number;
      sceneRadius?: number;
      maxBudget?: number;
      log?: (msg: string) => void;
      /** Forwarded to WalkExecutor on every baked repath. */
      useTeleportCatalog: true;
      policy?: WalkOptions['policy'];
      bankItemCounts?: Record<string, number>;
      /**
       * No-go zones for every baked repath (same as WalkOptions.avoidZones), as known ids or ad-hoc rects.
       * Why: WalkExecutor resolves the automatic catalog zones from live player state.
       */
      avoidZones?: WalkOptions['avoidZones'];
  }
  export interface WorldTile { x: number; z: number; level: number }
  export interface WalkOptions {
    radius?: number;
    timeoutMs?: number;
    attempts?: number;
    sceneRadius?: number;
    useTeleportCatalog?: boolean;
    policy?: {
      useTeleports: true;
      distanceBeforeTeleport?: number;
      useShips?: boolean;
      useShortcuts?: boolean;
      allowTeleportIds?: unknown;
      denyTeleportIds?: unknown;
    };
    avoidZones?: unknown[];
    pathFollow?: unknown;
    bankItemCounts?: Record<string, number>;
    forceRepath?: boolean;
    log?: (message: string) => void;
  }
  export const Traversal: {
    walkTo(tile: WorldTile, opts?: WalkOptions): Promise<boolean>;
    walkResilient(tile: WorldTile, opts?: WalkOptions): Promise<boolean>;
    preload(): void;
    remaining(): number;
    teleportsEnabled(): boolean;
    requestRepath(reason?: string): void;
    pureWalk: { useTeleportCatalog: false; policy: { useTeleports: false; }; };
    withTeles: { useTeleportCatalog: true; policy: { useTeleports: true; }; };
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

