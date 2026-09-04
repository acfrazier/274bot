// Generated from host verb tables — do not edit by hand.
// Regen: cargo test -p script --test host_js regen_host_js -- --ignored
// NativeTick Load is 0.2.5. Not a clone of rs2b0t-api.

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

export interface NearestBooth {
  x: number;
  z: number;
  level: number;
  name: string;
  op: string;
}

export interface ChatOption {
  text: string;
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
  com_id: number;
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
  hold: boolean;
  ours: boolean;
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
  my_name: string | null;
  in_combat: boolean;
  animating: boolean;
  main_modal_id: number;
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
  | { op: 'open-booth'; x?: number; z?: number; level?: number}
  | { op: 'open-stand'; x: number; z: number; level: number; kind: string; name?: string | null; stand_op?: number | null; choose?: string | null}
  | { op: 'walk'; x: number; z: number; level: number; allow_teleports?: boolean}
  | { op: 'walk-to'; x: number; z: number; level: number}
  | { op: 'deposit'; name: string}
  | { op: 'withdraw'; name: string; action: string}
  | { op: 'held'; name: string; action: string}
  | { op: 'close'}
  | { op: 'npc'; name: string; action: string; index?: number | null}
  | { op: 'loc'; x: number; z: number; level: number; action: string}
  | { op: 'obj'; x: number; z: number; level: number; name?: string | null; action: string}
  | { op: 'player'; name: string; action: string}
  | { op: 'use-on'; name: string; kind: string; target_name?: string | null; x: number; z: number; level: number; index?: number | null}
  | { op: 'use-widget-on'; component_id: number; kind: string; target_name?: string | null; x: number; z: number; level: number; index?: number | null}
  | { op: 'continue'}
  | { op: 'answer'; option: number}
  | { op: 'answer-count'; value: number}
  | { op: 'if-button'; component_id: number}
  | { op: 'close-modal'}
  | { op: 'side-tab'; tab: number}
  | { op: 'wear'; name: string}
  | { op: 'set-run'; on: boolean}
  | { op: 'set-retaliate'; on: boolean}
  | { op: 'set-note-mode'; on: boolean}
  | { op: 'set-camera-yaw'; yaw: number};
