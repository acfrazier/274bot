-- Luau script API v2: type declarations for the experimental Luau runtime.
--
-- Checked in by hand. `crates/script-luau/tests/type_declarations.rs` compares
-- the `declare api` block below with the member table of the runtime, member
-- by member and signature by signature, so a member cannot be added, removed
-- or retyped in the runtime without this file changing.
--
-- A Luau language server that loads definition files takes this file as one.
-- The stock `luau-analyze` has no such option; it checks a script after these
-- declarations are pasted above it as `local` declarations.
--
-- Not declared here, because the runtime has no such member: any `api.hunt`,
-- a packet API, a `require`, and every JS-only helper. The `scenario` global of
-- scenario files is described in docs/dev/scenario.d.luau.
--
-- Each member carries its call class in parentheses:
--   Read     reads the game state lent to the current entry (`tick`, `onPaint`)
--   Request  makes one host call that returns at once
--   Await    makes one host call and suspends `tick` until it settles
--   Output   writes script output

export type Tile = { x: number, z: number, level: number }

-- manifest -----------------------------------------------------------------

export type ShowIf = { key: string, anyOf: { string } }

export type BooleanSetting = {
    type: "boolean",
    default: boolean,
    label: string?,
    group: string?,
    help: string?,
    showIf: ShowIf?,
}

export type NumberSetting = {
    type: "number",
    default: number,
    min: number?,
    max: number?,
    label: string?,
    group: string?,
    help: string?,
    showIf: ShowIf?,
}

export type StringSetting = {
    type: "string",
    default: string,
    options: { string }?,
    optionLabels: { [string]: string }?,
    optionsFrom: ("loadouts" | "named-banks")?,
    label: string?,
    group: string?,
    help: string?,
    showIf: ShowIf?,
}

export type StringListSetting = {
    type: "string[]",
    default: { string },
    options: { string }?,
    optionLabels: { [string]: string }?,
    optionsFrom: ("loadouts" | "named-banks")?,
    label: string?,
    group: string?,
    help: string?,
    showIf: ShowIf?,
}

export type TileSetting = {
    type: "tile",
    default: Tile,
    label: string?,
    group: string?,
    help: string?,
    showIf: ShowIf?,
}

export type SettingDef = BooleanSetting | NumberSetting | StringSetting | StringListSetting | TileSetting

-- The closed literal table that follows the first statement `manifest`.
-- `apiVersion` must be the integer 2 (Luau has no numeric literal types).
-- `capabilities` must be empty: the Luau API has no packet member.
export type ScriptManifest = {
    apiVersion: number,
    name: string,
    version: string,
    author: string,
    description: string,
    category: string,
    tags: { string },
    settings: { [string]: SettingDef },
    capabilities: {},
}

-- The host reads this statement without a VM and erases it before the script
-- runs. The declaration exists for the type checker only.
declare function manifest(definition: ScriptManifest): ()

-- walk ---------------------------------------------------------------------

export type WalkOptions = {
    allow_teleports: boolean?,
    allow_wilderness: boolean?,
    allow_bank_fetch: boolean?,
}

export type AbortReason = "reset" | "superseded" | "terminated" | "user-input" | "parent-aborted"

-- The result of `walk.to` and `walk.near`. `ok` is `not failed`. A walk that a
-- session reset ended carries `aborted` and none of the position fields.
export type WalkResult = {
    ok: boolean,
    failed: boolean,
    x: number?,
    z: number?,
    level: number?,
    radius: number?,
    blocked: boolean?,
    no_path: boolean?,
    allow_teleports: boolean?,
    cancel_reason: ("none" | "user-input")?,
    request_id: number?,
    generation: number?,
    seq: number?,
    aborted: AbortReason?,
}

-- bank ---------------------------------------------------------------------

export type OpenBoothOptions = { name: string?, action: string? }
export type OpenStandOptions = { name: string?, stand_op: number?, choose: string? }

export type WithdrawXRequest = {
    name: string,
    count: number,
    bank_item_id: number,
    lands_as_id: number,
    action: string,
    bank_generation: number,
}

-- The result of `bank.openNearest`, `bank.depositAll` and `bank.close`.
-- `reason` is set for `refused` and `aborted`. `value` is set when `has_value`.
export type BankResult = {
    status: "done" | "failed" | "refused" | "aborted",
    has_value: boolean,
    value: any?,
    reason: string?,
}

-- route and puzzle ---------------------------------------------------------

export type AvoidRect = { minX: number, maxX: number, minZ: number, maxZ: number, level: number? }

-- `request_id` must be 0 or nil. `avoid` holds at most 16 rectangles.
export type InspectRouteOptions = {
    allow_teleports: boolean?,
    allow_wilderness: boolean?,
    allow_bank_fetch: boolean?,
    avoid: { AvoidRect }?,
    max_expansions: number?,
    request_id: number?,
}

export type RouteHop = {
    kind: string,
    locId: number,
    locName: string,
    action: string,
    option: number,
    ticks: number,
    from: Tile,
    to: Tile,
}

export type RouteInspectResult = {
    seq: number,
    generation: number,
    request_id: number,
    ok: boolean,
    reason: string,
    bank_planned: boolean,
    ticks: number,
    hops: { RouteHop },
}

export type PuzzleItem = { id: number, slot: number, count: number, component_id: number }

export type PuzzleBoard = {
    component_id: number,
    size: number,
    generation: number,
    items: { PuzzleItem },
}

-- combat -------------------------------------------------------------------

-- One box a combat target must stand in. All five keys are required.
export type AreaBox = { min_x: number, min_z: number, max_x: number, max_z: number, level: number }

-- Exactly one key.
export type CombatTarget = {
    npc: (string | { string })?,
    npcId: (number | { number })?,
    attackers: ("npcs" | "players" | "any")?,
    player: string?,
}

export type CombatRequest = {
    target: CombatTarget,
    pick: ("nearest" | "random" | "lowestHealth")?,
    notTargetingOthers: boolean?,
    area: (AreaBox | { AreaBox })?,
    radius: number?,
    lostRadius: number?,
    style: ("melee" | "ranged" | "magic")?,
    meleeMode: ("accurate" | "aggressive" | "defensive" | "controlled")?,
    rangedMode: ("accurate" | "rapid" | "longRange")?,
    spells: { string }?,
    fallbackSpells: boolean?,
    prayer: boolean?,
    food: boolean?,
    potions: boolean?,
    retaliate: boolean?,
    budgetTicks: number?,
    pvp: boolean?,
}

export type CombatReport = {
    ["end"]: "killed" | "target-gone" | "no-target" | "budget" | "died" | "aborted",
    reason: string?,
    npcType: number,
    ticks: number,
    swings: number,
    casts: number,
    damageTaken: number,
    food: number,
    prayerDoses: number,
    boostDoses: number,
    antifireDoses: number,
    protectSwitches: number,
    pvpSource: ("request" | "global" | "both")?,
}

-- `value["end"]` tells which fields exist: `fought` has `report`,
-- `interrupted` has `cause`, `refused` and `failed` have `reason`.
export type CombatEnd = {
    ["end"]: "fought" | "stopped" | "interrupted" | "refused" | "failed",
    token: number,
    report: CombatReport?,
    cause: ("pause" | "died" | "reconnect")?,
    reason: string?,
}

-- `done` has `value`. `refused` and `aborted` have `reason`.
export type CombatOutcome = {
    kind: "done" | "refused" | "aborted",
    value: CombatEnd?,
    reason: string?,
}

-- gatherer -----------------------------------------------------------------

export type GatherSettings = {
    skill: ("Woodcutting" | "Mining" | "Fishing")?,
    woodcuttingResources: { string }?,
    miningResources: { string }?,
    fishingMethod: string?,
    targetPreference: ("Best tier" | "Nearest")?,
    tickMethod: ("Off" | "Knife")?,
    location: ("Start" | "Site" | "Auto" | "Custom")?,
    site: string?,
    customTile: Tile?,
    radius: number?,
    disposition: ("Power" | "Bank")?,
    bank: "Nearest"?,
    baitTarget: number?,
    buyTools: boolean?,
    food: string?,
    foodTarget: number?,
    eatBelow: number?,
    coinTarget: number?,
    reserveTeleport: "Off"?,
    reserveCasts: number?,
    allowTeleports: boolean?,
    allowWilderness: boolean?,
    allowDangerZones: boolean?,
    deathPolicy: ("Stop" | "Recover")?,
    maxDeaths: number?,
}

export type GatherFailure = { code: string, message: string }
export type GatherCounts = { yielded: number, dropped: number, deposited: number, trips: number, xp: number }

-- `stopped` and `blocked` and `failed` carry `counts`. `blocked` also carries
-- `failure`. `refused` and `failed` carry `reason`.
export type GatherEnd = {
    ["end"]: "stopped" | "blocked" | "refused" | "failed",
    token: number,
    counts: GatherCounts?,
    failure: GatherFailure?,
    reason: string?,
}

export type GatherOutcome = {
    kind: "done" | "refused" | "aborted",
    value: GatherEnd?,
    reason: string?,
}

-- quests -------------------------------------------------------------------

export type QuestTabStatus = "notStarted" | "inProgress" | "complete" | "unknown"

-- `ok` true: `status` and `as_of_sequence`. `ok` false: `error`.
export type QuestStatusResult = {
    ok: boolean,
    status: QuestTabStatus?,
    as_of_sequence: number?,
    error: ("invalid-args" | "snapshot-unavailable" | "quest-tab-unbound" | "not-on-tab")?,
}

export type QuestPathRow = {
    id: string,
    display: string,
    journal: boolean,
    draft: boolean,
    stages: { string },
}

export type Truth3 = "true" | "false" | "unknown"

-- `state` is `known` (with `value`) or `unknown` (with `gap`).
export type Known = { state: "known" | "unknown", value: string?, gap: string? }

export type QuestProgressRow = {
    quest: string,
    display: string,
    colour: QuestTabStatus,
    stage: Known,
    complete: Truth3,
    rule: Known,
    flags: { { flag: string, truth: Truth3, count: number? } },
    evidence: { run: number, session: number, tick: number, sequence: number },
    journal_read: boolean,
    binding: string,
    role: string?,
}

export type QuestProgressEnd = {
    ["end"]: "done" | "refused",
    token: number,
    row: QuestProgressRow?,
    reason: string?,
}

export type QuestProgressOutcome = {
    kind: "done" | "refused" | "aborted",
    value: QuestProgressEnd?,
    reason: string?,
}

-- paint --------------------------------------------------------------------

export type TextMetrics = { width: number }

export type Gradient = {
    addColorStop: (self: Gradient, offset: number, color: string) -> (),
}

-- The context that `onPaint(ctx)` receives. It records at most 256 draw
-- operations for one frame on the 765 by 503 game image. Call methods with a
-- colon: `ctx:fillRect(0, 0, 10, 10)`. The context is valid only inside the
-- call. `fillText` takes no `maxWidth`.
export type PaintContext = {
    fillStyle: string | Gradient,
    strokeStyle: string,
    shadowColor: string,
    font: string,
    lineJoin: string,
    textAlign: string,
    textBaseline: string,
    lineWidth: number,
    shadowBlur: number,
    shadowOffsetX: number,
    shadowOffsetY: number,
    fillRect: (self: PaintContext, x: number, y: number, w: number, h: number) -> (),
    fillText: (self: PaintContext, text: string, x: number, y: number) -> (),
    measureText: (self: PaintContext, text: string) -> TextMetrics,
    beginPath: (self: PaintContext) -> (),
    closePath: (self: PaintContext) -> (),
    moveTo: (self: PaintContext, x: number, y: number) -> (),
    lineTo: (self: PaintContext, x: number, y: number) -> (),
    quadraticCurveTo: (self: PaintContext, cpx: number, cpy: number, x: number, y: number) -> (),
    arc: (self: PaintContext, x: number, y: number, radius: number, startAngle: number, endAngle: number, counterclockwise: boolean?) -> (),
    fill: (self: PaintContext) -> (),
    stroke: (self: PaintContext) -> (),
    clip: (self: PaintContext) -> (),
    save: (self: PaintContext) -> (),
    restore: (self: PaintContext) -> (),
    createLinearGradient: (self: PaintContext, x0: number, y0: number, x1: number, y1: number) -> Gradient,
    createRadialGradient: (self: PaintContext, x0: number, y0: number, r0: number, x1: number, y1: number, r1: number) -> Gradient,
}

-- entries ------------------------------------------------------------------

-- The table that a script returns. `tick` is required. `onPaint` draws one
-- frame. `onStop` runs once when the script ends, with the stop reason.
-- `api` is also a global, so `onPaint` and `onStop` can use it.
export type ScriptEntry = {
    tick: (api: Api) -> (),
    onPaint: ((ctx: PaintContext) -> ())?,
    onStop: ((reason: string) -> ())?,
}

-- api ----------------------------------------------------------------------

declare api: {
    --- Write one log line: the values joined by tabs. (Output)
    log: (...any) -> (),
    --- Stop the script. The entry ends at the call; onStop runs once. (Request)
    stop: (reason: string?) -> never,
    --- Suspend tick until the next unheld game tick. (Await)
    wait: () -> (),
    settings: {
        --- The text setting `key`, or `fallback` ("" when absent). (Request)
        str: (key: string, fallback: string?) -> string,
        --- The number setting `key`, or `fallback` (0 when absent). (Request)
        num: (key: string, fallback: number?) -> number,
        --- The boolean setting `key`, or `fallback` (false when absent). (Request)
        bool: (key: string, fallback: boolean?) -> boolean,
    },
    status: {
        --- The game tick of the lent input. (Read)
        tick: () -> number,
        --- Whether the client is in game. (Read)
        inGame: () -> boolean,
        --- The player's x, z and level, or nil when the host has no tile. (Read)
        tile: () -> (number?, number?, number?),
        --- The run energy. (Read)
        runEnergy: () -> number,
        --- Whether run is on. (Read)
        runEnabled: () -> boolean,
        --- Whether the player is in combat. (Read)
        inCombat: () -> boolean,
        --- Whether the player animates. (Read)
        animating: () -> boolean,
        --- The player's name, or nil. (Read)
        name: () -> string?,
    },
    walk: {
        --- Walk to the exact tile and wait for the walk outcome. (Await)
        to: (x: number, z: number, level: number, options: WalkOptions?) -> WalkResult,
        --- Walk to within `radius` tiles and wait for the walk outcome. (Await)
        near: (x: number, z: number, level: number, radius: number, options: WalkOptions?) -> WalkResult,
        --- Queue a walk to the nearest packed bank stand. (Request)
        nearestBank: () -> (),
    },
    bank: {
        --- Open the nearest world bank; return its typed seat status and value. (Await)
        openNearest: () -> BankResult,
        --- Deposit all bankable backpack items; return the typed seat status and value. (Await)
        depositAll: () -> BankResult,
        --- Close the current bank; return the typed seat status and value. (Await)
        close: () -> BankResult,
        --- Queue opening the exact bank booth loc at `(x, z, level)` with `id`. (Request)
        openBooth: (x: number, z: number, level: number, id: number, options: OpenBoothOptions?) -> (),
        --- Queue opening a packed bank stand (`booth` or `npc`) at `(x, z, level)`. (Request)
        openStand: (x: number, z: number, level: number, kind: string, options: OpenStandOptions?) -> (),
        --- Queue closing the open bank interface immediately. (Request)
        closeNow: () -> (),
        --- Queue toggling bank withdraw-as-note mode. (Request)
        setNoteMode: (on: boolean) -> (),
        --- Queue depositing all copies of the named item into the open bank. (Request)
        deposit: (name: string) -> (),
        --- Queue withdrawing the named item from the open bank with `action`. (Request)
        withdraw: (name: string, action: string) -> (),
        --- Queue a host-owned counted bank withdrawal for the current bank generation. (Request)
        withdrawX: (request: WithdrawXRequest) -> (),
        --- The current bank session generation from the lent snapshot. (Read)
        generation: () -> number,
    },
    dialog: {
        --- Queue the current dialog's Continue option. (Request)
        continue: () -> (),
        --- Queue a 1-based dialog option. (Request)
        answer: (option: number) -> (),
    },
    equipment: {
        --- Queue removing the named worn item. (Request)
        remove: (name: string) -> (),
        --- Queue wearing the named carried item. (Request)
        wear: (name: string) -> (),
    },
    interface: {
        --- Queue clicking the identified interface component. (Request)
        button: (component_id: number) -> (),
    },
    inventory: {
        --- Queue the named carried item's 1-based option. (Request)
        option: (name: string, option: number) -> (),
        --- Queue using one carried item on another by name. (Request)
        useOn: (source: string, target: string) -> (),
    },
    loc: {
        --- Queue the loc option at the exact tile, without an id fallback. (Request)
        interact: (tile: Tile, action: string) -> (),
    },
    interact: {
        --- Queue one NPC action by name, optionally at an exact index. (Request)
        npc: (name: string, action: string, index: number?) -> (),
        --- Queue one loc action at the exact tile and loc id. (Request)
        loc: (x: number, z: number, level: number, action: string, id: number) -> (),
        --- Queue one ground-item action at the exact tile and item id. (Request)
        obj: (x: number, z: number, level: number, action: string, id: number) -> (),
        --- Queue one player action by name. (Request)
        player: (name: string, action: string) -> (),
        --- Queue one held-item action by exact item slot and target id. (Request)
        held: (name: string, action: string, slot: number, target_item_id: number) -> (),
        --- Queue one held-item action by item name and action label. (Request)
        heldByName: (name: string, action: string) -> (),
    },
    route: {
        --- Queue one route-inspection query between `from` and `to`. (Request)
        inspect: (from: Tile, to: Tile, options: InspectRouteOptions?) -> (),
        --- Read the latest route-inspection result from the lent snapshot, or nil. (Read)
        result: () -> RouteInspectResult?,
    },
    puzzle: {
        --- Queue sliding one piece on the open puzzle board under `generation`. (Request)
        move: (id: number, slot: number, component: number, generation: number) -> (),
        --- Read the open puzzle board from the lent snapshot, or nil. (Read)
        board: () -> PuzzleBoard?,
    },
    combat: {
        --- Run one native combat fight and return its typed outcome. (Await)
        fight: (request: CombatRequest) -> CombatOutcome,
    },
    gather: {
        --- Run one native Gatherer session and return its typed outcome. (Await)
        run: (settings: GatherSettings?) -> GatherOutcome,
    },
    --- Read one quest-tab row status from the lent snapshot. (Read)
    questStatus: (name: string) -> QuestStatusResult,
    --- Read the index of released quest Paths. (Read)
    questPaths: () -> { QuestPathRow },
    --- Run one owned quest progress read and return its typed progress page. (Await)
    questProgress: (quest: string) -> QuestProgressOutcome,
}

export type Api = typeof(api)
