// Generated from host verb tables — do not edit by hand.
// Regen: cargo test -p script --test host_js regen_host_js -- --ignored
// NativeTick Load is 0.2.5. JS API v2 is NativeApi (explicit export const apiVersion = 2).
// Not a clone of rs2b0t-api.

/** Readonly raw i32 flags. at requires a finite integer index in [0, length). Negative and non-integers are undefined. */
export interface CollisionFlagsView {
  /** width*height, or 0 when !available. */
  length: number;
  at: (index: number) => number | undefined;
}

/** Posted one-plane collision identity. Retained views stay historical after a later post. */
export interface CollisionView {
  available: boolean;
  base_x: number;
  base_z: number;
  level: number;
  width: number;
  height: number;
  flags: CollisionFlagsView;
}

/** Absolute world tile `{x, z, level}`. */
export interface WorldTile {
  x: number;
  z: number;
  level: number;
}

/** One inventory/bank/equipment/trade row from the posted snapshot. */
export interface ItemRow {
  name: string | null;
  count: number;
  id: number;
  ops: string[];
  noted: boolean;
  cert: number;
  component_id: number;
  /** Container slot when the host has exact item identity. */
  slot?: number;
}

export interface StatRow {
  index: number;
  name: string;
  xp: number;
  base: number;
  effective: number;
}

/** One npc/loc/player/ground row from the posted snapshot. */
export interface SceneEntity {
  index: number;
  id: number;
  name: string | null;
  x: number;
  z: number;
  level: number;
  distance: number;
  health: number;
  max_health: number;
  in_combat: boolean;
  animating: boolean;
  actions: string[];
  reachable: boolean;
  reachable_adj: boolean;
  combat_level: number;
  /** 0 none, 1 npc, 2 player. */
  target_kind: number;
  /** -1 when not facing anyone. */
  target_index: number;
  /** NPC footprint in tiles. <1 means observation absent (old buffer / non-NPC row). Packet-time at last NPC_INFO. */
  size: number;
  /** Path-head network SW x. Valid world 0 is not absence when size>=1. */
  nx: number;
  /** Path-head network SW z. */
  nz: number;
}

/** A packed bank stand (booth loc or teller npc). */
export interface BankStand {
  name: string;
  x: number;
  z: number;
  level: number;
  kind: 'booth' | 'npc';
  op: number;
  choose: string | null;
}

/** Packed bank dest/readiness row from the host. */
export interface BankApproach {
  loc_id: number;
  x: number;
  z: number;
  level: number;
  can_operate: boolean;
  dest_ok: boolean;
  dest_x: number;
  dest_z: number;
  dest_level: number;
}

export interface NearestBooth {
  x: number;
  z: number;
  level: number;
  id: number;
  name: string;
  op: string;
}

export interface ChatOption {
  text: string;
}

/** Native on/off component identity for one toggle. */
export interface ToggleControls {
  onComId: number;
  offComId: number;
}

/** One native quest-tab row with its Rust-resolved coarse status. */
export interface QuestStatusRow {
  name: string;
  status: 'notStarted' | 'inProgress' | 'complete' | 'unknown';
  /** The row's walked TYPE_TEXT id (its click target). Omitted on an old buffer: the row has no target, and 0 is a real id. */
  component_id?: number;
}

/** Posted reach window metadata. Packed bits are not materialized; use api.walkable / canStep / canReach. */
export interface ReachQueryView {
  available: boolean;
  base_x: number;
  base_z: number;
  level: number;
  width: number;
  height: number;
}

export interface VarpRow {
  index: number;
  value: number;
}

export interface CombatStyleButton {
  mode: number;
  label: string;
  component_id: number;
}

export interface SideTabIface {
  index: number;
  id: number;
}

export interface ChatLine {
  seq: number;
  text: string;
}

export interface MakeButton {
  qty: number;
  comId: number;
}

export interface MakeProduct {
  object_id: number;
  name: string;
  buttons: MakeButton[];
}

/**
 * Walk/nav opt-ins for packed nav (`Traveller` / `ScriptWalkArm`).
 * All default off.
 */
export interface FindOptions {
  /** Allow packed-nav teleports (default off). */
  allow_teleports?: boolean;
  /**
   * Allow routes that enter or land in the wilderness zone.
   * Default off — nav refuses wilderness tiles without this opt-in.
   */
  allow_wilderness?: boolean;
  /** Latch a host BankBudget session when true. */
  allow_bank_fetch?: boolean;
}

/** Orbit camera read from the posted snapshot (`camera_yaw` / `camera_pitch`). */
export interface Camera {
  /** Follow-camera yaw. */
  yaw: number;
  /** Follow-camera pitch. */
  pitch: number;
  /** Orbit target yaw (`CameraView::orbit_yaw`). */
  orbit_yaw: number;
}

/** One shop stock row when `shop_open` is true. */
export interface ShopStockRow {
  name: string;
  count: number;
}

/** The PLAYER_INFO snapshot posted into an isolate. Delta posts omit unchanged fields. */
export interface Snapshot {
  /** Always carried; other fields are delta-posted. */
  tick: number;
  here: WorldTile | null;
  ingame: boolean;
  inv: ItemRow[];
  inv_size: number;
  stats: StatRow[];
  booths: WorldTile[];
  nearest_booth: NearestBooth | null;
  banks: BankStand[];
  bank: ItemRow[];
  bank_side: ItemRow[];
  bank_open: boolean;
  bank_loaded: boolean;
  bank_generation: number;
  count_dialog_open: boolean;
  withdraw_x_result_seq: number;
  withdraw_x_result: boolean;
  withdraw_load_result_seq: number;
  withdraw_load_result: boolean;
  bank_op_result_seq: number;
  bank_op_result: boolean;
  bank_note_on: number;
  bank_note_off: number;
  /** 2 = 3D ready. */
  scene_state: number;
  weight: number;
  /** Orbit camera yaw. */
  camera_yaw: number;
  /** Orbit camera pitch. */
  camera_pitch: number;
  /** Whether packed nav last armed with `allow_teleports`. */
  teleports_enabled: boolean;
  self_slot: number;
  trade_offer_open: boolean;
  trade_confirm_open: boolean;
  trade_partner: string | null;
  trade_mine: ItemRow[];
  trade_theirs: ItemRow[];
  trade_side: ItemRow[];
  trade_accept_id: number;
  trade_decline_id: number;
  shop_open: boolean;
  shop_stock: ShopStockRow[];
  reach: ReachQueryView;
  hold: boolean;
  ours: boolean;
  /** Packet-time NPC_INFO rows. Copy if retaining past this tick. size<1 is unavailable, not a synthetic 1. */
  npcs: SceneEntity[];
  locs: SceneEntity[];
  players: SceneEntity[];
  ground: SceneEntity[];
  equipment: ItemRow[];
  chat_open: boolean;
  chat_continue: boolean;
  chat_text: string | null;
  chat_options: ChatOption[];
  side_tab: number;
  varps: VarpRow[];
  combat_styles: CombatStyleButton[];
  run_energy: number;
  run_enabled: boolean;
  retaliate_enabled: boolean;
  retaliate_controls: ToggleControls | null;
  quest_statuses: QuestStatusRow[] | null;
  npc_boxes: { index: number; points: { x: number; y: number }[] }[] | null;
  my_name: string | null;
  in_combat: boolean;
  animating: boolean;
  main_modal_id: number;
  /** The main modal's paired TYPE_TEXT walk: `root` is the same integer `main_modal_id` carries, `texts` is its walk order with colour tags intact. `{ root: -1, texts: [] }` is an observed closed modal. Omitted: the post did not carry the pair (keep the last one). */
  main_modal_texts?: { root: number; texts: string[] };
  chat_modal_id: number;
  make_products: MakeProduct[];
  side_tab_ifaces: SideTabIface[];
  spell_buttons: CombatStyleButton[];
  chat_lines: ChatLine[];
}

/** The per-tick host handle (`__rs2b0t_host`) Compat scripts queue onto. */
export interface HostHandle {
  tick: number;
  snapshot: Snapshot;
  /** Interact queue drained by the host after each tick. */
  interact: InteractReq[];
  /** Guardian hold gate (read-only). */
  hold: boolean;
  /** Guardian claim (read-only). */
  ours: boolean;
}

/** One interact queued on the host handle; dispatched through the slot Driver. */
export type InteractReq =
  | { op: 'open-booth'; x: number; z: number; level: number; id: number; name?: string | null; action?: string | null}
  | { op: 'open-stand'; x: number; z: number; level: number; kind: string; name?: string | null; stand_op?: number | null; choose?: string | null}
  | { op: 'walk'; x: number; z: number; level: number; allow_teleports?: boolean; allow_wilderness?: boolean; allow_bank_fetch?: boolean; request_id?: number}
  | { op: 'walk-near'; x: number; z: number; level: number; radius: number; allow_teleports?: boolean; allow_wilderness?: boolean; allow_bank_fetch?: boolean; request_id?: number}
  | { op: 'walk-nearest-bank'}
  | { op: 'inspect-route'; x: number; z: number; level: number; from_x: number; from_z: number; from_level: number; allow_teleports?: boolean; allow_wilderness?: boolean; allow_bank_fetch?: boolean; avoid?: Array<{ minX: number; maxX: number; minZ: number; maxZ: number; level?: number }>; request_id?: number}
  | { op: 'walk-to'; x: number; z: number; level: number}
  | { op: 'deposit'; name: string}
  | { op: 'withdraw'; name: string; action: string}
  | { op: 'withdraw-x'; name: string; count: number; bank_item_id: number; lands_as_id: number; action: string; bank_generation: number}
  | { op: 'withdraw-load'; name: string; bank_generation: number}
  | { op: 'held'; name: string; action: string}
  | { op: 'inv-button'; id: number; slot: number; component: number; operation: number; bank_generation?: number}
  | { op: 'puzzle-move'; id: number; slot: number; component: number; generation: number}
  | { op: 'shop-button'; kind: string; name: string; id: number; slot: number; component: number; chunk: number}
  | { op: 'make-panel'; id: number; slot: number; component: number; operation: number}
  | { op: 'close'}
  | { op: 'npc'; name: string; action: string; index?: number | null}
  | { op: 'loc'; x: number; z: number; level: number; action: string; id?: number | null}
  | { op: 'obj'; x: number; z: number; level: number; name?: string | null; action: string}
  | { op: 'player'; name: string; action: string}
  | { op: 'use-on'; name: string; kind: string; target_name?: string | null; x: number; z: number; level: number; index?: number | null; source_item_id?: number | null; source_item_slot?: number | null; target_item_id?: number | null; target_item_slot?: number | null}
  | { op: 'use-widget-on'; component_id: number; kind: string; target_name?: string | null; x: number; z: number; level: number; index?: number | null}
  | { op: 'continue'}
  | { op: 'answer'; option: number}
  | { op: 'answer-count'; value: number}
  | { op: 'if-button'; component_id: number}
  | { op: 'close-modal'}
  | { op: 'side-tab'; tab: number}
  | { op: 'wear'; name: string}
  | { op: 'unequip'; name: string}
  | { op: 'set-run'; on: boolean}
  | { op: 'set-retaliate'; on: boolean}
  | { op: 'set-note-mode'; on: boolean}
  | { op: 'set-camera-yaw'; yaw: number}
  | { op: 'note-progress'}
  | { op: 'loop-settled'}
  | { op: 'wait-enqueued'}
  | { op: 'wait-settled'}
  | { op: 'recovery-anchor'; x: number; z: number; level: number}
  | { op: 'recovery-anchor-none'}
  | { op: 'key'; down: boolean; key: string; code?: string}
  | { op: 'mouse'; down: boolean; x: number; y: number; button?: number};

/** Host-owned snapshot fields exposed on NativeApi.snapshot. Delta posts omit unchanged fields. Do not mutate; valid until the next tick. */
export interface NativeSnapshot {
  ingame: boolean;
  here: WorldTile | null;
  inv: ItemRow[];
  /** 0 while the inv tab is tutorial-locked. */
  inv_size: number;
  stats: StatRow[];
  bank: ItemRow[];
  bank_side: ItemRow[];
  bank_open: boolean;
  bank_loaded: boolean;
  /** Pass this into withdraw-load / withdraw-x. A changed generation is stale, not exhaustion. */
  bank_generation: number;
  banks: BankStand[];
  nearest_booth: NearestBooth | null;
  bank_approaches: BankApproach[];
  count_dialog_open: boolean;
  /** The posted piece container. component_id -1 is a closed board, not a missing one. Rows are that widget's own sparse ItemRows. */
  puzzle_board: { component_id: number; size: number; items: ItemRow[] };
  /** Pass this into puzzle-move. A changed generation is stale, not a solved board. */
  puzzle_board_generation: number;
  withdraw_x_result_seq: number;
  withdraw_x_result: boolean;
  withdraw_load_result_seq: number;
  withdraw_load_result: boolean;
  bank_op_result_seq: number;
  bank_op_result: boolean;
  walk_outcome_seq: number;
  walk_outcome_generation: number;
  walk_outcome_failed: boolean;
  walk_outcome_x: number;
  walk_outcome_z: number;
  walk_outcome_level: number;
  walk_outcome_radius: number;
  walk_outcome_allow_teleports: boolean;
  walk_outcome_request_id: number;
  route_inspect_seq: number;
  route_inspect_generation: number;
  route_inspect_request_id: number;
  route_inspect_ok: boolean;
  route_inspect_reason: string;
  route_inspect_bank_planned: boolean;
  route_inspect_ticks: number;
  route_inspect_hops: Array<{ kind: string; locId: number; locName: string; action: string; option: number; from: WorldTile; to: WorldTile; ticks: number }>;
  route_inspect_prev_seq: number;
  route_inspect_prev_generation: number;
  route_inspect_prev_request_id: number;
  route_inspect_prev_ok: boolean;
  route_inspect_prev_reason: string;
  route_inspect_prev_bank_planned: boolean;
  route_inspect_prev_ticks: number;
  route_inspect_prev_hops: Array<{ kind: string; locId: number; locName: string; action: string; option: number; from: WorldTile; to: WorldTile; ticks: number }>;
  route_inspect_running_id: number;
  route_inspect_pending_id: number;
  route_inspect_accepted_id: number;
  route_inspect_replaced_id: number;
  route_inspect_replaced_prev_id: number;
  /** Latest registered token the host could not reserve. Not a ring terminal. */
  route_inspect_refused_id: number;
  route_inspect_refused_id_2: number;
  route_inspect_refused_id_3: number;
  /** Host unobserved obligation count. Advisory; may lag the next drain. */
  route_inspect_unobserved: number;
  /** One current-plane raw i32 grid. flags.at is indexed lx*height+lz. 0 is clear, not absent. */
  collision: CollisionView;
  /** Packet-time NPC_INFO rows. Copy if retaining past this tick. size<1 is unavailable, not a synthetic 1. */
  npcs: SceneEntity[];
  /** 0 none, 1 npc, 2 player. Packet-time local face. */
  self_target_kind: number;
  /** -1 when kind is 0. 0 is a legal NPC index. */
  self_target_index: number;
}

/** Typed settings access over the per-identity host bag. */
export interface NativeSettings {
  str(name: string, fallback?: string): string;
  num(name: string, fallback?: number): number;
  bool(name: string, fallback?: boolean): boolean;
}

/** Recording paint frame. end() publishes the host overlay. */
export interface NativePaintFrame {
  title(text: string): NativePaintFrame;
  row(...cols: Array<string | number>): NativePaintFrame;
  gap(): NativePaintFrame;
  end(): void;
}

export interface NativePaint {
  begin(opts?: { accent?: string }): NativePaintFrame;
}

/** Supported v2 request ops. Unknown op throws `not impl: request.<op>`. Returns void; completion is later snapshot seqs. */
export type NativeOp =
  | { op: 'held'; name: string; action: string}
  | { op: 'open-booth'; x: number; z: number; level: number; id: number; name?: string; action?: string}
  | { op: 'open-stand'; x: number; z: number; level: number; kind: string; name?: string; stand_op?: number; choose?: string}
  | { op: 'close'}
  | { op: 'set-note-mode'; on: boolean}
  | { op: 'deposit'; name: string}
  | { op: 'withdraw'; name: string; action: string}
  | { op: 'withdraw-load'; name: string; bank_generation: number}
  | { op: 'withdraw-x'; name: string; count: number; bank_item_id: number; lands_as_id: number; action: string; bank_generation: number}
  | { op: 'puzzle-move'; id: number; slot: number; component: number; generation: number}
  | { op: 'walk'; x: number; z: number; level: number; allow_teleports?: boolean; allow_wilderness?: boolean; allow_bank_fetch?: boolean; request_id?: number}
  | { op: 'walk-near'; x: number; z: number; level: number; radius: number; allow_teleports?: boolean; allow_wilderness?: boolean; allow_bank_fetch?: boolean; request_id?: number}
  | { op: 'walk-nearest-bank'}
  | { op: 'inspect-route'; from: WorldTile; to: WorldTile; allow_teleports?: boolean; allow_wilderness?: boolean; allow_bank_fetch?: boolean; avoid?: Array<{ minX: number; maxX: number; minZ: number; maxZ: number; level?: number }>; request_id?: number};

export type HelperResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: string };

/** prayerClear walk counts. timed_out may be nonzero; that is not all-off success. */
export interface PrayerClearCounts {
  clicked: number;
  timed_out: number;
}

/** Caller-supplied carry row. Omitted qty defaults to 1 on loadout helpers. */
export interface LoadoutCarry {
  item: string;
  qty?: number;
}

/** Caller-supplied loadout. Not a NativeSnapshot field. */
export interface LoadoutInput {
  name?: string;
  worn?: Record<string, string>;
  carry?: LoadoutCarry[];
  unassigned?: string[];
}

/** Recommended flask form. short is present on plannedPotions values. */
export interface PotionPlan {
  skill: string;
  short?: string;
  flask: string;
  doses: string[];
  want: number;
}

export interface RangeLoadout {
  weapon: string;
  projectile: string;
  thrown: boolean;
}

export interface GatherId {
  alias: string;
  id: number;
}

/** Loc-resource method row. publication is null when the landed row has none. */
export interface GatherLocResourceRow {
  skill: string;
  table: string;
  resource_key: string;
  loc_ids: GatherId[];
  empty_ids: GatherId[];
  output: GatherId | null;
  level: number;
  qualification: string;
  partial_sides: string[];
  missing_transform: string[];
  publication: string | null;
}

/** Fishing method row. No loc ids and no spawn tile. */
export interface GatherFishingRow {
  skill: string;
  category: string;
  primary_op: string;
  pair_op: string | null;
  level: null;
  output: null;
  qualification: string;
  partial_sides: string[];
}

export type GatherMethodRow = GatherLocResourceRow | GatherFishingRow;

/** Coverage beside method rows. Not a resource hit. */
export interface GatherCoverageRecord {
  class: string;
  table?: string;
  resource_key?: string;
  alias?: string;
  on_revision?: number;
  other_pin_id?: number;
  copied?: boolean;
  reason: string;
}

/** One published-woods world placement. `plane` is the stored level, not the query's `level`. */
export interface GatherPlacementRow {
  loc_id: number;
  x: number;
  z: number;
  plane: number;
}

/** One level's published-woods placements. `resource_ids` is the methods loc-id set, not the hit list. */
export interface GatherPlacementResult {
  rows: GatherPlacementRow[];
  truncated: boolean;
  resource_ids: GatherId[];
  qualification: string;
}

export interface QuestSkillGate {
  skill: string;
  level: number;
}

export interface QuestItemAlias {
  alias: string;
  quantity: number | null;
  kind: string;
}

/** Requirements on the identity row. Qualification stays partial. */
export interface QuestRequirements {
  qualification: string;
  skills: QuestSkillGate[];
  items: QuestItemAlias[];
  empty_must_have: boolean;
  unknown_as_satisfied: boolean;
}

/** Landed identity row. complete is the constant, not a range. */
export interface QuestIdentityRow {
  id: string;
  component: string;
  display: string;
  varp: string;
  varp_id: number;
  complete: number;
  quest_points: number;
  unknown_sides: string[];
  requirements: QuestRequirements;
}

/** One landed trail membership row. `access` is present only on the one bounded inclusion. */
export interface ClueRow {
  alias: string;
  id: number;
  role: string;
  /** Raw param lines, in file order. Values are never coerced. */
  params: Array<{ key: string; value: string }>;
  access?: 'constrained';
}

/** Caller numbers only. Absent fields default; a present wrong type is invalid-args. */
export interface PackPlanInput {
  hostWant?: number;
  heldFood?: number;
  freeSlots?: number;
  reserveSlots?: number;
  perCast?: number;
  weaponName?: string;
  weaponInBackpack?: boolean;
  weaponEquipped?: boolean;
  casketAlias?: string;
}

/** `runeTarget`, `weaponNeeded`, and `rewardSlots` only when the caller asked for them. */
export interface PackPlanTargets {
  coordToolSlots: 3;
  teleportCasts: 20;
  food: number;
  runeTarget?: number;
  weaponNeeded?: boolean;
  rewardSlots?: number;
}

/** Caller-supplied one-level box. Not a plane and not a radius form. */
export interface SceneRegionInput {
  min_x: number;
  min_z: number;
  max_x: number;
  max_z: number;
  level: number;
}

/** Copied collision bounds. Collision flags stay on the snapshot page. */
export interface SceneBounds {
  available: boolean;
  base_x: number;
  base_z: number;
  level: number;
  width: number;
  height: number;
}

/** Historical posted row copy. Not the live entity: no index, name, or nested tile. */
export interface SceneProjectionRow {
  id: number;
  x: number;
  z: number;
  level: number;
  actions: string[];
}

/** One posted scene vector. as_of_sequence is snapshot.tick. */
export interface SceneProjection {
  as_of_sequence: number;
  scene: SceneBounds;
  rows: SceneProjectionRow[];
  truncated: boolean;
}

/** Public JS API v2 handle. Explicit `export const apiVersion = 2` only. */
export interface NativeApi {
  readonly tick: number;
  /** Host-owned, delta-merged. Read only. Do not mutate; copy if retaining. */
  readonly snapshot: NativeSnapshot;
  readonly settings: NativeSettings;
  log(message: string): void;
  stop(reason?: string): void;
  readonly paint: NativePaint;
  request(op: NativeOp): void;
  /** Isolate-owned inspect token. Pair with request inspect-route request_id, or query inspectSettled/inspectValue. Caller-invented ids never consume host jobs; 0 is snapshot-only preview. */
  inspectBegin(opts: { from: WorldTile; to: WorldTile; allow_teleports?: boolean; allow_wilderness?: boolean; allow_bank_fetch?: boolean; avoid?: Array<{ minX: number; maxX: number; minZ: number; maxZ: number; level?: number }>; timeout_ms?: number }): number;
  inspectSettled(token: number): boolean;
  inspectValue(token: number): { ok: boolean; reason: string; bankPlanned: boolean; ticks: number; hops: Array<{ kind: string; locId: number; locName: string; action: string; option: number; from: WorldTile; to: WorldTile; ticks: number }>; request_id: number } | null;
  prayerPoints(): HelperResult<number>;
  prayerMax(): HelperResult<number>;
  prayerFull(): HelperResult<boolean>;
  prayerKnown(input: { name: string }): HelperResult<boolean>;
  prayerAvailable(input: { name: string }): HelperResult<boolean>;
  prayerActive(input: { name: string }): HelperResult<boolean>;
  /** Named async private-lifecycle exception. Final HelperResult only; callers never see Step.
   * Before: a second Set/Clear overwrote the private pump and could hang the first Promise; a sync tick that did not return that Promise did not advance it.
   * After: a second Set/Clear while one operation is already admitted returns `{ok:false, error:'busy'}` without begin/click. The admitted operation keeps ownership and must settle. Sequential `await` is the preferred example; fire-and-forget still progresses on later eligible NativeTicks. Additional public error: `busy`.
   */
  prayerSet(input: { name: string; on: boolean }): Promise<HelperResult<boolean>>;
  /** Completes the 15-row walk. timed_out may be nonzero; LIVE later requires all off. Same busy refuse as prayerSet. */
  prayerClear(): Promise<HelperResult<PrayerClearCounts>>;
  foodCount(input: { items: ItemRow[]; foodName: string }): HelperResult<number>;
  foodHealAmount(input: { foodName: string }): HelperResult<number>;
  combatKeepNames(input: { food: string; style?: string; spell?: string; ammo?: string; weapon?: string; extra?: string[] }): HelperResult<string[]>;
  runesPerCast(input: { spellName: string; wielded: string[] }): HelperResult<Array<{ rune: string; count: number }> | null>;
  escapeRunesFor(input: { id: string }): HelperResult<{ runes: Array<{ rune: string; count: number }>; level: number; label: string }>;
  /** Sync fact read. Omitted input or `{}` omits the skill. Not a Promise and not a request op. */
  gatherMethods(input?: { skill?: string }): HelperResult<{ rows: GatherMethodRow[]; coverage: GatherCoverageRecord[] }>;
  /** Sync resource-key read. Zero matches is unknown-resource, not an empty rows list. */
  gatherResource(input: { name: string }): HelperResult<{ rows: GatherLocResourceRow[] }>;
  /** Sync published-woods world placements inside a required one-level region. Omitted `region` is `missing-region`. A mining `resource` is a marked unknown empty, never `family-unavailable:gather_placements`. Not a Promise and not a request op. */
  gatherPlacements(input: { resource: string; region: SceneRegionInput; limit: number }): HelperResult<GatherPlacementResult>;
  /** Sync fact read. Exactly one of name or id, and it must be a string. Not a Promise and not a request op. */
  questIdentity(input: { name: string } | { id: string }): HelperResult<QuestIdentityRow>;
  /** Sync seed-id requirements read. A name field is not a key. Not a Promise and not a request op. */
  questPrereqs(input: { id: string; name?: string }): HelperResult<QuestRequirements>;
  /** Sync landed trail-row read, pure pack arithmetic, pure hard-kit status, the pure keep predicate over caller facts, and the clue machine's begin / one awaited run / retry. `run` is the only Promise and none is a request op; `packPlan`, `hardKit`, and `keep` read no snapshot, inventory, or family, and `retry` is the machine's latch clear — never a connection-boundary reset and never a token abort. */
  clue: { row(input: { id: number } | { alias: string }): HelperResult<ClueRow>; heldStep(): HelperResult<ClueRow>; packPlan(input: PackPlanInput): HelperResult<PackPlanTargets>; hardKit(input: { attack: number; lostCity: boolean; items: { id: number; count: number }[] }): HelperResult<{ status: 'ready' }>; keep(input: { name: string; extra?: string[] }): HelperResult<{ keep: boolean }>; begin(input?: object): HelperResult<{ token: number }>; run(input: { token: number }, hooks?: ClueHooks): Promise<ClueOutcome>; retry(): HelperResult<{ cleared: true }> };
  /** Sync posted-loc copy. Historical copy, not live. Not a Promise and not a request op. */
  sceneLocs(input: { ids: number[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>;
  /** Sync posted-npc copy. actions is required: omitted is not match-any. Historical copy, not live. Not a Promise and not a request op. */
  sceneNpcs(input: { types: number[]; actions: string[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>;
  /** Sync posted-tab status copy. A missing page is snapshot-unavailable and a null tab is quest-tab-unbound. Not a Promise and not a request op. */
  questStatus(input: { name: string }): HelperResult<{ status: 'notStarted' | 'inProgress' | 'complete' | 'unknown'; as_of_sequence: number }>;
  /** Sync owned-root begin. One `if-button` per token on the posted row id, enqueued synchronously after a generation check. Not a Promise and not a request op; a refusal is `{ ok: false, error }`. */
  questJournalBegin(input: { name: string }): HelperResult<{ token: number }>;
  /** Sync owned-root next. Not-done is `{ pending: true }` (no `ok` field) — not empty lines. Not a Promise. */
  questJournalNext(input: { token: number }): HelperResult<{ lines: string[]; root: number; as_of_sequence: number }>;
  /** Sync owned-root close. One `close-modal` only while the latest pair is still the acquired root and texts. Not-done is `{ pending: true }` (no `ok` field). Not a Promise; never returns journal lines. */
  questJournalClose(input: { token: number }): HelperResult<{ closed: true; as_of_sequence: number }>;
  foodOf(input: { loadout: LoadoutInput | null; fallback: string }): HelperResult<string>;
  gearOf(input: { loadout: LoadoutInput | null }): HelperResult<string[]>;
  suppliesOf(input: { loadout: LoadoutInput | null }): HelperResult<Array<{ item: string; qty: number }>>;
  weaponOf(input: { loadout: LoadoutInput | null; fallback?: string | null }): HelperResult<string | null>;
  rangeLoadoutOf(input: { weapon: string; ammo: string }): HelperResult<RangeLoadout>;
  boostFaded(input: { base: number; effective: number; floor?: number }): HelperResult<boolean>;
  plannedPotions(input: { carry: Array<{ item: string; qty: number }> }): HelperResult<PotionPlan[]>;
  potionToSip(input: { plans: PotionPlan[]; held: number[]; levels: Array<{ skill: string; base: number; effective: number }> }): HelperResult<PotionPlan | null>;
  lineOfSight(input: { from: WorldTile; to: WorldTile; size?: number }): HelperResult<boolean>;
  walkable(input: { tile: WorldTile } | WorldTile): HelperResult<boolean>;
  canStep(input: { from: WorldTile; to: WorldTile }): HelperResult<boolean>;
  canReach(input: { tile: WorldTile; adjacentOk?: boolean; maxSteps?: number }): HelperResult<boolean>;
  /** Hunt Task sessions: `*Begin(site)` keeps the site with a token; `*Validate` is one synchronous read (Rust calls the hook getters and `inArea`); `*Run` awaits one Rust run (walks, ops, waits and retries are the host's). */
  fightBegin(site: HuntSite): HelperResult<{ token: number }>;
  fightValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;
  fightRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>;
  fightReset(input: HuntToken): HelperResult<null>;
  fightInterruptWatch(input: HuntToken): HelperResult<null>;
  fightBlocksLoot(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;
  holdBegin(site: HuntSite): HelperResult<{ token: number }>;
  holdValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;
  holdRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>;
  retreatBegin(site: HuntSite): HelperResult<{ token: number }>;
  retreatValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;
  retreatRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>;
  walkspotBegin(site: HuntSite): HelperResult<{ token: number }>;
  walkspotValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;
  walkspotRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>;
  enterBegin(site: HuntSite): HelperResult<{ token: number }>;
  enterValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;
  /** `value`: inside the lair. */
  enterRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;
  /** One awaited run; `value`: out of the lair. */
  leaveRun(site: HuntSite, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;
  /** One awaited run of the Jailer leg alone (corridor walk, kill, take the jail key); `value`: the jail key is held. Not v1 `acquireKey`, which composes leave, the bank stop and up to three Velrak fetches and settles a `KeyState`; v2 composes those legs from `leaveRun`, `bankRun` and `cellRun` itself. */
  keyRun(site: HuntSite, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;
  /** One awaited run through the jail cell (jail key, unlock, Velrak, back out); `value`: the site's key is held outside the cell. */
  cellRun(site: HuntSite, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;
  /** One awaited bank trip; `value`: restocked. */
  bankRun(site: HuntSite, opts?: HuntBankOptions, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;
}

export type HuntToken = { token: number };
export type HuntBox = { minX: number; maxX: number; minZ: number; maxZ: number; level: number };
/** Plain site data. Its area is `boxes`, unless `hooks.inArea` answers. */
export interface HuntSite {
  key: string;
  boxes?: HuntBox[];
  target?: string;
  alsoHunt?: string[];
  safespots?: WorldTile[];
  meleeAnchor?: WorldTile;
  approach?: WorldTile[];
  fireAtRange?: boolean;
  rangedThreat?: boolean;
  bank?: WorldTile | null;
  keyItem?: { name: string; id: number } | null;
  coins?: number | null;
  escapeTeleportId?: string | null;
  walkOut?: WorldTile | null;
  talkGate?: { npc: string; op: string; choose: string; stand: WorldTile } | null;
  feeGate?: { npc: string; op: string; coins: number; stand: WorldTile; entrance?: { locId: number; op: string } | null; paidLine: string; prepaidLine: string } | null;
  gate?: { locId: number; op: string; outside?: WorldTile | null; inside?: WorldTile | null } | null;
  exit?: { locId: number; op: string; stand: WorldTile } | null;
}
/** Bank-trip loadout (the frozen `BankOpts`), merged over the site. Rust owns the defaults: `runeCasts` 150, `runeBuffer` 300, `escapeStock` 2, `ammo` 500, `healTo` 0.9. */
export interface HuntBankOptions {
  withdrawFood?: boolean;
  wear?: string[];
  carry?: string[];
  runeCasts?: number;
  runeBuffer?: number;
  escapeStock?: number;
  ammo?: number;
  potions?: Array<{ flask: string; potion: { doses: string[] }; want: number }>;
  flasks?: Array<{ flask: string; doses: string[]; want: number }>;
  healTo?: number;
}
/** The caller's own hooks, all optional. Getters and notifications are called synchronously by Rust when a decision reads them (a returned promise is `not impl`); `eatOnce`, `armSpecial`, `sustain` and `leave` are awaited. An absent getter reads as its default. */
export interface HuntHooks {
  log?(message: string): void;
  vlog?(message: string): void;
  setStatus?(message: string): void;
  eatOnce?(): boolean | Promise<boolean>;
  armSpecial?(): void | Promise<void>;
  sustain?(): void | Promise<void>;
  /** Replaces the leave run inside `bankRun` / `keyRun` / `cellRun`; `true` when out. */
  leave?(): boolean | Promise<boolean>;
  countBurial?(): void;
  countKill?(): void;
  setSafespotIndex?(index: number): void;
  setTarget?(index: number | null): void;
  pickWeapon?(names: string[]): void;
  parkFor?(reason: string): void;
  countBankTrip?(): void;
  died?(): boolean;
  targetIdx?(): number | null;
  hpFraction?(): number;
  panicHp?(): number;
  retreatHp?(): number;
  hasFood?(): boolean;
  needEat?(): boolean;
  style?(): 'melee' | 'range' | 'mage';
  safespotIndex?(): number;
  buryBones?(): boolean;
  boneName?(): string;
  shieldReady?(): boolean;
  parked?(): boolean;
  leaveByWalk?(): boolean;
  foodName?(): string;
  foodWithdraw?(): number;
  weaponName?(): string;
  ammoName?(): string;
  spellName?(): string;
  keepExtra?(): string[];
  /** The site's area test; replaces `boxes` when present. */
  inArea?(tile: WorldTile): boolean;
}
/** One run's settlement. A hook that throws rejects the promise with that value. `aborted` reasons are the host's. A run the Rust stepper itself gives up on (a KBD site, an unexpected reply, a lost walk) settles `done` with `false` (boolean runs) or `null` (fight/hold/retreat/walkspot), the same as a run that did not get there: treat anything but `done` with `true` as failure, and for the `null` runs prove the result from the scene (the player's tile). */
export type HuntOutcome<T> =
  | { kind: 'done'; value: T }
  | { kind: 'refused'; reason: string }
  | { kind: 'aborted'; reason: 'reset' | 'superseded' | 'terminated' | 'unknown' };
/** Callbacks frozen for one clue run. Rust awaits each returned promise before advancing the machine. */
export interface ClueHooks {
  enabled?(): boolean | Promise<boolean>;
  log?(message: string): void | Promise<void>;
  setStatus?(message: string): void | Promise<void>;
}
export type ClueRunValue =
  | { kind: 'yield' | 'done' | 'dead' | 'abandon' | 'guardian-lost'; token: number }
  | { kind: 'aborted'; token: number; reason: string };
/** One clue run's settlement. A hook that throws rejects the promise with that value. */
export type ClueOutcome =
  | { kind: 'done'; value: ClueRunValue }
  | { kind: 'refused'; reason: string }
  | { kind: 'aborted'; reason: 'reset' | 'superseded' | 'terminated' | 'unknown' };
