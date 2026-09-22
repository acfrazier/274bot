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
    assert!(src.contains("exact_rank: number[]"));
    assert!(src.contains("adjacent_rank: number[]"));
    assert!(src.contains("reach: ReachQueryView"));
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
    assert!(!src.contains("gatherMethods(input?: { skill?: string }): Promise"));
    assert!(src.contains(
        "questIdentity(input: { name: string } | { id: string }): HelperResult<QuestIdentityRow>"
    ));
    assert!(src.contains(
        "questPrereqs(input: { id: string; name?: string }): HelperResult<QuestRequirements>"
    ));
    assert!(!src.contains("questIdentity(input: { name: string } | { id: string }): Promise"));
    assert!(!src.contains("questPrereqs(input: { id: string; name?: string }): Promise"));
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
    const START: &str = "export interface NativeSnapshot {";
    let start = src
        .find(START)
        .unwrap_or_else(|| panic!("missing NativeSnapshot"));
    let rest = &src[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("unclosed NativeSnapshot"));
    &rest[..=end]
}

/// Writes `host-js/index.d.ts` from the host verb tables.
#[test]
#[ignore]
fn regen_host_js() {
    write_host_js_dts().expect("write host-js/index.d.ts");
    eprintln!("wrote {}", host_js_path().display());
}
