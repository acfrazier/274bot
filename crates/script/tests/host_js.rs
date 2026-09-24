// Host JS living index.d.ts — our verbs, not rs2b0t names.
use script::host_js::{host_js_path, render_host_js_dts, write_host_js_dts};

#[test]
fn host_js_dts_is_fresh() {
    let path = host_js_path();
    let on_disk =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let rendered = render_host_js_dts();
    assert_eq!(
        on_disk, rendered,
        "host-js/index.d.ts is stale; run: cargo test -p script --test host_js regen_host_js -- --ignored"
    );
}

#[test]
fn host_js_dts_includes_required_interfaces() {
    let src = render_host_js_dts();
    assert!(src.contains("export interface FindOptions"));
    assert!(src.contains("allow_wilderness"));
    assert!(
        src.contains("wilderness"),
        "FindOptions JSDoc must mention wilderness exclusion"
    );
    assert!(src.contains("export interface Camera"));
    assert!(src.contains("orbit_yaw"));
    assert!(src.contains("export interface ShopStockRow"));
    assert!(src.contains("export interface ReachQueryView"));
    assert!(!src.contains("exact_rank: number[]"));
    assert!(!src.contains("adjacent_rank: number[]"));
    assert!(src.contains("reach: ReachQueryView"));
    assert!(src.contains(
        "walkable(input: { tile: WorldTile } | WorldTile): HelperResult<boolean>"
    ));
    assert!(src.contains(
        "canStep(input: { from: WorldTile; to: WorldTile }): HelperResult<boolean>"
    ));
    assert!(src.contains(
        "canReach(input: { tile: WorldTile; adjacentOk?: boolean; maxSteps?: number }): HelperResult<boolean>"
    ));
    assert!(src.contains("export interface CollisionFlagsView"));
    assert!(src.contains("export interface CollisionView"));
    assert!(src.contains("collision: CollisionView"));
    assert!(src.contains(
        "lineOfSight(input: { from: WorldTile; to: WorldTile; size?: number }): HelperResult<boolean>"
    ));
    assert!(src.contains("fightBegin(input?: object): HelperResult<{ token: number }>"));
    assert!(src.contains(
        "fightNext(input: { token: number; reply?: unknown } & Record<string, unknown>): FightStep"
    ));
    assert!(src.contains("holdBegin(input?: object): HelperResult<{ token: number }>"));
    assert!(src.contains(
        "holdNext(input: { token: number; reply?: unknown } & Record<string, unknown>): HoldStep"
    ));
    assert!(src.contains("export type FightStep"));
    assert!(src.contains("export type HoldStep"));
    assert!(src.contains("export type RetreatStep"));
    assert!(src.contains("walkspotBegin(input?: object): HelperResult<{ token: number }>"));
    assert!(src.contains(
        "walkspotNext(input: { token: number; reply?: unknown } & Record<string, unknown>): WalkStep"
    ));
    assert!(src.contains("export type WalkStep"));
    let walk_step = src.split("export type WalkStep").nth(1).expect("WalkStep");
    assert!(walk_step.contains("kind: 'yield'"));
    assert!(walk_step.contains("kind: 'aborted'"));
    assert!(
        !walk_step.contains("status: 'aborted'; token: number; kind: 'aborted'"),
        "WalkStep must not copy FightStep ok:true aborted"
    );
    assert!(src.contains("kind: 'yield'"));
    assert!(src.contains("export interface QuestStatusRow"));
    assert!(src.contains("quest_statuses: QuestStatusRow[] | null"));
    assert!(src.contains("slot?: number"));
    assert!(src.contains("source_item_id?: number | null"));
    assert!(src.contains("source_item_slot?: number | null"));
    assert!(src.contains("target_item_id?: number | null"));
    assert!(src.contains("target_item_slot?: number | null"));
    assert!(
        !src.contains("Game.teleport"),
        "must not export Game.teleport"
    );
    assert!(
        !src.contains("teleport("),
        "must not export teleport string-table helper"
    );
    assert!(src.contains("export interface NativeApi"));
    assert!(src.contains("export type HelperResult"));
    assert!(src.contains("export interface PrayerClearCounts"));
    assert!(src.contains("prayerPoints(): HelperResult<number>"));
    assert!(src.contains("foodCount(input: { items: ItemRow[]; foodName: string })"));
    assert!(src.contains("escapeRunesFor(input: { id: string })"));
    assert!(src.contains(
        "gatherMethods(input?: { skill?: string }): HelperResult<{ rows: GatherMethodRow[]; coverage: GatherCoverageRecord[] }>"
    ));
    assert!(src.contains(
        "gatherResource(input: { name: string }): HelperResult<{ rows: GatherLocResourceRow[] }>"
    ));
    assert!(src.contains(
        "gatherPlacements(input: { resource: string; region: SceneRegionInput; limit: number }): HelperResult<GatherPlacementResult>"
    ));
    assert!(!src.contains(
        "gatherPlacements(input: { resource: string; region: SceneRegionInput; limit: number }): Promise"
    ));
    assert!(src.contains("export interface GatherPlacementRow {"));
    assert!(src.contains("export interface GatherPlacementResult {"));
    let placement_result = src
        .split("export interface GatherPlacementResult {")
        .nth(1)
        .expect("GatherPlacementResult");
    let placement_result = &placement_result[..placement_result.find("\n}\n").expect("closed")];
    assert!(placement_result.contains("rows: GatherPlacementRow[];"));
    assert!(placement_result.contains("resource_ids: GatherId[];"));
    assert!(placement_result.contains("qualification: string;"));
    assert!(
        !placement_result.contains("SceneProjection"),
        "placements must not reuse the posted-scene row type: {placement_result}"
    );
    assert!(!src.contains("gatherMethods(input?: { skill?: string }): Promise"));
    assert!(src.contains(
        "questIdentity(input: { name: string } | { id: string }): HelperResult<QuestIdentityRow>"
    ));
    assert!(src.contains(
        "questPrereqs(input: { id: string; name?: string }): HelperResult<QuestRequirements>"
    ));
    assert!(!src.contains("questIdentity(input: { name: string } | { id: string }): Promise"));
    assert!(!src.contains("questPrereqs(input: { id: string; name?: string }): Promise"));
    assert!(src.contains("export interface ClueRow {"));
    assert!(src.contains("params: Array<{ key: string; value: string }>;"));
    assert!(src.contains("access?: 'constrained';"));
    assert!(src.contains(
        "clue: { row(input: { id: number } | { alias: string }): HelperResult<ClueRow>; heldStep(): HelperResult<ClueRow>; packPlan(input: PackPlanInput): HelperResult<PackPlanTargets>; hardKit(input: { attack: number; lostCity: boolean; items: { id: number; count: number }[] }): HelperResult<{ status: 'ready' }>; keep(input: { name: string; extra?: string[] }): HelperResult<{ keep: boolean }>; begin(input?: object): HelperResult<{ token: number }>; next(input: { token: number; resume?: boolean }): ClueStep; retry(): HelperResult<{ cleared: true }> }"
    ));
    assert!(src.contains("export type ClueStep"));
    let clue_step = src.split("export type ClueStep").nth(1).expect("ClueStep");
    assert!(clue_step.contains("kind: 'wait' | 'yield' | 'callback.enabled'"));
    assert!(clue_step.contains("kind: 'callback.log' | 'callback.setStatus'; message: string"));
    assert!(
        !clue_step.contains("status: 'done'"),
        "ClueStep must not promise a done status: {clue_step}"
    );
    assert!(src.contains("export interface PackPlanInput {"));
    assert!(src.contains("  rewardSlots?: number;"));
    assert!(!src.contains("clue: { row(input: { id: number } | { alias: string }): Promise"));
    assert!(!src.contains("heldStep(input"));
    assert!(!src.contains("heldStep(): Promise"));
    assert!(!src.contains("packPlan(input: PackPlanInput): Promise"));
    assert!(!src.contains("hardKit(input: { attack: number; lostCity: boolean; items: { id: number; count: number }[] }): Promise"));
    // Hard-kit is a status, not a kit read: no snapshot or inventory page.
    assert!(!src.contains("hardKit(input: { includeBank"));
    assert!(!src.contains("keep(input: { name: string; extra?: string[] }): Promise"));
    // Keep is a predicate, not a bank or inventory read.
    assert!(!src.contains("keep(input: { name: string; extra?: string[]; bank"));
    assert!(!src.contains("keep(input: { name: string; items"));
    assert!(!src.contains("clueRow("));
    assert!(src.contains(
        "sceneLocs(input: { ids: number[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>"
    ));
    assert!(src.contains(
        "sceneNpcs(input: { types: number[]; actions: string[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>"
    ));
    assert!(!src.contains(
        "sceneLocs(input: { ids: number[]; limit: number; region?: SceneRegionInput }): Promise"
    ));
    assert!(!src.contains(
        "sceneNpcs(input: { types: number[]; actions: string[]; limit: number; region?: SceneRegionInput }): Promise"
    ));
    assert!(src.contains("export interface SceneRegionInput"));
    assert!(src.contains("export interface SceneBounds"));
    assert!(src.contains("export interface SceneProjectionRow"));
    assert!(src.contains("export interface SceneProjection {"));
    assert!(src.contains("as_of_sequence: number;"));
    assert!(src.contains("truncated: boolean;"));
    assert!(src.contains(
        "questStatus(input: { name: string }): HelperResult<{ status: 'notStarted' | 'inProgress' | 'complete' | 'unknown'; as_of_sequence: number }>"
    ));
    assert!(!src.contains("questStatus(input: { name: string }): Promise"));
    assert!(!src.contains("questStatus(input: { name: string }): HelperResult<QuestStatusRow>"));
    assert!(src.contains("export interface LoadoutInput"));
    assert!(src.contains("export interface PotionPlan"));
    assert!(src.contains("foodOf(input: { loadout: LoadoutInput | null; fallback: string })"));
    assert!(src.contains("potionToSip(input: { plans: PotionPlan[]; held: number[]; levels: Array<{ skill: string; base: number; effective: number }> })"));
    assert!(src.contains(
        "prayerSet(input: { name: string; on: boolean }): Promise<HelperResult<boolean>>"
    ));
    assert!(src.contains("prayerClear(): Promise<HelperResult<PrayerClearCounts>>"));
    assert!(
        !src.contains("prayerSetBegin"),
        "named async Set/Clear is the private-lifecycle exception; no Begin/Next"
    );
    assert!(src.contains("export type NativeOp"));
    assert!(src.contains("op: 'walk-nearest-bank'"));
    assert!(src.contains("{ op: 'deposit'; name: string}"));
    assert!(src.contains("op: 'walk'"));
    assert!(src.contains("op: 'walk-near'"));
    assert!(
        !src.contains("export type NativeOp =\n  | { op: 'walk'"),
        "v2 NativeOp must not dump the full InteractReq union"
    );

    let native = native_snapshot_block(&src);
    assert!(
        native.contains("npcs: SceneEntity[]"),
        "typed NativeSnapshot.npcs required"
    );
    assert!(native.contains("self_target_kind: number"));
    assert!(native.contains("self_target_index: number"));
    assert!(
        native.contains("Packet-time NPC_INFO"),
        "npcs must document packet-time freshness"
    );
    assert!(
        native.contains("size<1 is unavailable"),
        "npcs must document unavailable size"
    );
    assert!(
        !native.contains("locs:") && !native.contains("players:") && !native.contains("ground:"),
        "locs/players/ground stay off NativeSnapshot: {native}"
    );
}

fn native_snapshot_block(src: &str) -> &str {
    interface_block(src, "NativeSnapshot")
}

/// One `export interface X { … }` block from the rendered declarations.
fn interface_block<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("export interface {name} {{"))
        .unwrap_or_else(|| panic!("missing {name}"));
    let rest = &src[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("unclosed {name}"));
    &rest[..=end]
}

/// One `export type X = …;` block from the rendered declarations.
fn type_block<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("export type {name} ="))
        .unwrap_or_else(|| panic!("missing {name}"));
    let rest = &src[start..];
    let end = rest
        .find(";\n")
        .unwrap_or_else(|| panic!("unclosed {name}"));
    &rest[..end]
}

/// `HostHandle.interact` is the queue `Bank`/`Banking` push onto: the bank
/// shim queues `walk-nearest-bank` to reach a stand and `withdraw-load` to
/// fill the pack (`bank.js`, `periodic_bank.js`). A union without them types
/// a queue narrower than the one the isolate drains. `inspect-ack` stays off
/// it: that ack is the isolate→host reply, never a script request.
#[test]
fn interact_union_publishes_the_banked_queue_ops() {
    let src = render_host_js_dts();
    let union = type_block(&src, "InteractReq");
    assert!(union.contains("| { op: 'walk-nearest-bank'}"), "{union}");
    assert!(
        union.contains("| { op: 'withdraw-load'; name: string; bank_generation: number}"),
        "{union}"
    );
    assert!(
        !union.contains("inspect-ack"),
        "the isolate consume-ack stays off the published queue: {union}"
    );
}

/// The declared MakeButton key is the posted one: `make_product_array` posts
/// `comId` on each button and the ClientAdapter reader maps `b.comId`. A
/// `com_id` declaration types a field no posted row carries.
#[test]
fn make_button_publishes_the_posted_component_key() {
    let src = render_host_js_dts();
    let button = interface_block(&src, "MakeButton");
    assert!(button.contains("comId: number;"), "{button}");
    assert!(!button.contains("com_id"), "{button}");
}

/// Writes `host-js/index.d.ts` from the host verb tables.
#[test]
#[ignore]
fn regen_host_js() {
    write_host_js_dts().expect("write host-js/index.d.ts");
    eprintln!("wrote {}", host_js_path().display());
}
