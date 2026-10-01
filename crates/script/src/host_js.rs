//! Generated Host JS types (`host-js/index.d.ts`) from our verb tables.
//! Snapshot fields, [`crate::shim::InteractReq`] ops, and nav
//! [`crate::FindOptions`] — not rs2b0t names. NativeTick Load is 0.2.5.

use crate::gatherer::settings::schema;
use crate::gatherer::status::{FieldKind, KEYS};
use crate::SettingDef;
use std::path::{Path, PathBuf};

struct TsField {
    name: &'static str,
    ty: &'static str,
    optional: bool,
    doc: Option<&'static str>,
}

struct TsInterface {
    name: &'static str,
    doc: Option<&'static str>,
    fields: &'static [TsField],
}

struct InteractVariant {
    op: &'static str,
    fields: &'static [TsField],
}

/// `crates/script/host-js/index.d.ts`
pub fn host_js_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("host-js/index.d.ts")
}

pub fn write_host_js_dts() -> Result<(), String> {
    let path = host_js_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let src = render_host_js_dts();
    std::fs::write(&path, src).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn write_host_js_dts_to(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let src = render_host_js_dts();
    std::fs::write(path, src).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Render the living Host JS declaration file from the host verb tables.
pub fn render_host_js_dts() -> String {
    let mut out = String::from(
        "// Generated from host verb tables — do not edit by hand.\n\
         // Regen: cargo test -p script --test host_js regen_host_js -- --ignored\n\
         // NativeTick Load is 0.2.5. JS API v2 is NativeApi (explicit export const apiVersion = 2).\n\
         // Not a clone of rs2b0t-api.\n\n",
    );

    for iface in SUPPORTING_INTERFACES {
        render_interface(&mut out, iface);
        out.push('\n');
    }

    render_find_options(&mut out);
    out.push('\n');

    render_interface(
        &mut out,
        &TsInterface {
            name: "Camera",
            doc: Some(
                "Orbit camera read from the posted snapshot (`camera_yaw` / `camera_pitch`).",
            ),
            fields: &[
                TsField {
                    name: "yaw",
                    ty: "number",
                    optional: false,
                    doc: Some("Follow-camera yaw."),
                },
                TsField {
                    name: "pitch",
                    ty: "number",
                    optional: false,
                    doc: Some("Follow-camera pitch."),
                },
                TsField {
                    name: "orbit_yaw",
                    ty: "number",
                    optional: false,
                    doc: Some("Orbit target yaw (`CameraView::orbit_yaw`)."),
                },
            ],
        },
    );
    out.push('\n');

    render_interface(
        &mut out,
        &TsInterface {
            name: "ShopStockRow",
            doc: Some("One shop stock row when `shop_open` is true."),
            fields: &[
                TsField {
                    name: "name",
                    ty: "string",
                    optional: false,
                    doc: None,
                },
                TsField {
                    name: "count",
                    ty: "number",
                    optional: false,
                    doc: None,
                },
            ],
        },
    );
    out.push('\n');

    render_interface(
        &mut out,
        &TsInterface {
            name: "Snapshot",
            doc: Some(
                "The PLAYER_INFO snapshot posted into an isolate. Delta posts omit unchanged fields.",
            ),
            fields: SNAPSHOT_FIELDS,
        },
    );
    out.push('\n');

    render_interface(
        &mut out,
        &TsInterface {
            name: "HostHandle",
            doc: Some("The per-tick host handle (`__rs2b0t_host`) Compat scripts queue onto."),
            fields: &[
                TsField {
                    name: "tick",
                    ty: "number",
                    optional: false,
                    doc: None,
                },
                TsField {
                    name: "snapshot",
                    ty: "Snapshot",
                    optional: false,
                    doc: None,
                },
                TsField {
                    name: "interact",
                    ty: "InteractReq[]",
                    optional: false,
                    doc: Some("Interact queue drained by the host after each tick."),
                },
                TsField {
                    name: "hold",
                    ty: "boolean",
                    optional: false,
                    doc: Some("Guardian hold gate (read-only)."),
                },
                TsField {
                    name: "ours",
                    ty: "boolean",
                    optional: false,
                    doc: Some("Guardian claim (read-only)."),
                },
            ],
        },
    );
    out.push('\n');

    render_interact_union(&mut out);
    render_native_v2(&mut out);

    out
}

fn render_find_options(out: &mut String) {
    out.push_str("/**\n");
    out.push_str(" * Walk/nav opt-ins for packed nav (`Traveller` / `ScriptWalkArm`).\n");
    out.push_str(" * All default off.\n");
    out.push_str(" */\n");
    out.push_str("export interface FindOptions {\n");
    out.push_str("  /** Allow packed-nav teleports (default off). */\n");
    out.push_str("  allow_teleports?: boolean;\n");
    out.push_str("  /**\n");
    out.push_str("   * Allow routes that enter or land in the wilderness zone.\n");
    out.push_str("   * Default off — nav refuses wilderness tiles without this opt-in.\n");
    out.push_str("   */\n");
    out.push_str("  allow_wilderness?: boolean;\n");
    out.push_str("  /** Latch a host BankBudget session when true. */\n");
    out.push_str("  allow_bank_fetch?: boolean;\n");
    out.push_str("}\n");
}

fn render_interface(out: &mut String, iface: &TsInterface) {
    if let Some(doc) = iface.doc {
        out.push_str("/** ");
        out.push_str(doc);
        out.push_str(" */\n");
    }
    out.push_str("export interface ");
    out.push_str(iface.name);
    out.push_str(" {\n");
    for field in iface.fields {
        if let Some(doc) = field.doc {
            out.push_str("  /** ");
            out.push_str(doc);
            out.push_str(" */\n");
        }
        out.push_str("  ");
        out.push_str(field.name);
        if field.optional {
            out.push('?');
        }
        out.push_str(": ");
        out.push_str(field.ty);
        out.push_str(";\n");
    }
    out.push_str("}\n");
}

fn render_interact_union(out: &mut String) {
    out.push_str(
        "/** One interact queued on the host handle; dispatched through the slot Driver. */\n",
    );
    out.push_str("export type InteractReq =\n");
    for (i, variant) in INTERACT_VARIANTS.iter().enumerate() {
        out.push_str("  | { op: '");
        out.push_str(variant.op);
        out.push('\'');
        for field in variant.fields {
            out.push_str("; ");
            out.push_str(field.name);
            if field.optional {
                out.push('?');
            }
            out.push_str(": ");
            out.push_str(field.ty);
        }
        out.push('}');
        if i + 1 < INTERACT_VARIANTS.len() {
            out.push('\n');
        }
    }
    out.push_str(";\n");
}

fn render_native_v2(out: &mut String) {
    out.push('\n');
    out.push_str("/** Host-owned snapshot fields exposed on NativeApi.snapshot. Delta posts omit unchanged fields. Do not mutate; valid until the next tick. */\n");
    out.push_str("export interface NativeSnapshot {\n");
    for field in NATIVE_SNAPSHOT_FIELDS {
        if let Some(doc) = field.doc {
            out.push_str("  /** ");
            out.push_str(doc);
            out.push_str(" */\n");
        }
        out.push_str("  ");
        out.push_str(field.name);
        out.push_str(": ");
        out.push_str(field.ty);
        out.push_str(";\n");
    }
    out.push_str("}\n\n");
    out.push_str("/** Typed settings access over the per-identity host bag. */\n");
    out.push_str("export interface NativeSettings {\n");
    out.push_str("  str(name: string, fallback?: string): string;\n");
    out.push_str("  num(name: string, fallback?: number): number;\n");
    out.push_str("  bool(name: string, fallback?: boolean): boolean;\n");
    out.push_str("}\n\n");
    out.push_str("/** Recording paint frame. end() publishes the host overlay. */\n");
    out.push_str("export interface NativePaintFrame {\n");
    out.push_str("  title(text: string): NativePaintFrame;\n");
    out.push_str("  row(...cols: Array<string | number>): NativePaintFrame;\n");
    out.push_str("  gap(): NativePaintFrame;\n");
    out.push_str("  end(): void;\n");
    out.push_str("}\n\n");
    out.push_str("export interface NativePaint {\n");
    out.push_str("  begin(opts?: { accent?: string }): NativePaintFrame;\n");
    out.push_str("}\n\n");
    out.push_str("/** Supported v2 request ops. Unknown op throws `not impl: request.<op>`. Returns void; completion is later snapshot seqs. */\n");
    out.push_str("export type NativeOp =\n");
    for (i, variant) in NATIVE_OP_VARIANTS.iter().enumerate() {
        out.push_str("  | { op: '");
        out.push_str(variant.op);
        out.push('\'');
        for field in variant.fields {
            out.push_str("; ");
            out.push_str(field.name);
            if field.optional {
                out.push('?');
            }
            out.push_str(": ");
            out.push_str(field.ty);
        }
        out.push('}');
        if i + 1 < NATIVE_OP_VARIANTS.len() {
            out.push('\n');
        }
    }
    out.push_str(";\n\n");
    out.push_str("export type HelperResult<T> =\n");
    out.push_str("  | { ok: true; value: T }\n");
    out.push_str("  | { ok: false; error: string };\n\n");
    out.push_str(
        "/** prayerClear walk counts. timed_out may be nonzero; that is not all-off success. */\n",
    );
    out.push_str("export interface PrayerClearCounts {\n");
    out.push_str("  clicked: number;\n");
    out.push_str("  timed_out: number;\n");
    out.push_str("}\n\n");
    out.push_str(
        "/** Caller-supplied carry row. Omitted qty defaults to 1 on loadout helpers. */\n",
    );
    out.push_str("export interface LoadoutCarry {\n");
    out.push_str("  item: string;\n");
    out.push_str("  qty?: number;\n");
    out.push_str("}\n\n");
    out.push_str("/** Caller-supplied loadout. Not a NativeSnapshot field. */\n");
    out.push_str("export interface LoadoutInput {\n");
    out.push_str("  name?: string;\n");
    out.push_str("  worn?: Record<string, string>;\n");
    out.push_str("  carry?: LoadoutCarry[];\n");
    out.push_str("  unassigned?: string[];\n");
    out.push_str("}\n\n");
    out.push_str("/** Recommended flask form. short is present on plannedPotions values. */\n");
    out.push_str("export interface PotionPlan {\n");
    out.push_str("  skill: string;\n");
    out.push_str("  short?: string;\n");
    out.push_str("  flask: string;\n");
    out.push_str("  doses: string[];\n");
    out.push_str("  want: number;\n");
    out.push_str("}\n\n");
    out.push_str("export interface RangeLoadout {\n");
    out.push_str("  weapon: string;\n");
    out.push_str("  projectile: string;\n");
    out.push_str("  thrown: boolean;\n");
    out.push_str("}\n\n");
    out.push_str("export interface GatherId {\n");
    out.push_str("  alias: string;\n");
    out.push_str("  id: number;\n");
    out.push_str("}\n\n");
    out.push_str("/** Why a gathering fact is not complete: a machine code and the content spans (`file:first-last`) behind it. */\n");
    out.push_str("export interface GatherGap {\n");
    out.push_str("  code: string;\n");
    out.push_str("  sources: string[];\n");
    out.push_str("}\n\n");
    out.push_str("/** A gathering fact beside its completeness. `known` is a complete set, `partial` keeps the rows found and says what is missing, `unknown` has no rows and no claim of emptiness. */\n");
    out.push_str("export type GatherCell<T> =\n");
    out.push_str("  | { state: 'known'; value: T }\n");
    out.push_str("  | { state: 'partial'; value: T; gaps: GatherGap[] }\n");
    out.push_str("  | { state: 'unknown'; gaps: GatherGap[] };\n\n");
    out.push_str("export interface GatherGate {\n");
    out.push_str("  skill: number;\n");
    out.push_str("  level: number;\n");
    out.push_str("}\n\n");
    out.push_str("/** Source-derived respawn scaling: the raw content value and the tick bounds the scale rule gives it. */\n");
    out.push_str("export interface GatherRespawn {\n");
    out.push_str("  raw: number;\n");
    out.push_str("  scale: GatherCell<{ rule: string; min_ticks: number; max_ticks: number; sources: string[] }>;\n");
    out.push_str("  source: string;\n");
    out.push_str("}\n\n");
    out.push_str("/** One interactable entity of a method. Respawn belongs to the target; `null` inside `known` means no respawn applies. */\n");
    out.push_str("export interface GatherTargetRow {\n");
    out.push_str("  kind: 'loc' | 'npc';\n");
    out.push_str("  id: number;\n");
    out.push_str("  alias: string | null;\n");
    out.push_str("  op: number;\n");
    out.push_str("  class: 'resource' | 'depleted' | 'hazard' | 'unclassified';\n");
    out.push_str("  respawn: GatherCell<GatherRespawn | null>;\n");
    out.push_str("}\n\n");
    out.push_str("export interface GatherRequirementRow {\n");
    out.push_str("  id: string;\n");
    out.push_str("  source: string;\n");
    out.push_str("  kind: string;\n");
    out.push_str("  skill?: number;\n");
    out.push_str("  level?: number;\n");
    out.push_str("}\n\n");
    out.push_str("/** One gathering method: a woodcutting tree kind, a mining rock kind, or one fishing option of one spot type. */\n");
    out.push_str("export interface GatherMethodRow {\n");
    out.push_str("  id: string;\n");
    out.push_str("  skill: 'woodcutting' | 'mining' | 'fishing';\n");
    out.push_str("  resource_key: string | null;\n");
    out.push_str("  resources: string[];\n");
    out.push_str("  op: { slot: number; label: string } | null;\n");
    out.push_str("  loc_ids: GatherId[];\n");
    out.push_str("  npc_ids: GatherId[];\n");
    out.push_str("  empty_ids: GatherId[];\n");
    out.push_str("  hazard_ids: GatherId[];\n");
    out.push_str("  unclassified_ids: GatherId[];\n");
    out.push_str("  targets: GatherCell<GatherTargetRow[]>;\n");
    out.push_str("  products: GatherCell<Array<GatherId & { level: number }>>;\n");
    out.push_str("  tools: GatherCell<Array<GatherId & { use_gate: GatherGate | null; wield_gate: GatherGate | null }>>;\n");
    out.push_str("  consumes: GatherCell<Array<GatherId & { count: number }>>;\n");
    out.push_str("  requirements: GatherCell<GatherRequirementRow[]>;\n");
    out.push_str("  placements: GatherCell<{ count: number }>;\n");
    out.push_str("  qualification: 'complete' | 'partial';\n");
    out.push_str("}\n\n");
    out.push_str("/** One gap in the loaded pin's gathering facts: an incomplete method cell (`partial`/`unknown`) or an entity content could not classify (`unclassified`). */\n");
    out.push_str("export interface GatherCoverageRecord {\n");
    out.push_str("  class: 'partial' | 'unknown' | 'unclassified';\n");
    out.push_str("  code: string;\n");
    out.push_str("  sources: string[];\n");
    out.push_str("  method?: string;\n");
    out.push_str("  cell?: string;\n");
    out.push_str("  skill?: string;\n");
    out.push_str("  kind?: 'loc' | 'npc';\n");
    out.push_str("  id?: number;\n");
    out.push_str("  alias?: string | null;\n");
    out.push_str("}\n\n");
    out.push_str("/** One world placement of a method's resource target. `plane` is the placement's level; `spot` is the cursor form of its catalog-local id. */\n");
    out.push_str("export interface GatherPlacementRow {\n");
    out.push_str("  spot: string;\n");
    out.push_str("  method: string;\n");
    out.push_str("  kind: 'loc' | 'npc';\n");
    out.push_str("  id: number;\n");
    out.push_str("  alias: string | null;\n");
    out.push_str("  loc_id?: number;\n");
    out.push_str("  npc_id?: number;\n");
    out.push_str("  x: number;\n");
    out.push_str("  z: number;\n");
    out.push_str("  plane: number;\n");
    out.push_str("  width: number;\n");
    out.push_str("  length: number;\n");
    out.push_str("  movement: GatherCell<SceneRegionInput | null>;\n");
    out.push_str("  source: string;\n");
    out.push_str("}\n\n");
    out.push_str("/** One bounded page of placements in ascending id order. `next` resumes after the last row when `truncated`; `resource_ids` names the resource targets of the matched methods, not the hit list. `qualification: 'unknown'` carries no rows and says why in `gaps`. */\n");
    out.push_str("export interface GatherPlacementResult {\n");
    out.push_str("  rows: GatherPlacementRow[];\n");
    out.push_str("  truncated: boolean;\n");
    out.push_str("  next: string | null;\n");
    out.push_str("  resource_ids: GatherId[];\n");
    out.push_str("  qualification: 'complete' | 'unknown';\n");
    out.push_str("  gaps?: Array<{ method: string; gap: GatherGap }>;\n");
    out.push_str("}\n\n");
    out.push_str("export interface QuestSkillGate {\n");
    out.push_str("  skill: string;\n");
    out.push_str("  level: number;\n");
    out.push_str("}\n\n");
    out.push_str("export interface QuestItemAlias {\n");
    out.push_str("  alias: string;\n");
    out.push_str("  quantity: number | null;\n");
    out.push_str("  kind: string;\n");
    out.push_str("}\n\n");
    out.push_str("/** Requirements on the identity row. Qualification stays partial. */\n");
    out.push_str("export interface QuestRequirements {\n");
    out.push_str("  qualification: string;\n");
    out.push_str("  skills: QuestSkillGate[];\n");
    out.push_str("  items: QuestItemAlias[];\n");
    out.push_str("  empty_must_have: boolean;\n");
    out.push_str("  unknown_as_satisfied: boolean;\n");
    out.push_str("}\n\n");
    out.push_str("/** Landed identity row. complete is the constant, not a range. */\n");
    out.push_str("export interface QuestIdentityRow {\n");
    out.push_str("  id: string;\n");
    out.push_str("  component: string;\n");
    out.push_str("  display: string;\n");
    out.push_str("  varp: string;\n");
    out.push_str("  varp_id: number;\n");
    out.push_str("  complete: number;\n");
    out.push_str("  quest_points: number;\n");
    out.push_str("  unknown_sides: string[];\n");
    out.push_str("  requirements: QuestRequirements;\n");
    out.push_str("}\n\n");
    out.push_str("/** One landed trail membership row. `access` is present only on the one bounded inclusion. */\n");
    out.push_str("export interface ClueRow {\n");
    out.push_str("  alias: string;\n");
    out.push_str("  id: number;\n");
    out.push_str("  role: string;\n");
    out.push_str("  /** Raw param lines, in file order. Values are never coerced. */\n");
    out.push_str("  params: Array<{ key: string; value: string }>;\n");
    out.push_str("  access?: 'constrained';\n");
    out.push_str("}\n\n");
    out.push_str("/** Caller numbers only. Absent fields default; a present wrong type is invalid-args. */\n");
    out.push_str("export interface PackPlanInput {\n");
    out.push_str("  hostWant?: number;\n");
    out.push_str("  heldFood?: number;\n");
    out.push_str("  freeSlots?: number;\n");
    out.push_str("  reserveSlots?: number;\n");
    out.push_str("  perCast?: number;\n");
    out.push_str("  weaponName?: string;\n");
    out.push_str("  weaponInBackpack?: boolean;\n");
    out.push_str("  weaponEquipped?: boolean;\n");
    out.push_str("  casketAlias?: string;\n");
    out.push_str("}\n\n");
    out.push_str("/** `runeTarget`, `weaponNeeded`, and `rewardSlots` only when the caller asked for them. */\n");
    out.push_str("export interface PackPlanTargets {\n");
    out.push_str("  coordToolSlots: 3;\n");
    out.push_str("  teleportCasts: 20;\n");
    out.push_str("  food: number;\n");
    out.push_str("  runeTarget?: number;\n");
    out.push_str("  weaponNeeded?: boolean;\n");
    out.push_str("  rewardSlots?: number;\n");
    out.push_str("}\n\n");
    out.push_str("/** Caller-supplied one-level box. Not a plane and not a radius form. */\n");
    out.push_str("export interface SceneRegionInput {\n");
    out.push_str("  min_x: number;\n");
    out.push_str("  min_z: number;\n");
    out.push_str("  max_x: number;\n");
    out.push_str("  max_z: number;\n");
    out.push_str("  level: number;\n");
    out.push_str("}\n\n");
    out.push_str("/** Copied collision bounds. Collision flags stay on the snapshot page. */\n");
    out.push_str("export interface SceneBounds {\n");
    out.push_str("  available: boolean;\n");
    out.push_str("  base_x: number;\n");
    out.push_str("  base_z: number;\n");
    out.push_str("  level: number;\n");
    out.push_str("  width: number;\n");
    out.push_str("  height: number;\n");
    out.push_str("}\n\n");
    out.push_str(
        "/** Historical posted row copy. Not the live entity: no index, name, or nested tile. */\n",
    );
    out.push_str("export interface SceneProjectionRow {\n");
    out.push_str("  id: number;\n");
    out.push_str("  x: number;\n");
    out.push_str("  z: number;\n");
    out.push_str("  level: number;\n");
    out.push_str("  actions: string[];\n");
    out.push_str("}\n\n");
    out.push_str("/** One posted scene vector. as_of_sequence is snapshot.tick. */\n");
    out.push_str("export interface SceneProjection {\n");
    out.push_str("  as_of_sequence: number;\n");
    out.push_str("  scene: SceneBounds;\n");
    out.push_str("  rows: SceneProjectionRow[];\n");
    out.push_str("  truncated: boolean;\n");
    out.push_str("}\n\n");
    out.push_str("export interface BankLocation {\n");
    out.push_str("  name: string;\n  tile: WorldTile;\n  approach?: WorldTile;\n");
    out.push_str("  requires?: { skill?: { name: string; level: number }; quest?: string; setting?: string };\n");
    out.push_str(
        "  access?: { name: string; op: string; openFirst?: { name: string; op: string } };\n",
    );
    out.push_str("  npcAccess?: { name: string; op: string; choose?: string };\n}\n\n");
    out.push_str("/** Public JS API v2 handle. Explicit `export const apiVersion = 2` only. */\n");
    out.push_str("export interface NativeApi {\n");
    out.push_str("  readonly tick: number;\n");
    out.push_str(
        "  /** Host-owned, delta-merged. Read only. Do not mutate; copy if retaining. */\n",
    );
    out.push_str("  readonly snapshot: NativeSnapshot;\n");
    out.push_str("  readonly settings: NativeSettings;\n");
    out.push_str("  log(message: string): void;\n");
    out.push_str("  stop(reason?: string): void;\n");
    out.push_str("  readonly paint: NativePaint;\n");
    out.push_str("  request(op: NativeOp): void;\n");
    out.push_str("  /** Isolate-owned inspect token. Pair with request inspect-route request_id, or query inspectSettled/inspectValue. Caller-invented ids never consume host jobs; 0 is snapshot-only preview. */\n");
    out.push_str("  inspectBegin(opts: { from: WorldTile; to: WorldTile; allow_teleports?: boolean; allow_wilderness?: boolean; allow_bank_fetch?: boolean; avoid?: Array<{ minX: number; maxX: number; minZ: number; maxZ: number; level?: number }>; timeout_ms?: number }): number;\n");
    out.push_str("  inspectSettled(token: number): boolean;\n");
    out.push_str("  inspectValue(token: number): { ok: boolean; reason: string; bankPlanned: boolean; ticks: number; hops: Array<{ kind: string; locId: number; locName: string; action: string; option: number; from: WorldTile; to: WorldTile; ticks: number }>; request_id: number } | null;\n");
    out.push_str("  prayerPoints(): HelperResult<number>;\n");
    out.push_str("  prayerMax(): HelperResult<number>;\n");
    out.push_str("  prayerFull(): HelperResult<boolean>;\n");
    out.push_str("  prayerKnown(input: { name: string }): HelperResult<boolean>;\n");
    out.push_str("  prayerAvailable(input: { name: string }): HelperResult<boolean>;\n");
    out.push_str("  prayerActive(input: { name: string }): HelperResult<boolean>;\n");
    out.push_str("  /** Named async private-lifecycle exception. Final HelperResult only; callers never see Step.\n");
    out.push_str("   * Before: a second Set/Clear overwrote the private pump and could hang the first Promise; a sync tick that did not return that Promise did not advance it.\n");
    out.push_str("   * After: a second Set/Clear while one operation is already admitted returns `{ok:false, error:'busy'}` without begin/click. The admitted operation keeps ownership and must settle. Sequential `await` is the preferred example; fire-and-forget still progresses on later eligible NativeTicks. Additional public error: `busy`.\n");
    out.push_str("   */\n");
    out.push_str(
        "  prayerSet(input: { name: string; on: boolean }): Promise<HelperResult<boolean>>;\n",
    );
    out.push_str("  /** Completes the 15-row walk. timed_out may be nonzero; LIVE later requires all off. Same busy refuse as prayerSet. */\n");
    out.push_str("  prayerClear(): Promise<HelperResult<PrayerClearCounts>>;\n");
    out.push_str("  /** Select without moving. Native wilderness defaults false; an air fallback does not prove reachability. */\n");
    out.push_str("  bankNearestReachable(input?: { from?: WorldTile; allow_wilderness?: boolean; use_mage_bank?: boolean; use_zanaris_bank?: boolean }): Promise<HelperResult<BankLocation | null>>;\n");
    out.push_str(
        "  foodCount(input: { items: ItemRow[]; foodName: string }): HelperResult<number>;\n",
    );
    out.push_str("  foodHealAmount(input: { foodName: string }): HelperResult<number>;\n");
    out.push_str("  combatKeepNames(input: { food: string; style?: string; spell?: string; ammo?: string; weapon?: string; extra?: string[] }): HelperResult<string[]>;\n");
    out.push_str("  runesPerCast(input: { spellName: string; wielded: string[] }): HelperResult<Array<{ rune: string; count: number }> | null>;\n");
    out.push_str("  escapeRunesFor(input: { id: string }): HelperResult<{ runes: Array<{ rune: string; count: number }>; level: number; label: string }>;\n");
    out.push_str("  /** Sync fact read. Omitted input or `{}` omits the skill. `coverage` lists every incomplete fact of the loaded pin, whatever the skill filter. Not a Promise and not a request op. */\n");
    out.push_str("  gatherMethods(input?: { skill?: string }): HelperResult<{ rows: GatherMethodRow[]; coverage: GatherCoverageRecord[] }>;\n");
    out.push_str("  /** Sync resource-key read (wood/ore key or fish item alias). Zero matches is unknown-resource, not an empty rows list. */\n");
    out.push_str(
        "  gatherResource(input: { name: string }): HelperResult<{ rows: GatherMethodRow[] }>;\n",
    );
    out.push_str("  /** Sync world placements of a resource key inside a required one-level region, ascending by id. Omitted `region` is `missing-region`. `limit` is 1..=64; `after` is a previous page's `next`. Incomplete placement coverage is `qualification: 'unknown'` with no rows, never an empty complete page. Not a Promise and not a request op. */\n");
    out.push_str("  gatherPlacements(input: { resource: string; region: SceneRegionInput; limit: number; after?: string }): HelperResult<GatherPlacementResult>;\n");
    out.push_str("  /** Sync fact read. Exactly one of name or id, and it must be a string. Not a Promise and not a request op. */\n");
    out.push_str("  questIdentity(input: { name: string } | { id: string }): HelperResult<QuestIdentityRow>;\n");
    out.push_str("  /** Sync seed-id requirements read. A name field is not a key. Not a Promise and not a request op. */\n");
    out.push_str(
        "  questPrereqs(input: { id: string; name?: string }): HelperResult<QuestRequirements>;\n",
    );
    out.push_str("  /** Sync landed trail-row read, pure pack arithmetic, pure hard-kit status, the pure keep predicate over caller facts, and the clue machine's begin / one awaited run / retry. `run` is the only Promise and none is a request op; `packPlan`, `hardKit`, and `keep` read no snapshot, inventory, or family, and `retry` is the machine's latch clear — never a connection-boundary reset and never a token abort. */\n");
    out.push_str("  clue: { row(input: { id: number } | { alias: string }): HelperResult<ClueRow>; heldStep(): HelperResult<ClueRow>; packPlan(input: PackPlanInput): HelperResult<PackPlanTargets>; hardKit(input: { attack: number; lostCity: boolean; items: { id: number; count: number }[] }): HelperResult<{ status: 'ready' }>; keep(input: { name: string; extra?: string[] }): HelperResult<{ keep: boolean }>; begin(input?: object): HelperResult<{ token: number }>; run(input: { token: number }, hooks?: ClueHooks): Promise<ClueOutcome>; retry(): HelperResult<{ cleared: true }> };\n");
    out.push_str("  /** Sync posted-loc copy. Historical copy, not live. Not a Promise and not a request op. */\n");
    out.push_str("  sceneLocs(input: { ids: number[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>;\n");
    out.push_str("  /** Sync posted-npc copy. actions is required: omitted is not match-any. Historical copy, not live. Not a Promise and not a request op. */\n");
    out.push_str("  sceneNpcs(input: { types: number[]; actions: string[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>;\n");
    out.push_str("  /** Sync posted-tab status copy. A missing page is snapshot-unavailable and a null tab is quest-tab-unbound. Not a Promise and not a request op. */\n");
    out.push_str("  questStatus(input: { name: string }): HelperResult<{ status: 'notStarted' | 'inProgress' | 'complete' | 'unknown'; as_of_sequence: number }>;\n");
    out.push_str("  /** Sync owned-root begin. Admits one posted quest row and returns its token without clicking; refusals include `invalid-args`, unavailable/unbound scene data, an unknown quest, an occupied modal, `busy`, `stale`, and `frozen`. */\n");
    out.push_str(
        "  questJournalBegin(input: { name: string }): HelperResult<{ token: number }>;\n",
    );
    out.push_str("  /** One awaited run. Rust clicks the admitted row, acquires its exact modal, returns its lines, and closes only that modal inside a bounded observation window. */\n");
    out.push_str("  questJournalRun(input: { token: number }): Promise<QuestJournalOutcome>;\n");
    out.push_str("  /** Native gathering: the Gatherer card the operator starts from Browse, driven by this script. One session per slot. While a session is live the host owns the slot's foreground: this script's game ops are dropped, not deferred. */\n");
    out.push_str("  gather: {\n");
    out.push_str("    /** One awaited session. Settles at its terminal (`value.end`). Refused `busy` until the previous session's promise has settled. */\n");
    out.push_str("    run(settings?: GatherSettings): Promise<GatherOutcome>;\n");
    out.push_str("    /** Ends the live session; its `run` resolves `stopped`. Idempotent. Refused `no-session`. */\n");
    out.push_str("    stop(): HelperResult<null>;\n");
    out.push_str("  };\n");
    out.push_str(
        "  /** Sync read of the release Path index. Not a Promise and not a request op. */\n",
    );
    out.push_str("  questPaths(): HelperResult<{ rows: QuestPathRow[] }>;\n");
    out.push_str("  /** One awaited owned progress read: tab colour first; the quiet host journal read only when the colour is in-progress and the released Path has journal rules. */\n");
    out.push_str("  questProgress(input: { quest: string }): Promise<QuestProgressOutcome>;\n");
    out.push_str("  foodOf(input: { loadout: LoadoutInput | null; fallback: string }): HelperResult<string>;\n");
    out.push_str("  gearOf(input: { loadout: LoadoutInput | null }): HelperResult<string[]>;\n");
    out.push_str("  suppliesOf(input: { loadout: LoadoutInput | null }): HelperResult<Array<{ item: string; qty: number }>>;\n");
    out.push_str("  weaponOf(input: { loadout: LoadoutInput | null; fallback?: string | null }): HelperResult<string | null>;\n");
    out.push_str(
        "  rangeLoadoutOf(input: { weapon: string; ammo: string }): HelperResult<RangeLoadout>;\n",
    );
    out.push_str("  boostFaded(input: { base: number; effective: number; floor?: number }): HelperResult<boolean>;\n");
    out.push_str("  plannedPotions(input: { carry: Array<{ item: string; qty: number }> }): HelperResult<PotionPlan[]>;\n");
    out.push_str("  potionToSip(input: { plans: PotionPlan[]; held: number[]; levels: Array<{ skill: string; base: number; effective: number }> }): HelperResult<PotionPlan | null>;\n");
    out.push_str("  lineOfSight(input: { from: WorldTile; to: WorldTile; size?: number }): HelperResult<boolean>;\n");
    out.push_str("  walkable(input: { tile: WorldTile } | WorldTile): HelperResult<boolean>;\n");
    out.push_str("  canStep(input: { from: WorldTile; to: WorldTile }): HelperResult<boolean>;\n");
    out.push_str("  canReach(input: { tile: WorldTile; adjacentOk?: boolean; maxSteps?: number }): HelperResult<boolean>;\n");
    out.push_str("  /** Hunt Task sessions: `*Begin(site)` keeps the site with a token; `*Validate` is one synchronous read (Rust calls the hook getters and `inArea`); `*Run` awaits one Rust run (walks, ops, waits and retries are the host's). */\n");
    out.push_str("  fightBegin(site: HuntSite): HelperResult<{ token: number }>;\n");
    out.push_str("  fightValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;\n");
    out.push_str("  fightRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>;\n");
    out.push_str("  fightReset(input: HuntToken): HelperResult<null>;\n");
    out.push_str("  fightInterruptWatch(input: HuntToken): HelperResult<null>;\n");
    out.push_str(
        "  fightBlocksLoot(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;\n",
    );
    out.push_str("  holdBegin(site: HuntSite): HelperResult<{ token: number }>;\n");
    out.push_str("  holdValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;\n");
    out.push_str("  holdRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>;\n");
    out.push_str("  retreatBegin(site: HuntSite): HelperResult<{ token: number }>;\n");
    out.push_str(
        "  retreatValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;\n",
    );
    out.push_str(
        "  retreatRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>;\n",
    );
    out.push_str("  walkspotBegin(site: HuntSite): HelperResult<{ token: number }>;\n");
    out.push_str(
        "  walkspotValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;\n",
    );
    out.push_str(
        "  walkspotRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>;\n",
    );
    out.push_str("  enterBegin(site: HuntSite): HelperResult<{ token: number }>;\n");
    out.push_str("  enterValidate(input: HuntToken, hooks?: HuntHooks): HelperResult<boolean>;\n");
    out.push_str("  /** `value`: inside the lair. */\n");
    out.push_str(
        "  enterRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;\n",
    );
    out.push_str("  /** One awaited run; `value`: out of the lair. */\n");
    out.push_str("  leaveRun(site: HuntSite, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;\n");
    out.push_str("  /** One awaited run of the Jailer leg alone (corridor walk, kill, take the jail key); `value`: the jail key is held. Not v1 `acquireKey`, which composes leave, the bank stop and up to three Velrak fetches and settles a `KeyState`; v2 composes those legs from `leaveRun`, `bankRun` and `cellRun` itself. */\n");
    out.push_str("  keyRun(site: HuntSite, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;\n");
    out.push_str("  /** One awaited run through the jail cell (jail key, unlock, Velrak, back out); `value`: the site's key is held outside the cell. */\n");
    out.push_str("  cellRun(site: HuntSite, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;\n");
    out.push_str("  /** One awaited bank trip; `value`: restocked. */\n");
    out.push_str("  bankRun(site: HuntSite, opts?: HuntBankOptions, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>;\n");
    out.push_str("}\n\n");
    out.push_str("export type HuntToken = { token: number };\n");
    out.push_str("export type HuntBox = { minX: number; maxX: number; minZ: number; maxZ: number; level: number };\n");
    out.push_str("/** Plain site data. Its area is `boxes`, unless `hooks.inArea` answers. */\n");
    out.push_str("export interface HuntSite {\n");
    out.push_str("  key: string;\n");
    out.push_str("  boxes?: HuntBox[];\n");
    out.push_str("  target?: string;\n");
    out.push_str("  alsoHunt?: string[];\n");
    out.push_str("  safespots?: WorldTile[];\n");
    out.push_str("  meleeAnchor?: WorldTile;\n");
    out.push_str("  approach?: WorldTile[];\n");
    out.push_str("  fireAtRange?: boolean;\n");
    out.push_str("  rangedThreat?: boolean;\n");
    out.push_str("  bank?: WorldTile | null;\n");
    out.push_str("  keyItem?: { name: string; id: number } | null;\n");
    out.push_str("  coins?: number | null;\n");
    out.push_str("  escapeTeleportId?: string | null;\n");
    out.push_str("  walkOut?: WorldTile | null;\n");
    out.push_str(
        "  talkGate?: { npc: string; op: string; choose: string; stand: WorldTile } | null;\n",
    );
    out.push_str("  feeGate?: { npc: string; op: string; coins: number; stand: WorldTile; entrance?: { locId: number; op: string } | null; paidLine: string; prepaidLine: string } | null;\n");
    out.push_str("  gate?: { locId: number; op: string; outside?: WorldTile | null; inside?: WorldTile | null } | null;\n");
    out.push_str("  exit?: { locId: number; op: string; stand: WorldTile } | null;\n");
    out.push_str("}\n");
    out.push_str("/** Bank-trip loadout (the frozen `BankOpts`), merged over the site. Rust owns the defaults: `runeCasts` 150, `runeBuffer` 300, `escapeStock` 2, `ammo` 500, `healTo` 0.9. */\n");
    out.push_str("export interface HuntBankOptions {\n");
    out.push_str("  withdrawFood?: boolean;\n");
    out.push_str("  wear?: string[];\n");
    out.push_str("  carry?: string[];\n");
    out.push_str("  runeCasts?: number;\n");
    out.push_str("  runeBuffer?: number;\n");
    out.push_str("  escapeStock?: number;\n");
    out.push_str("  ammo?: number;\n");
    out.push_str(
        "  potions?: Array<{ flask: string; potion: { doses: string[] }; want: number }>;\n",
    );
    out.push_str("  flasks?: Array<{ flask: string; doses: string[]; want: number }>;\n");
    out.push_str("  healTo?: number;\n");
    out.push_str("}\n");
    out.push_str("/** The caller's own hooks, all optional. Getters and notifications are called synchronously by Rust when a decision reads them (a returned promise is `not impl`); `eatOnce`, `armSpecial`, `sustain` and `leave` are awaited. An absent getter reads as its default. */\n");
    out.push_str("export interface HuntHooks {\n");
    out.push_str("  log?(message: string): void;\n");
    out.push_str("  vlog?(message: string): void;\n");
    out.push_str("  setStatus?(message: string): void;\n");
    out.push_str("  eatOnce?(): boolean | Promise<boolean>;\n");
    out.push_str("  armSpecial?(): void | Promise<void>;\n");
    out.push_str("  sustain?(): void | Promise<void>;\n");
    out.push_str("  /** Replaces the leave run inside `bankRun` / `keyRun` / `cellRun`; `true` when out. */\n");
    out.push_str("  leave?(): boolean | Promise<boolean>;\n");
    out.push_str("  countBurial?(): void;\n");
    out.push_str("  countKill?(): void;\n");
    out.push_str("  setSafespotIndex?(index: number): void;\n");
    out.push_str("  setTarget?(index: number | null): void;\n");
    out.push_str("  pickWeapon?(names: string[]): void;\n");
    out.push_str("  parkFor?(reason: string): void;\n");
    out.push_str("  countBankTrip?(): void;\n");
    out.push_str("  died?(): boolean;\n");
    out.push_str("  targetIdx?(): number | null;\n");
    out.push_str("  hpFraction?(): number;\n");
    out.push_str("  panicHp?(): number;\n");
    out.push_str("  retreatHp?(): number;\n");
    out.push_str("  hasFood?(): boolean;\n");
    out.push_str("  needEat?(): boolean;\n");
    out.push_str("  style?(): 'melee' | 'range' | 'mage';\n");
    out.push_str("  safespotIndex?(): number;\n");
    out.push_str("  buryBones?(): boolean;\n");
    out.push_str("  boneName?(): string;\n");
    out.push_str("  shieldReady?(): boolean;\n");
    out.push_str("  parked?(): boolean;\n");
    out.push_str("  leaveByWalk?(): boolean;\n");
    out.push_str("  foodName?(): string;\n");
    out.push_str("  foodWithdraw?(): number;\n");
    out.push_str("  weaponName?(): string;\n");
    out.push_str("  ammoName?(): string;\n");
    out.push_str("  spellName?(): string;\n");
    out.push_str("  keepExtra?(): string[];\n");
    out.push_str("  /** The site's area test; replaces `boxes` when present. */\n");
    out.push_str("  inArea?(tile: WorldTile): boolean;\n");
    out.push_str("}\n");
    out.push_str(
        "/** One run's settlement. A hook that throws rejects the promise with that value. `aborted` reasons are the host's. A run the Rust stepper itself gives up on (a KBD site, an unexpected reply, a lost walk) settles `done` with `false` (boolean runs) or `null` (fight/hold/retreat/walkspot), the same as a run that did not get there: treat anything but `done` with `true` as failure, and for the `null` runs prove the result from the scene (the player's tile). */\n",
    );
    out.push_str("export type HuntOutcome<T> =\n");
    out.push_str("  | { kind: 'done'; value: T }\n");
    out.push_str("  | { kind: 'refused'; reason: string }\n");
    out.push_str(
        "  | { kind: 'aborted'; reason: 'reset' | 'superseded' | 'terminated' | 'unknown' };\n",
    );
    out.push_str("/** Callbacks frozen for one clue run. Rust awaits each returned promise before advancing the machine. */\n");
    out.push_str("export interface ClueHooks {\n");
    out.push_str("  /** Gates a held step; absent defaults to true. */\n");
    out.push_str("  enabled?(): boolean | Promise<boolean>;\n");
    out.push_str("  log?(message: string): void | Promise<void>;\n");
    out.push_str("  setStatus?(message: string): void | Promise<void>;\n");
    out.push_str("}\n");
    out.push_str("export type ClueRunValue =\n");
    out.push_str(
        "  | { kind: 'yield' | 'done' | 'dead' | 'abandon' | 'guardian-lost'; token: number }\n",
    );
    out.push_str("  | { kind: 'aborted'; token: number; reason: string };\n");
    out.push_str("/** One clue run's settlement. A hook that throws rejects the promise with that value. */\n");
    out.push_str("export type ClueOutcome =\n");
    out.push_str("  | { kind: 'done'; value: ClueRunValue }\n");
    out.push_str("  | { kind: 'refused'; reason: string }\n");
    out.push_str(
        "  | { kind: 'aborted'; reason: 'reset' | 'superseded' | 'terminated' | 'unknown' };\n",
    );
    out.push_str("/** The journal machine's terminal value. `as_of_sequence` observed the acquired lines; `closed_as_of_sequence` later proved that exact modal closed. */\n");
    out.push_str("export type QuestJournalRunValue =\n");
    out.push_str("  | { kind: 'done'; token: number; lines: string[]; root: number; as_of_sequence: number; closed_as_of_sequence: number }\n");
    out.push_str("  | { kind: 'aborted'; token: number; reason: string };\n");
    out.push_str("export type QuestJournalOutcome =\n");
    out.push_str("  | { kind: 'done'; value: QuestJournalRunValue }\n");
    out.push_str("  | { kind: 'refused'; reason: string }\n");
    out.push_str(
        "  | { kind: 'aborted'; reason: 'reset' | 'superseded' | 'terminated' | 'unknown' };\n",
    );
    out.push('\n');
    render_gather_settings(out);
    out.push('\n');
    render_gather_status(out);
    out.push('\n');
    render_gather_session(out);
    out.push('\n');
    render_quest_progress(out);
}

/// `QuestPathRow`, `Truth3`, `Known`, `QuestProgressRow`,
/// `QuestProgressEnd` and `QuestProgressOutcome`: the release Path index
/// and one owned progress read (design section 2, slice B).
fn render_quest_progress(out: &mut String) {
    out.push_str(
        "/** One released quest path from the Quester release index; stages and journal availability come from its Path document. */\n",
    );
    out.push_str("export interface QuestPathRow {\n");
    out.push_str("  id: string;\n");
    out.push_str("  display: string;\n");
    out.push_str("  journal: boolean;\n");
    out.push_str("  stages: string[];\n");
    out.push_str("}\n\n");
    out.push_str("/** Three-valued truth used by progress rows and their flags. */\n");
    out.push_str("export type Truth3 = 'true' | 'false' | 'unknown';\n\n");
    out.push_str("/** A resolved value or the content gap behind an unknown one. */\n");
    out.push_str("export type Known<T> =\n");
    out.push_str("  | { state: 'known'; value: T }\n");
    out.push_str("  | { state: 'unknown'; gap: string };\n\n");
    out.push_str("/** One owned quest progress read: tab colour first; the quiet host journal read only when the colour is in-progress and the released Path has journal rules. */\n");
    out.push_str("export interface QuestProgressRow {\n");
    out.push_str("  quest: string;\n");
    out.push_str("  display: string;\n");
    out.push_str("  colour: 'notStarted' | 'inProgress' | 'complete' | 'unknown';\n");
    out.push_str("  stage: Known<string>;\n");
    out.push_str("  complete: Truth3;\n");
    out.push_str("  rule: Known<string>;\n");
    out.push_str("  flags: Array<{ flag: string; truth: Truth3; count: number | null }>;\n");
    out.push_str("  evidence: { run: number; session: number; tick: number; sequence: number };\n");
    out.push_str("  journal_read: boolean;\n");
    out.push_str("  binding: string;\n");
    out.push_str("  role: string | null;\n");
    out.push_str("}\n\n");
    out.push_str("/** How a progress read ended (the `value` of a `done` outcome). `refused` carries `unknown-path`, `busy`, `unavailable:<why>` or `failed:<why>` and never ran. */\n");
    out.push_str("export type QuestProgressEnd =\n");
    out.push_str("  | { end: 'done'; token: number; row: QuestProgressRow }\n");
    out.push_str("  | { end: 'refused'; token: number; reason: string };\n\n");
    out.push_str("export type QuestProgressOutcome =\n");
    out.push_str("  | { kind: 'done'; value: QuestProgressEnd }\n");
    out.push_str("  | { kind: 'refused'; reason: string }\n");
    out.push_str(
        "  | { kind: 'aborted'; reason: 'reset' | 'superseded' | 'terminated' | 'unknown' };\n",
    );
}

/// `GatherSettings`, rendered row by row from the Gatherer card's own
/// settings schema (`gatherer::settings::schema`).
fn render_gather_settings(out: &mut String) {
    out.push_str("/** The Gatherer card's own settings schema (crates/script/src/gatherer/settings.rs), rendered row by row. Every key is optional and takes the card default. A malformed object (unknown key, wrong type, non-integer number) is refused `invalid-settings`; a semantic failure is refused `invalid-setting:<field>:<code>` (the card's own keyed ConfigError). */\n");
    out.push_str("export interface GatherSettings {\n");
    for def in schema() {
        out.push_str("  /** ");
        out.push_str(&gather_setting_doc(def));
        out.push_str(" */\n");
        out.push_str("  ");
        out.push_str(&def.id);
        out.push_str("?: ");
        out.push_str(&gather_setting_ts(def));
        out.push_str(";\n");
    }
    out.push_str("}\n");
}

fn gather_setting_ts(def: &SettingDef) -> String {
    match def.ty.as_str() {
        "boolean" => "boolean".to_string(),
        "number" => "number".to_string(),
        "tile" => "WorldTile".to_string(),
        "list" => "string[]".to_string(),
        "string" if !def.options.is_empty() => def
            .options
            .iter()
            .map(|option| format!("'{option}'"))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => "string".to_string(),
    }
}

fn gather_setting_doc(def: &SettingDef) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let (Some(min), Some(max)) = (def.min.as_deref(), def.max.as_deref()) {
        parts.push(format!("{min}..{max}"));
    }
    match def.default.as_deref() {
        Some(default) => {
            if def.ty == "list" && !default.starts_with('[') {
                parts.push(format!("default [\"{default}\"]"));
            } else if def.ty == "string" {
                parts.push(format!("default \"{default}\""));
            } else {
                parts.push(format!("default {default}"));
            }
        }
        None => parts.push("no card default".to_string()),
    }
    if def.show_if.is_some() {
        parts.push("only when its card show-if holds".to_string());
    }
    parts.join(", ")
}

/// `GatherStatus`, keyed exactly as the card publishes it
/// (`gatherer::status::KEYS`).
fn render_gather_status(out: &mut String) {
    out.push_str("/** The Gatherer's published status, keyed exactly as the card publishes it (gatherer::status::KEYS). `xp_per_hour` is -1 below five minutes elapsed. */\n");
    out.push_str("export interface GatherStatus {\n");
    for (key, kind) in KEYS {
        out.push_str("  ");
        out.push_str(key);
        out.push_str(": ");
        out.push_str(match kind {
            FieldKind::Text => "string",
            FieldKind::Integer => "number",
        });
        out.push_str(";\n");
    }
    out.push_str("}\n");
}

/// `GatherSession`, `GatherFailure`, `GatherCounts`, `GatherEnd` and
/// `GatherOutcome`: the slot's live API session and how a session ended.
/// Quest progress reads (`QuestPathRow`, `Truth3`, `Known`,
/// `QuestProgressRow`, `QuestProgressEnd`, `QuestProgressOutcome`) are
/// rendered by `render_quest_progress` beside it.
fn render_gather_session(out: &mut String) {
    out.push_str("/** The slot's live API session; `null` when none. Host-owned, read only. */\n");
    out.push_str("export interface GatherSession {\n");
    out.push_str("  token: number;\n");
    out.push_str("  /** 'preparing' while the host prepares the card off-pump; 'running' once installed */\n");
    out.push_str("  phase: 'preparing' | 'running';\n");
    out.push_str("  /** `null` until the card publishes its first status (preparation and the install tick) */\n");
    out.push_str("  status: GatherStatus | null;\n");
    out.push_str("}\n\n");
    out.push_str("export interface GatherFailure {\n");
    out.push_str("  code: string;\n");
    out.push_str("  message: string;\n");
    out.push_str("  retryable: boolean;\n");
    out.push_str("}\n\n");
    out.push_str("export interface GatherCounts {\n");
    out.push_str("  yielded: number;\n");
    out.push_str("  dropped: number;\n");
    out.push_str("  deposited: number;\n");
    out.push_str("  trips: number;\n");
    out.push_str("  xp: number;\n");
    out.push_str("}\n\n");
    out.push_str("/** How a session ended (the `value` of a `done` outcome). `blocked` keeps the slot's retained anchor/counters so the next `run` resumes; `stopped` clears them (operator-Stop semantics). */\n");
    out.push_str("export type GatherEnd =\n");
    out.push_str("  | { end: 'stopped'; token: number; counts: GatherCounts }\n");
    out.push_str(
        "  | { end: 'blocked'; token: number; failure: GatherFailure; counts: GatherCounts }\n",
    );
    out.push_str("  | { end: 'refused'; token: number; reason: string }\n");
    out.push_str("  | { end: 'failed'; token: number; reason: string; counts: GatherCounts };\n\n");
    out.push_str("export type GatherOutcome =\n");
    out.push_str("  | { kind: 'done'; value: GatherEnd }\n");
    out.push_str("  | { kind: 'refused'; reason: string }\n");
    out.push_str(
        "  | { kind: 'aborted'; reason: 'reset' | 'superseded' | 'terminated' | 'unknown' };\n",
    );
}

const SUPPORTING_INTERFACES: &[TsInterface] = &[
    TsInterface {
        name: "CollisionFlagsView",
        doc: Some("Readonly raw i32 flags. at requires a finite integer index in [0, length). Negative and non-integers are undefined."),
        fields: &[
            TsField {
                name: "length",
                ty: "number",
                optional: false,
                doc: Some("width*height, or 0 when !available."),
            },
            TsField {
                name: "at",
                ty: "(index: number) => number | undefined",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "CollisionView",
        doc: Some("Posted one-plane collision identity. Retained views stay historical after a later post."),
        fields: &[
            TsField { name: "available", ty: "boolean", optional: false, doc: None },
            TsField { name: "base_x", ty: "number", optional: false, doc: None },
            TsField { name: "base_z", ty: "number", optional: false, doc: None },
            TsField { name: "level", ty: "number", optional: false, doc: None },
            TsField { name: "width", ty: "number", optional: false, doc: None },
            TsField { name: "height", ty: "number", optional: false, doc: None },
            TsField { name: "flags", ty: "CollisionFlagsView", optional: false, doc: None },
        ],
    },
    TsInterface {
        name: "WorldTile",
        doc: Some("Absolute world tile `{x, z, level}`."),
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "ItemRow",
        doc: Some("One inventory/bank/equipment/trade row from the posted snapshot."),
        fields: &[
            TsField {
                name: "name",
                ty: "string | null",
                optional: false,
                doc: None,
            },
            TsField {
                name: "count",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "ops",
                ty: "string[]",
                optional: false,
                doc: None,
            },
            TsField {
                name: "noted",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "cert",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component_id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "slot",
                ty: "number",
                optional: true,
                doc: Some("Container slot when the host has exact item identity."),
            },
        ],
    },
    TsInterface {
        name: "StatRow",
        doc: None,
        fields: &[
            TsField {
                name: "index",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "xp",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "base",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "effective",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "SceneEntity",
        doc: Some("One npc/loc/player/ground row from the posted snapshot."),
        fields: &[
            TsField {
                name: "index",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string | null",
                optional: false,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "distance",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "health",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "max_health",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "in_combat",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "animating",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "actions",
                ty: "string[]",
                optional: false,
                doc: None,
            },
            TsField {
                name: "reachable",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "reachable_adj",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "combat_level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "target_kind",
                ty: "number",
                optional: false,
                doc: Some("0 none, 1 npc, 2 player."),
            },
            TsField {
                name: "target_index",
                ty: "number",
                optional: false,
                doc: Some("-1 when not facing anyone."),
            },
            TsField {
                name: "size",
                ty: "number",
                optional: false,
                doc: Some("NPC footprint in tiles. <1 means observation absent (old buffer / non-NPC row). Packet-time at last NPC_INFO."),
            },
            TsField {
                name: "nx",
                ty: "number",
                optional: false,
                doc: Some("Path-head network SW x. Valid world 0 is not absence when size>=1."),
            },
            TsField {
                name: "nz",
                ty: "number",
                optional: false,
                doc: Some("Path-head network SW z."),
            },
        ],
    },
    TsInterface {
        name: "BankStand",
        doc: Some("A packed bank stand (booth loc or teller npc)."),
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "kind",
                ty: "'booth' | 'npc'",
                optional: false,
                doc: None,
            },
            TsField {
                name: "op",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "choose",
                ty: "string | null",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "BankApproach",
        doc: Some("Packed bank dest/readiness row from the host."),
        fields: &[
            TsField { name: "loc_id", ty: "number", optional: false, doc: None },
            TsField { name: "x", ty: "number", optional: false, doc: None },
            TsField { name: "z", ty: "number", optional: false, doc: None },
            TsField { name: "level", ty: "number", optional: false, doc: None },
            TsField { name: "can_operate", ty: "boolean", optional: false, doc: None },
            TsField { name: "dest_ok", ty: "boolean", optional: false, doc: None },
            TsField { name: "dest_x", ty: "number", optional: false, doc: None },
            TsField { name: "dest_z", ty: "number", optional: false, doc: None },
            TsField { name: "dest_level", ty: "number", optional: false, doc: None },
        ],
    },
    TsInterface {
        name: "NearestBooth",
        doc: None,
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "op",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "ChatOption",
        doc: None,
        fields: &[
            TsField {
                name: "text",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "comId",
                ty: "number",
                optional: false,
                doc: Some("Exact BUTTON_OK component identity."),
            },
        ],
    },
    TsInterface {
        name: "ToggleControls",
        doc: Some("Native on/off component identity for one toggle."),
        fields: &[
            TsField {
                name: "onComId",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "offComId",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "QuestStatusRow",
        doc: Some("One native quest-tab row with its Rust-resolved coarse status."),
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "status",
                ty: "'notStarted' | 'inProgress' | 'complete' | 'unknown'",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component_id",
                ty: "number",
                optional: true,
                doc: Some(
                    "The row's walked TYPE_TEXT id (its click target). Omitted on an old buffer: the row has no target, and 0 is a real id.",
                ),
            },
        ],
    },
    TsInterface {
        name: "ReachQueryView",
        doc: Some("Posted reach window metadata. Packed bits are not materialized; use api.walkable / canStep / canReach."),
        fields: &[
            TsField {
                name: "available",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "base_x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "base_z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "width",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "height",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "VarpRow",
        doc: None,
        fields: &[
            TsField {
                name: "index",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "value",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "CombatStyleButton",
        doc: None,
        fields: &[
            TsField {
                name: "mode",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "label",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component_id",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "SideTabIface",
        doc: None,
        fields: &[
            TsField {
                name: "index",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "ChatLine",
        doc: None,
        fields: &[
            TsField {
                name: "seq",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "text",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "MakeButton",
        doc: None,
        fields: &[
            TsField {
                name: "qty",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "comId",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    TsInterface {
        name: "MakeProduct",
        doc: None,
        fields: &[
            TsField {
                name: "object_id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "buttons",
                ty: "MakeButton[]",
                optional: false,
                doc: None,
            },
        ],
    },
];

const SNAPSHOT_FIELDS: &[TsField] = &[
    TsField {
        name: "tick",
        ty: "number",
        optional: false,
        doc: Some("Always carried; other fields are delta-posted."),
    },
    TsField {
        name: "here",
        ty: "WorldTile | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "ingame",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "inv",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "inv_size",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "stats",
        ty: "StatRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "booths",
        ty: "WorldTile[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "nearest_booth",
        ty: "NearestBooth | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "banks",
        ty: "BankStand[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_side",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_loaded",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_generation",
        ty: "number",
        optional: false,
        doc: Some("Open/close session identity for bank actions; item updates do not change it."),
    },
    TsField {
        name: "bank_snapshot_generation",
        ty: "number",
        optional: false,
        doc: Some("Open bank item packet generation; -1 while closed. Not an action session identity."),
    },
    TsField {
        name: "count_dialog_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "withdraw_x_result_seq",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "withdraw_x_result",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "withdraw_load_result_seq",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "withdraw_load_result",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_op_result_seq",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_op_result",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_note_on",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "bank_note_off",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "scene_state",
        ty: "number",
        optional: false,
        doc: Some("2 = 3D ready."),
    },
    TsField {
        name: "weight",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "camera_yaw",
        ty: "number",
        optional: false,
        doc: Some("Orbit camera yaw."),
    },
    TsField {
        name: "camera_pitch",
        ty: "number",
        optional: false,
        doc: Some("Orbit camera pitch."),
    },
    TsField {
        name: "teleports_enabled",
        ty: "boolean",
        optional: false,
        doc: Some("Whether packed nav last armed with `allow_teleports`."),
    },
    TsField {
        name: "self_slot",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_offer_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_confirm_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_partner",
        ty: "string | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_mine",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_theirs",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_side",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_accept_id",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "trade_decline_id",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "shop_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "shop_stock",
        ty: "ShopStockRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "reach",
        ty: "ReachQueryView",
        optional: false,
        doc: None,
    },
    TsField {
        name: "hold",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "ours",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "npcs",
        ty: "SceneEntity[]",
        optional: false,
        doc: Some("Packet-time NPC_INFO rows. Copy if retaining past this tick. size<1 is unavailable, not a synthetic 1."),
    },
    TsField {
        name: "locs",
        ty: "SceneEntity[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "players",
        ty: "SceneEntity[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "ground",
        ty: "SceneEntity[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "equipment",
        ty: "ItemRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_open",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_continue",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_text",
        ty: "string | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_options",
        ty: "ChatOption[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "side_tab",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "varps",
        ty: "VarpRow[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "combat_styles",
        ty: "CombatStyleButton[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "run_energy",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "run_enabled",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "retaliate_enabled",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "retaliate_controls",
        ty: "ToggleControls | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "quest_statuses",
        ty: "QuestStatusRow[] | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "npc_boxes",
        ty: "{ index: number; points: { x: number; y: number }[] }[] | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "my_name",
        ty: "string | null",
        optional: false,
        doc: None,
    },
    TsField {
        name: "in_combat",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "animating",
        ty: "boolean",
        optional: false,
        doc: None,
    },
    TsField {
        name: "main_modal_id",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "main_modal_texts",
        ty: "{ root: number; texts: string[] }",
        optional: true,
        doc: Some(
            "The main modal's paired TYPE_TEXT walk: `root` is the same integer `main_modal_id` carries, `texts` is its walk order with colour tags intact. `{ root: -1, texts: [] }` is an observed closed modal. Omitted: the post did not carry the pair (keep the last one).",
        ),
    },
    TsField {
        name: "chat_modal_id",
        ty: "number",
        optional: false,
        doc: None,
    },
    TsField {
        name: "make_products",
        ty: "MakeProduct[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "side_tab_ifaces",
        ty: "SideTabIface[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "spell_buttons",
        ty: "CombatStyleButton[]",
        optional: false,
        doc: None,
    },
    TsField {
        name: "chat_lines",
        ty: "ChatLine[]",
        optional: false,
        doc: None,
    },
];

const INTERACT_VARIANTS: &[InteractVariant] = &[
    InteractVariant {
        op: "open-booth",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "open-stand",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "kind",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "stand_op",
                ty: "number | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "choose",
                ty: "string | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "walk",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "allow_teleports",
                ty: "boolean",
                optional: true,
                doc: None,
            },
            TsField {
                name: "allow_wilderness",
                ty: "boolean",
                optional: true,
                doc: None,
            },
            TsField {
                name: "allow_bank_fetch",
                ty: "boolean",
                optional: true,
                doc: None,
            },
            TsField {
                name: "request_id",
                ty: "number",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "walk-near",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "radius",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "allow_teleports",
                ty: "boolean",
                optional: true,
                doc: None,
            },
            TsField {
                name: "allow_wilderness",
                ty: "boolean",
                optional: true,
                doc: None,
            },
            TsField {
                name: "allow_bank_fetch",
                ty: "boolean",
                optional: true,
                doc: None,
            },
            TsField {
                name: "request_id",
                ty: "number",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "walk-nearest-bank",
        fields: &[],
    },
    InteractVariant {
        op: "inspect-route",
        fields: &[
            TsField { name: "x", ty: "number", optional: false, doc: None },
            TsField { name: "z", ty: "number", optional: false, doc: None },
            TsField { name: "level", ty: "number", optional: false, doc: None },
            TsField { name: "from_x", ty: "number", optional: false, doc: None },
            TsField { name: "from_z", ty: "number", optional: false, doc: None },
            TsField { name: "from_level", ty: "number", optional: false, doc: None },
            TsField { name: "allow_teleports", ty: "boolean", optional: true, doc: None },
            TsField { name: "allow_wilderness", ty: "boolean", optional: true, doc: None },
            TsField { name: "allow_bank_fetch", ty: "boolean", optional: true, doc: None },
            TsField { name: "avoid", ty: "Array<{ minX: number; maxX: number; minZ: number; maxZ: number; level?: number }>", optional: true, doc: None },
            TsField { name: "request_id", ty: "number", optional: true, doc: None },
        ],
    },
    InteractVariant {
        op: "walk-to",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "deposit",
        fields: &[TsField {
            name: "name",
            ty: "string",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "withdraw",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "withdraw-x",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "count",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "bank_item_id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "lands_as_id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "bank_generation",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "withdraw-load",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "bank_generation",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "held",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "inv-button",
        fields: &[
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "slot",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "operation",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "bank_generation",
                ty: "number",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "puzzle-move",
        fields: &[
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "slot",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "generation",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "shop-button",
        fields: &[
            TsField {
                name: "kind",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "slot",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "chunk",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "make-panel",
        fields: &[
            TsField {
                name: "id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "slot",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "component",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "operation",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "close",
        fields: &[],
    },
    InteractVariant {
        op: "npc",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "index",
                ty: "number | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "loc",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "id",
                ty: "number | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "obj",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "player",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "action",
                ty: "string",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "use-on",
        fields: &[
            TsField {
                name: "name",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "kind",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "target_name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "index",
                ty: "number | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "source_item_id",
                ty: "number | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "source_item_slot",
                ty: "number | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "target_item_id",
                ty: "number | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "target_item_slot",
                ty: "number | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "use-widget-on",
        fields: &[
            TsField {
                name: "component_id",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "kind",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "target_name",
                ty: "string | null",
                optional: true,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "index",
                ty: "number | null",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "continue",
        fields: &[],
    },
    InteractVariant {
        op: "answer",
        fields: &[TsField {
            name: "option",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "answer-count",
        fields: &[TsField {
            name: "value",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "if-button",
        fields: &[TsField {
            name: "component_id",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "close-modal",
        fields: &[],
    },
    InteractVariant {
        op: "side-tab",
        fields: &[TsField {
            name: "tab",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "wear",
        fields: &[TsField {
            name: "name",
            ty: "string",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "unequip",
        fields: &[TsField {
            name: "name",
            ty: "string",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "set-run",
        fields: &[TsField {
            name: "on",
            ty: "boolean",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "set-retaliate",
        fields: &[TsField {
            name: "on",
            ty: "boolean",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "set-note-mode",
        fields: &[TsField {
            name: "on",
            ty: "boolean",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "set-camera-yaw",
        fields: &[TsField {
            name: "yaw",
            ty: "number",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "note-progress",
        fields: &[],
    },
    InteractVariant {
        op: "loop-settled",
        fields: &[],
    },
    InteractVariant {
        op: "wait-enqueued",
        fields: &[],
    },
    InteractVariant {
        op: "wait-settled",
        fields: &[],
    },
    InteractVariant {
        op: "recovery-anchor",
        fields: &[
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "z",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "level",
                ty: "number",
                optional: false,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "recovery-anchor-none",
        fields: &[],
    },
    InteractVariant {
        op: "key",
        fields: &[
            TsField {
                name: "down",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "key",
                ty: "string",
                optional: false,
                doc: None,
            },
            TsField {
                name: "code",
                ty: "string",
                optional: true,
                doc: None,
            },
        ],
    },
    InteractVariant {
        op: "mouse",
        fields: &[
            TsField {
                name: "down",
                ty: "boolean",
                optional: false,
                doc: None,
            },
            TsField {
                name: "x",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "y",
                ty: "number",
                optional: false,
                doc: None,
            },
            TsField {
                name: "button",
                ty: "number",
                optional: true,
                doc: None,
            },
        ],
    },
];

const NATIVE_SNAPSHOT_FIELDS: &[TsField] = &[
    TsField { name: "ingame", ty: "boolean", optional: false, doc: None },
    TsField { name: "here", ty: "WorldTile | null", optional: false, doc: None },
    TsField { name: "inv", ty: "ItemRow[]", optional: false, doc: None },
    TsField {
        name: "inv_size",
        ty: "number",
        optional: false,
        doc: Some("0 while the inv tab is tutorial-locked."),
    },
    TsField { name: "stats", ty: "StatRow[]", optional: false, doc: None },
    TsField { name: "bank", ty: "ItemRow[]", optional: false, doc: None },
    TsField { name: "bank_side", ty: "ItemRow[]", optional: false, doc: None },
    TsField { name: "bank_open", ty: "boolean", optional: false, doc: None },
    TsField { name: "bank_loaded", ty: "boolean", optional: false, doc: None },
    TsField {
        name: "bank_generation",
        ty: "number",
        optional: false,
        doc: Some("Pass this into withdraw-load / withdraw-x. A changed generation is stale, not exhaustion."),
    },
    TsField {
        name: "bank_snapshot_generation",
        ty: "number",
        optional: false,
        doc: Some("Open bank item packet generation; -1 while closed. Not an action session identity."),
    },
    TsField { name: "banks", ty: "BankStand[]", optional: false, doc: None },
    TsField { name: "nearest_booth", ty: "NearestBooth | null", optional: false, doc: None },
    TsField { name: "bank_approaches", ty: "BankApproach[]", optional: false, doc: None },
    TsField { name: "count_dialog_open", ty: "boolean", optional: false, doc: None },
    TsField {
        name: "puzzle_board",
        ty: "{ component_id: number; size: number; items: ItemRow[] }",
        optional: false,
        doc: Some("The posted piece container. component_id -1 is a closed board, not a missing one. Rows are that widget's own sparse ItemRows."),
    },
    TsField {
        name: "puzzle_board_generation",
        ty: "number",
        optional: false,
        doc: Some("Pass this into puzzle-move. A changed generation is stale, not a solved board."),
    },
    TsField { name: "withdraw_x_result_seq", ty: "number", optional: false, doc: None },
    TsField { name: "withdraw_x_result", ty: "boolean", optional: false, doc: None },
    TsField { name: "withdraw_load_result_seq", ty: "number", optional: false, doc: None },
    TsField { name: "withdraw_load_result", ty: "boolean", optional: false, doc: None },
    TsField { name: "bank_op_result_seq", ty: "number", optional: false, doc: None },
    TsField { name: "bank_op_result", ty: "boolean", optional: false, doc: None },
    TsField { name: "walk_outcome_seq", ty: "number", optional: false, doc: None },
    TsField { name: "walk_outcome_generation", ty: "number", optional: false, doc: None },
    TsField { name: "walk_outcome_failed", ty: "boolean", optional: false, doc: None },
    TsField { name: "walk_outcome_x", ty: "number", optional: false, doc: None },
    TsField { name: "walk_outcome_z", ty: "number", optional: false, doc: None },
    TsField { name: "walk_outcome_level", ty: "number", optional: false, doc: None },
    TsField { name: "walk_outcome_radius", ty: "number", optional: false, doc: None },
    TsField { name: "walk_outcome_allow_teleports", ty: "boolean", optional: false, doc: None },
    TsField { name: "walk_outcome_request_id", ty: "number", optional: false, doc: None },
    TsField { name: "bank_selection", ty: "{ request_id: number; generation: number; kind: 'near' | 'reachable' | 'fallback' | 'none'; bank: BankLocation | null } | null", optional: false, doc: Some("Latest select-only completion; fallback is not a reachability proof.") },
    TsField { name: "route_inspect_seq", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_generation", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_request_id", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_ok", ty: "boolean", optional: false, doc: None },
    TsField { name: "route_inspect_reason", ty: "string", optional: false, doc: None },
    TsField { name: "route_inspect_bank_planned", ty: "boolean", optional: false, doc: None },
    TsField { name: "route_inspect_ticks", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_hops", ty: "Array<{ kind: string; locId: number; locName: string; action: string; option: number; from: WorldTile; to: WorldTile; ticks: number }>", optional: false, doc: None },
    TsField { name: "route_inspect_prev_seq", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_prev_generation", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_prev_request_id", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_prev_ok", ty: "boolean", optional: false, doc: None },
    TsField { name: "route_inspect_prev_reason", ty: "string", optional: false, doc: None },
    TsField { name: "route_inspect_prev_bank_planned", ty: "boolean", optional: false, doc: None },
    TsField { name: "route_inspect_prev_ticks", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_prev_hops", ty: "Array<{ kind: string; locId: number; locName: string; action: string; option: number; from: WorldTile; to: WorldTile; ticks: number }>", optional: false, doc: None },
    TsField { name: "route_inspect_running_id", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_pending_id", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_accepted_id", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_replaced_id", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_replaced_prev_id", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_refused_id", ty: "number", optional: false, doc: Some("Latest registered token the host could not reserve. Not a ring terminal.") },
    TsField { name: "route_inspect_refused_id_2", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_refused_id_3", ty: "number", optional: false, doc: None },
    TsField { name: "route_inspect_unobserved", ty: "number", optional: false, doc: Some("Host unobserved obligation count. Advisory; may lag the next drain.") },
    TsField {
        name: "collision",
        ty: "CollisionView",
        optional: false,
        doc: Some("One current-plane raw i32 grid. flags.at is indexed lx*height+lz. 0 is clear, not absent."),
    },
    TsField {
        name: "npcs",
        ty: "SceneEntity[]",
        optional: false,
        doc: Some("Packet-time NPC_INFO rows. Copy if retaining past this tick. size<1 is unavailable, not a synthetic 1."),
    },
    TsField {
        name: "self_target_kind",
        ty: "number",
        optional: false,
        doc: Some("0 none, 1 npc, 2 player. Packet-time local face."),
    },
    TsField {
        name: "self_target_index",
        ty: "number",
        optional: false,
        doc: Some("-1 when kind is 0. 0 is a legal NPC index."),
    },
    TsField {
        name: "gather",
        ty: "GatherSession | null",
        optional: false,
        doc: Some("Live API gather session; null when none."),
    },
];

const NATIVE_OP_VARIANTS: &[InteractVariant] = &[
    InteractVariant {
        op: "held",
        fields: &[
            TsField { name: "name", ty: "string", optional: false, doc: None },
            TsField { name: "action", ty: "string", optional: false, doc: None },
        ],
    },
    InteractVariant {
        op: "open-booth",
        fields: &[
            TsField { name: "x", ty: "number", optional: false, doc: None },
            TsField { name: "z", ty: "number", optional: false, doc: None },
            TsField { name: "level", ty: "number", optional: false, doc: None },
            TsField { name: "id", ty: "number", optional: false, doc: None },
            TsField { name: "name", ty: "string", optional: true, doc: None },
            TsField { name: "action", ty: "string", optional: true, doc: None },
        ],
    },
    InteractVariant {
        op: "open-stand",
        fields: &[
            TsField { name: "x", ty: "number", optional: false, doc: None },
            TsField { name: "z", ty: "number", optional: false, doc: None },
            TsField { name: "level", ty: "number", optional: false, doc: None },
            TsField { name: "kind", ty: "string", optional: false, doc: None },
            TsField { name: "name", ty: "string", optional: true, doc: None },
            TsField { name: "stand_op", ty: "number", optional: true, doc: None },
            TsField { name: "choose", ty: "string", optional: true, doc: None },
        ],
    },
    InteractVariant { op: "close", fields: &[] },
    InteractVariant {
        op: "set-note-mode",
        fields: &[TsField { name: "on", ty: "boolean", optional: false, doc: None }],
    },
    InteractVariant {
        op: "deposit",
        fields: &[TsField {
            name: "name",
            ty: "string",
            optional: false,
            doc: None,
        }],
    },
    InteractVariant {
        op: "withdraw",
        fields: &[
            TsField { name: "name", ty: "string", optional: false, doc: None },
            TsField { name: "action", ty: "string", optional: false, doc: None },
        ],
    },
    InteractVariant {
        op: "withdraw-load",
        fields: &[
            TsField { name: "name", ty: "string", optional: false, doc: None },
            TsField { name: "bank_generation", ty: "number", optional: false, doc: None },
        ],
    },
    InteractVariant {
        op: "withdraw-x",
        fields: &[
            TsField { name: "name", ty: "string", optional: false, doc: None },
            TsField { name: "count", ty: "number", optional: false, doc: None },
            TsField { name: "bank_item_id", ty: "number", optional: false, doc: None },
            TsField { name: "lands_as_id", ty: "number", optional: false, doc: None },
            TsField { name: "action", ty: "string", optional: false, doc: None },
            TsField { name: "bank_generation", ty: "number", optional: false, doc: None },
        ],
    },
    InteractVariant {
        op: "puzzle-move",
        fields: &[
            TsField { name: "id", ty: "number", optional: false, doc: None },
            TsField { name: "slot", ty: "number", optional: false, doc: None },
            TsField { name: "component", ty: "number", optional: false, doc: None },
            TsField { name: "generation", ty: "number", optional: false, doc: None },
        ],
    },
    InteractVariant {
        op: "walk",
        fields: &[
            TsField { name: "x", ty: "number", optional: false, doc: None },
            TsField { name: "z", ty: "number", optional: false, doc: None },
            TsField { name: "level", ty: "number", optional: false, doc: None },
            TsField { name: "allow_teleports", ty: "boolean", optional: true, doc: None },
            TsField { name: "allow_wilderness", ty: "boolean", optional: true, doc: None },
            TsField { name: "allow_bank_fetch", ty: "boolean", optional: true, doc: None },
            TsField { name: "request_id", ty: "number", optional: true, doc: None },
        ],
    },
    InteractVariant {
        op: "walk-near",
        fields: &[
            TsField { name: "x", ty: "number", optional: false, doc: None },
            TsField { name: "z", ty: "number", optional: false, doc: None },
            TsField { name: "level", ty: "number", optional: false, doc: None },
            TsField { name: "radius", ty: "number", optional: false, doc: None },
            TsField { name: "allow_teleports", ty: "boolean", optional: true, doc: None },
            TsField { name: "allow_wilderness", ty: "boolean", optional: true, doc: None },
            TsField { name: "allow_bank_fetch", ty: "boolean", optional: true, doc: None },
            TsField { name: "request_id", ty: "number", optional: true, doc: None },
        ],
    },
    InteractVariant { op: "walk-nearest-bank", fields: &[] },
    InteractVariant {
        op: "inspect-route",
        fields: &[
            TsField { name: "from", ty: "WorldTile", optional: false, doc: None },
            TsField { name: "to", ty: "WorldTile", optional: false, doc: None },
            TsField { name: "allow_teleports", ty: "boolean", optional: true, doc: None },
            TsField { name: "allow_wilderness", ty: "boolean", optional: true, doc: None },
            TsField { name: "allow_bank_fetch", ty: "boolean", optional: true, doc: None },
            TsField { name: "avoid", ty: "Array<{ minX: number; maxX: number; minZ: number; maxZ: number; level?: number }>", optional: true, doc: None },
            TsField { name: "request_id", ty: "number", optional: true, doc: Some("Isolate inspectBegin token only. 0 is snapshot-only and still runs a host preview. Invented nonzero ids never reach the host.") },
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_find_options_wilderness_jsdoc() {
        let src = render_host_js_dts();
        assert!(src.contains("allow_wilderness"));
        assert!(src.contains("wilderness zone"));
    }

    #[test]
    fn render_excludes_game_teleport() {
        let src = render_host_js_dts();
        assert!(!src.contains("Game.teleport"));
        assert!(!src.contains("teleport("));
    }
}
