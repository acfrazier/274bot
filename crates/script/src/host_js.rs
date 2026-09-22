//! Generated Host JS types (`host-js/index.d.ts`) from our verb tables.
//! Snapshot fields, [`crate::shim::InteractReq`] ops, and nav
//! [`crate::FindOptions`] — not rs2b0t names. NativeTick Load is 0.2.5.

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
    out.push_str("/** prayerClear walk counts. timed_out may be nonzero; that is not all-off success. */\n");
    out.push_str("export interface PrayerClearCounts {\n");
    out.push_str("  clicked: number;\n");
    out.push_str("  timed_out: number;\n");
    out.push_str("}\n\n");
    out.push_str("/** Caller-supplied carry row. Omitted qty defaults to 1 on loadout helpers. */\n");
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
    out.push_str("/** Loc-resource method row. publication is null when the landed row has none. */\n");
    out.push_str("export interface GatherLocResourceRow {\n");
    out.push_str("  skill: string;\n");
    out.push_str("  table: string;\n");
    out.push_str("  resource_key: string;\n");
    out.push_str("  loc_ids: GatherId[];\n");
    out.push_str("  empty_ids: GatherId[];\n");
    out.push_str("  output: GatherId | null;\n");
    out.push_str("  level: number;\n");
    out.push_str("  qualification: string;\n");
    out.push_str("  partial_sides: string[];\n");
    out.push_str("  missing_transform: string[];\n");
    out.push_str("  publication: string | null;\n");
    out.push_str("}\n\n");
    out.push_str("/** Fishing method row. No loc ids and no spawn tile. */\n");
    out.push_str("export interface GatherFishingRow {\n");
    out.push_str("  skill: string;\n");
    out.push_str("  category: string;\n");
    out.push_str("  primary_op: string;\n");
    out.push_str("  pair_op: string | null;\n");
    out.push_str("  level: null;\n");
    out.push_str("  output: null;\n");
    out.push_str("  qualification: string;\n");
    out.push_str("  partial_sides: string[];\n");
    out.push_str("}\n\n");
    out.push_str("export type GatherMethodRow = GatherLocResourceRow | GatherFishingRow;\n\n");
    out.push_str("/** Coverage beside method rows. Not a resource hit. */\n");
    out.push_str("export interface GatherCoverageRecord {\n");
    out.push_str("  class: string;\n");
    out.push_str("  table?: string;\n");
    out.push_str("  resource_key?: string;\n");
    out.push_str("  alias?: string;\n");
    out.push_str("  on_revision?: number;\n");
    out.push_str("  other_pin_id?: number;\n");
    out.push_str("  copied?: boolean;\n");
    out.push_str("  reason: string;\n");
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
    out.push_str("/** Historical posted row copy. Not the live entity: no index, name, or nested tile. */\n");
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
    out.push_str("/** Public JS API v2 handle. Explicit `export const apiVersion = 2` only. */\n");
    out.push_str("export interface NativeApi {\n");
    out.push_str("  readonly tick: number;\n");
    out.push_str("  /** Host-owned, delta-merged. Read only. Do not mutate; copy if retaining. */\n");
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
    out.push_str("  prayerSet(input: { name: string; on: boolean }): Promise<HelperResult<boolean>>;\n");
    out.push_str("  /** Completes the 15-row walk. timed_out may be nonzero; LIVE later requires all off. Same busy refuse as prayerSet. */\n");
    out.push_str("  prayerClear(): Promise<HelperResult<PrayerClearCounts>>;\n");
    out.push_str("  foodCount(input: { items: ItemRow[]; foodName: string }): HelperResult<number>;\n");
    out.push_str("  foodHealAmount(input: { foodName: string }): HelperResult<number>;\n");
    out.push_str("  combatKeepNames(input: { food: string; style?: string; spell?: string; ammo?: string; weapon?: string; extra?: string[] }): HelperResult<string[]>;\n");
    out.push_str("  runesPerCast(input: { spellName: string; wielded: string[] }): HelperResult<Array<{ rune: string; count: number }> | null>;\n");
    out.push_str("  escapeRunesFor(input: { id: string }): HelperResult<{ runes: Array<{ rune: string; count: number }>; level: number; label: string }>;\n");
    out.push_str("  /** Sync fact read. Omitted input or `{}` omits the skill. Not a Promise and not a request op. */\n");
    out.push_str("  gatherMethods(input?: { skill?: string }): HelperResult<{ rows: GatherMethodRow[]; coverage: GatherCoverageRecord[] }>;\n");
    out.push_str("  /** Sync resource-key read. Zero matches is unknown-resource, not an empty rows list. */\n");
    out.push_str("  gatherResource(input: { name: string }): HelperResult<{ rows: GatherLocResourceRow[] }>;\n");
    out.push_str("  /** Sync fact read. Exactly one of name or id, and it must be a string. Not a Promise and not a request op. */\n");
    out.push_str("  questIdentity(input: { name: string } | { id: string }): HelperResult<QuestIdentityRow>;\n");
    out.push_str("  /** Sync seed-id requirements read. A name field is not a key. Not a Promise and not a request op. */\n");
    out.push_str("  questPrereqs(input: { id: string; name?: string }): HelperResult<QuestRequirements>;\n");
    out.push_str("  /** Sync landed trail-row read and pure pack arithmetic over caller numbers. Neither is a Promise and neither is a request op; `packPlan` reads no snapshot, inventory, or family. */\n");
    out.push_str("  clue: { row(input: { id: number } | { alias: string }): HelperResult<ClueRow>; heldStep(): HelperResult<ClueRow>; packPlan(input: PackPlanInput): HelperResult<PackPlanTargets> };\n");
    out.push_str("  /** Sync posted-loc copy. Historical copy, not live. Not a Promise and not a request op. */\n");
    out.push_str("  sceneLocs(input: { ids: number[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>;\n");
    out.push_str("  /** Sync posted-npc copy. actions is required: omitted is not match-any. Historical copy, not live. Not a Promise and not a request op. */\n");
    out.push_str("  sceneNpcs(input: { types: number[]; actions: string[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>;\n");
    out.push_str("  /** Sync posted-tab status copy. A missing page is snapshot-unavailable and a null tab is quest-tab-unbound. Not a Promise and not a request op. */\n");
    out.push_str("  questStatus(input: { name: string }): HelperResult<{ status: 'notStarted' | 'inProgress' | 'complete' | 'unknown'; as_of_sequence: number }>;\n");
    out.push_str("  /** Sync owned-root begin. One `if-button` per token on the posted row id, enqueued synchronously after a generation check. Not a Promise and not a request op; a refusal is `{ ok: false, error }`. */\n");
    out.push_str("  questJournalBegin(input: { name: string }): HelperResult<{ token: number }>;\n");
    out.push_str("  /** Sync owned-root next. Not-done is `{ pending: true }` (no `ok` field) — not empty lines. Not a Promise. */\n");
    out.push_str("  questJournalNext(input: { token: number }): HelperResult<{ lines: string[]; root: number; as_of_sequence: number }>;\n");
    out.push_str("  /** Sync owned-root close. One `close-modal` only while the latest pair is still the acquired root and texts. Not-done is `{ pending: true }` (no `ok` field). Not a Promise; never returns journal lines. */\n");
    out.push_str("  questJournalClose(input: { token: number }): HelperResult<{ closed: true; as_of_sequence: number }>;\n");
    out.push_str("  foodOf(input: { loadout: LoadoutInput | null; fallback: string }): HelperResult<string>;\n");
    out.push_str("  gearOf(input: { loadout: LoadoutInput | null }): HelperResult<string[]>;\n");
    out.push_str("  suppliesOf(input: { loadout: LoadoutInput | null }): HelperResult<Array<{ item: string; qty: number }>>;\n");
    out.push_str("  weaponOf(input: { loadout: LoadoutInput | null; fallback?: string | null }): HelperResult<string | null>;\n");
    out.push_str("  rangeLoadoutOf(input: { weapon: string; ammo: string }): HelperResult<RangeLoadout>;\n");
    out.push_str("  boostFaded(input: { base: number; effective: number; floor?: number }): HelperResult<boolean>;\n");
    out.push_str("  plannedPotions(input: { carry: Array<{ item: string; qty: number }> }): HelperResult<PotionPlan[]>;\n");
    out.push_str("  potionToSip(input: { plans: PotionPlan[]; held: number[]; levels: Array<{ skill: string; base: number; effective: number }> }): HelperResult<PotionPlan | null>;\n");
    out.push_str("  lineOfSight(input: { from: WorldTile; to: WorldTile; size?: number }): HelperResult<boolean>;\n");
    out.push_str("  fightBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  fightValidate(input: { token: number } & Record<string, unknown>): HelperResult<boolean>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield. Aborted is not anonymous done. */\n");
    out.push_str("  fightNext(input: { token: number; reply?: unknown } & Record<string, unknown>): FightStep;\n");
    out.push_str("  fightReset(input: { token: number }): HelperResult<null>;\n");
    out.push_str("  fightInterruptWatch(input: { token: number }): HelperResult<null>;\n");
    out.push_str("  fightBlocksLoot(input: { token: number } & Record<string, unknown>): HelperResult<boolean>;\n");
    out.push_str("  holdBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  holdValidate(input: { token: number } & Record<string, unknown>): HelperResult<boolean>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield. Aborted is not anonymous done. */\n");
    out.push_str("  holdNext(input: { token: number; reply?: unknown } & Record<string, unknown>): HoldStep;\n");
    out.push_str("  retreatBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  retreatValidate(input: { token: number } & Record<string, unknown>): HelperResult<boolean>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield. Aborted is not anonymous done. */\n");
    out.push_str("  retreatNext(input: { token: number; reply?: unknown } & Record<string, unknown>): RetreatStep;\n");
    out.push_str("  walkspotBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  walkspotValidate(input: { token: number } & Record<string, unknown>): HelperResult<boolean>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield. Aborted is not anonymous done. */\n");
    out.push_str("  walkspotNext(input: { token: number; reply?: unknown } & Record<string, unknown>): WalkStep;\n");
    out.push_str("  enterBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  enterValidate(input: { token: number } & Record<string, unknown>): HelperResult<boolean>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield and a boolean value. Aborted is ok false, not FightStep. */\n");
    out.push_str("  enterNext(input: { token: number; reply?: unknown } & Record<string, unknown>): EnterStep;\n");
    out.push_str("  leaveBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield and a boolean value. Aborted is ok false, not FightStep. */\n");
    out.push_str("  leaveNext(input: { token: number; reply?: unknown } & Record<string, unknown>): LeaveStep;\n");
    out.push_str("  keyBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield and a boolean value. Aborted is ok false, not FightStep. */\n");
    out.push_str("  keyNext(input: { token: number; reply?: unknown } & Record<string, unknown>): KeyStep;\n");
    out.push_str("  cellBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield and a boolean value. Aborted is ok false, not FightStep. */\n");
    out.push_str("  cellNext(input: { token: number; reply?: unknown } & Record<string, unknown>): CellStep;\n");
    out.push_str("  bankBegin(input?: object): HelperResult<{ token: number }>;\n");
    out.push_str("  /** One effect per call. Yield is status done with kind yield and a boolean value. Aborted is ok false, not FightStep. */\n");
    out.push_str("  bankNext(input: { token: number; reply?: unknown } & Record<string, unknown>): BankStep;\n");
    out.push_str("}\n\n");
    out.push_str("export type FightStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield' }\n");
    out.push_str("  | { ok: true; status: 'aborted'; token: number; kind: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
    out.push_str("export type HoldStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield' }\n");
    out.push_str("  | { ok: false; error: string; kind: 'aborted'; token: number; status: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
    out.push_str("export type RetreatStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield' }\n");
    out.push_str("  | { ok: false; error: string; kind: 'aborted'; token: number; status: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
    out.push_str("export type WalkStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield' }\n");
    out.push_str("  | { ok: false; error: string; kind: 'aborted'; token: number; status: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
    out.push_str("export type EnterStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield'; value: boolean }\n");
    out.push_str("  | { ok: false; error: string; kind: 'aborted'; token: number; status: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
    out.push_str("export type LeaveStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield'; value: boolean }\n");
    out.push_str("  | { ok: false; error: string; kind: 'aborted'; token: number; status: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
    out.push_str("export type KeyStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield'; value: boolean }\n");
    out.push_str("  | { ok: false; error: string; kind: 'aborted'; token: number; status: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
    out.push_str("export type CellStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield'; value: boolean }\n");
    out.push_str("  | { ok: false; error: string; kind: 'aborted'; token: number; status: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
    out.push_str("export type BankStep =\n");
    out.push_str("  | { ok: true; status: 'continue'; token: number; kind: string }\n");
    out.push_str("  | { ok: true; status: 'done'; token: number; kind: 'yield'; value: boolean }\n");
    out.push_str("  | { ok: false; error: string; kind: 'aborted'; token: number; status: 'aborted' }\n");
    out.push_str("  | { ok: false; error: string };\n");
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
        fields: &[TsField {
            name: "text",
            ty: "string",
            optional: false,
            doc: None,
        }],
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
        doc: Some("Compact native coordinate reachability with bounded dequeue metadata."),
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
            TsField {
                name: "walkable",
                ty: "number[]",
                optional: false,
                doc: None,
            },
            TsField {
                name: "reachable",
                ty: "number[]",
                optional: false,
                doc: None,
            },
            TsField {
                name: "reachable_adj",
                ty: "number[]",
                optional: false,
                doc: None,
            },
            TsField {
                name: "exact_rank",
                ty: "number[]",
                optional: false,
                doc: Some("Earliest exact dequeue rank; 65535 means unreachable."),
            },
            TsField {
                name: "adjacent_rank",
                ty: "number[]",
                optional: false,
                doc: Some(
                    "Earliest exact-or-valid-adjacent dequeue rank; 65535 means unreachable.",
                ),
            },
            TsField {
                name: "step",
                ty: "number[]",
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
                name: "com_id",
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
        doc: None,
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
    TsField { name: "banks", ty: "BankStand[]", optional: false, doc: None },
    TsField { name: "nearest_booth", ty: "NearestBooth | null", optional: false, doc: None },
    TsField { name: "bank_approaches", ty: "BankApproach[]", optional: false, doc: None },
    TsField { name: "count_dialog_open", ty: "boolean", optional: false, doc: None },
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
